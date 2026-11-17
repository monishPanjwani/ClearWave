//! App-owned, reversible XDG login integration. No shell is used by Exec.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
const MARKER: &str = "X-ClearWave-Managed=true\n";
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub enabled: bool,
    pub message: String,
}
pub fn path() -> Result<PathBuf> {
    Ok(crate::settings::config_path()?
        .parent()
        .context("Config directory missing")?
        .parent()
        .context("Config root missing")?
        .join("autostart/clearwave.desktop"))
}
pub fn executable() -> Result<PathBuf> {
    let path = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    validate_executable(&path)?;
    Ok(path)
}
fn validate_executable(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "Startup executable must have an absolute path"
    );
    ensure!(
        path.is_file() && fs::metadata(path)?.permissions().mode() & 0o111 != 0,
        "Startup executable is missing or not executable: {}. Enable startup again to repair it.",
        path.display()
    );
    ensure!(
        !path.to_string_lossy().contains(['\n', '\r', '\0']),
        "Invalid executable path"
    );
    Ok(())
}
pub fn entry(exe: &Path) -> Result<String> {
    validate_executable(exe)?;
    format_entry(exe)
}

fn owned(text: &str) -> bool {
    text.lines().any(|l| l == MARKER.trim_end())
}
fn target(text: &str) -> Option<PathBuf> {
    text.lines()
        .find_map(|l| l.strip_prefix("X-ClearWave-Executable="))
        .map(|s| PathBuf::from(s.replace("\\\\", "\\")))
}
fn exists_including_symlink(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn inspect_at(path: &Path) -> State {
    let result = (|| -> Result<State> {
        if !exists_including_symlink(path)? {
            return Ok(State {
                enabled: false,
                message: "Login startup is off".into(),
            });
        }
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Startup entry is a symlink; preserve it and resolve manually"
        );
        let text = fs::read_to_string(path)?;
        ensure!(
            owned(&text),
            "Startup entry is user-managed; resolve the conflict manually"
        );
        let exe = target(&text).context("Startup target is missing; enable again to repair")?;
        validate_executable(&exe)?;
        ensure!(
            entry(&exe)? == text,
            "Startup entry was edited; preserving it. Resolve manually"
        );
        Ok(State {
            enabled: true,
            message: format!("Starts at graphical login: {}", exe.display()),
        })
    })();
    result.unwrap_or_else(|e| State {
        enabled: false,
        message: format!("{e:#}"),
    })
}
pub fn inspect() -> State {
    path().map(|p| inspect_at(&p)).unwrap_or_else(|e| State {
        enabled: false,
        message: format!("{e:#}"),
    })
}
pub fn set_at(path: &Path, enabled: bool, exe: &Path) -> Result<()> {
    if exists_including_symlink(path)? {
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Refusing to replace a symlink startup entry"
        );
        let text = fs::read_to_string(path)?;
        ensure!(owned(&text), "Startup entry is user-managed; preserving it");
        // Compare against its original target even if that executable has moved.
        let old = target(&text).context("Malformed owned startup entry; preserving it")?;
        let expected = entry_unchecked(&old)?;
        ensure!(text == expected, "Startup entry was edited; preserving it");
    }
    if !enabled {
        if exists_including_symlink(path)? {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    let contents = entry(exe)?;
    let parent = path.parent().context("Invalid autostart path")?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".clearwave-{}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        created = true;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&tmp);
    }
    result
}
fn entry_unchecked(exe: &Path) -> Result<String> {
    format_entry(exe)
}
fn format_entry(exe: &Path) -> Result<String> {
    let text = exe.to_str().context("Startup path must be UTF-8")?;
    let mut q = String::new();
    for c in text.chars() {
        match c {
            '%' => q.push_str("%%"),
            '\\' => q.push_str("\\\\\\\\"),
            '"' | '$' | '`' => {
                q.push_str("\\\\");
                q.push(c);
            }
            _ => q.push(c),
        }
    }
    // GIO does not expand %% in argv[0]. Keep percent-containing executable
    // paths in an argument instead. env executes the absolute path without a shell.
    let launcher = if text.contains('%') {
        "/usr/bin/env "
    } else {
        ""
    };
    Ok(format!("[Desktop Entry]\nType=Application\nName=ClearWave\nComment=Start ClearWave with your explicitly saved login policy\nExec={launcher}\"{q}\" startup\nTerminal=false\n{MARKER}X-ClearWave-Executable={}\n",text.replace('\\',"\\\\")))
}
pub fn set(enabled: bool) -> Result<()> {
    if enabled {
        let mut child = std::process::Command::new("systemctl")
            .args(["--user", "is-enabled", "clearwave.service"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(child) = child.as_mut() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if let Some(status) = child.try_wait()? {
                    ensure!(!status.success(),"clearwave.service is enabled. Disable that legacy startup owner explicitly before enabling XDG login startup");
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("Cannot verify legacy clearwave.service startup ownership (systemctl timeout)");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    let exe = if enabled {
        executable()?
    } else {
        PathBuf::new()
    };
    set_at(&path()?, enabled, &exe)
}
/// Retarget an existing unmodified app-owned entry after a desktop installation.
pub fn repair() -> Result<()> {
    let path = path()?;
    if exists_including_symlink(&path)? {
        set_at(&path, true, &executable()?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integration_is_reversible_and_preserves_conflicts() {
        let dir = std::env::temp_dir().join(format!("cw-startup-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("clearwave.desktop");
        let exe = std::env::current_exe().unwrap();
        set_at(&p, true, &exe).unwrap();
        assert!(inspect_at(&p).enabled);
        set_at(&p, true, &exe).unwrap();
        fs::write(&p, "unrelated").unwrap();
        assert!(set_at(&p, false, &exe).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "unrelated");
        fs::remove_file(&p).unwrap();
        set_at(&p, true, &exe).unwrap();
        set_at(&p, false, &exe).unwrap();
        set_at(&p, false, &exe).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn exec_escaping() {
        let text = format_entry(Path::new("/a b/\"%$`\\tool")).unwrap();
        assert!(text.contains("Exec=/usr/bin/env \"/a b/\\\\\"%%\\\\$\\\\`\\\\\\\\tool\" startup"));
    }
    #[test]
    fn preserves_dangling_symlinks_and_existing_temporary_files() {
        let dir = std::env::temp_dir().join(format!("cw-startup-conflict-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("clearwave.desktop");
        let exe = std::env::current_exe().unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), &p).unwrap();
        assert!(inspect_at(&p).message.contains("symlink"));
        assert!(set_at(&p, true, &exe).is_err());
        assert!(set_at(&p, false, &exe).is_err());
        assert!(fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
        fs::remove_file(&p).unwrap();
        let tmp = dir.join(format!(".clearwave-{}.tmp", std::process::id()));
        fs::write(&tmp, "existing file").unwrap();
        assert!(set_at(&p, true, &exe).is_err());
        assert_eq!(fs::read_to_string(&tmp).unwrap(), "existing file");
        fs::remove_dir_all(dir).unwrap();
    }
}
