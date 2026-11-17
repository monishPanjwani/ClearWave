#![cfg(target_os = "linux")]
//! Isolated mock audio server: generated samples only; never reaches user audio.
use clearwave::{
    engine::{request_at, Request, Response},
    settings::Config,
};
use std::{
    fs,
    os::unix::{
        fs::{symlink, PermissionsExt},
        net::UnixStream,
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture {
    dir: PathBuf,
    child: Child,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "clearwave-lifecycle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(dir.join("bin")).unwrap();
        fs::create_dir(dir.join("config")).unwrap();
        fs::write(
            dir.join("bin/audio"),
            include_str!("fixtures/audio-server.py"),
        )
        .unwrap();
        fs::set_permissions(dir.join("bin/audio"), fs::Permissions::from_mode(0o755)).unwrap();
        for name in ["pactl", "parec", "pacat"] {
            symlink("audio", dir.join("bin").join(name)).unwrap();
        }
        let child = Command::new(env!("CARGO_BIN_EXE_clearwave"))
            .arg("daemon")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent-clearwave-test-bus",
            )
            .env("XDG_RUNTIME_DIR", &dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("CLEARWAVE_MOCK_DIR", &dir)
            .env("PATH", dir.join("bin"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let f = Self { dir, child };
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(f.socket()).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        f
    }
    fn socket(&self) -> PathBuf {
        self.dir.join("clearwave/engine.sock")
    }
    fn request(&self, r: Request) -> Response {
        request_at(&self.socket(), &r, Duration::from_secs(15)).unwrap()
    }
    fn state(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.dir.join("state.json")).unwrap()).unwrap()
    }
    fn wait_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "daemon shutdown timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}
#[test]
fn independent_sessions_live_controls_default_ownership_and_rollback() {
    let mut f = Fixture::new();
    let initial = f.request(Request::Status).status.unwrap();
    assert!(!initial.input.running && !initial.output.running);
    // A second daemon must fail without unlinking the first daemon's socket.
    let second = Command::new(env!("CARGO_BIN_EXE_clearwave"))
        .arg("daemon")
        .env("XDG_RUNTIME_DIR", &f.dir)
        .output()
        .unwrap();
    assert!(!second.status.success());
    f.request(Request::Status);
    let mut config = Config::default();
    config.input.device = "mic".into();
    config.output.device = "speakers".into();
    config.input.processing.denoise = false;
    f.request(Request::Configure {
        config: Box::new(config.clone()),
    });
    f.request(Request::Start { input: false });
    f.request(Request::Start { input: true });
    let s = f.request(Request::Status).status.unwrap();
    assert!(s.input.running && s.output.running);
    let output_route = s.output_route.clone();
    let input_route = s.input_route.clone();
    config.theme = Some("catppuccin-latte".into());
    f.request(Request::Configure {
        config: Box::new(config.clone()),
    });
    let themed = f.request(Request::Status).status.unwrap();
    assert_eq!(themed.input_route, input_route);
    assert_eq!(themed.output_route, output_route);
    assert!(themed.input.running && themed.output.running);

    config.input.processing.gain_db = -12.0;
    config.output.processing.eq_db[2] = 6.0;
    f.request(Request::Configure {
        config: Box::new(config.clone()),
    });
    f.request(Request::Stop { input: true });
    let s = f.request(Request::Status).status.unwrap();
    assert!(!s.input.running && s.output.running);
    assert_eq!(s.output_route, output_route);
    f.request(Request::Start { input: true });
    f.request(Request::MakeDefault { input: true });
    f.request(Request::MakeDefault { input: false });
    let changed = Command::new(f.dir.join("bin/pactl"))
        .args(["set-default-sink", "later-user-choice"])
        .env("CLEARWAVE_MOCK_DIR", &f.dir)
        .status()
        .unwrap();
    assert!(changed.success());
    f.request(Request::Stop { input: false });
    let s = f.request(Request::Status).status.unwrap();
    assert!(s.input.running && !s.output.running);
    assert_eq!(f.state()["default_sink"], "later-user-choice");
    f.request(Request::Stop { input: true });
    assert_eq!(f.state()["default_source"], "mic");
    assert!(f.state()["modules"].as_object().unwrap().is_empty());
    // Failure after capture launch must reap capture and unload the output module.
    fs::remove_file(f.dir.join("bin/pacat")).unwrap();
    assert!(request_at(
        &f.socket(),
        &Request::Start { input: false },
        Duration::from_secs(15)
    )
    .is_err());
    let s = f.request(Request::Status).status.unwrap();
    assert!(!s.output.running && s.output_route.is_empty());
    assert!(f.state()["modules"].as_object().unwrap().is_empty());
    for entry in fs::read_dir(&f.dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("capture-")
        {
            let pid = fs::read_to_string(path).unwrap();
            assert!(
                !PathBuf::from(format!("/proc/{}", pid.trim())).exists(),
                "capture process leaked"
            );
        }
    }
    f.request(Request::Shutdown);
    f.wait_exit();
    assert!(!f.socket().exists());
    let persisted = Config::load(&f.dir.join("config/clearwave/settings.json")).unwrap();
    assert_eq!(persisted, config);
}

#[test]
fn login_policy_runs_once_and_stop_cancels_missing_device_retry() {
    let mut f = Fixture::new();
    let mut c = Config::default();
    c.input.device = "missing-microphone".into();
    c.output.device = "speakers".into();
    c.input.processing.denoise = false;
    c.startup.output = true;
    c.startup.microphone = true;
    f.request(Request::Configure {
        config: Box::new(c.clone()),
    });
    // Configuring login consent does not activate either path now.
    let s = f.request(Request::Status).status.unwrap();
    assert!(!s.input.running && !s.output.running);
    f.request(Request::Startup);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = f.request(Request::Status).status.unwrap();
        if s.output.running {
            assert!(!s.input.running);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(f.state()["default_sink"], "speakers");
    f.request(Request::StopAll);
    c.input.device = "mic".into();
    f.request(Request::Configure {
        config: Box::new(c),
    });
    f.request(Request::Startup); // duplicate login invocation must not reactivate stopped audio
    std::thread::sleep(Duration::from_millis(2300));
    let s = f.request(Request::Status).status.unwrap();
    assert!(!s.input.running && !s.output.running);
    assert!(f.state()["modules"].as_object().unwrap().is_empty());
    f.request(Request::Shutdown);
    f.wait_exit();
    assert!(!f.socket().exists());
}
#[test]
fn login_can_start_both_saved_paths_with_explicit_defaults() {
    let mut f = Fixture::new();
    let mut c = Config::default();
    c.input.device = "mic".into();
    c.output.device = "speakers".into();
    c.input.processing.denoise = false;
    c.startup.output = true;
    c.startup.microphone = true;
    c.startup.default_input = true;
    c.startup.default_output = true;
    f.request(Request::Configure {
        config: Box::new(c),
    });
    f.request(Request::Startup);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = f.request(Request::Status).status.unwrap();
        if s.input.running && s.output.running {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(f.state()["default_source"], "mic");
    assert_ne!(f.state()["default_sink"], "speakers");
    f.request(Request::StopAll);
    assert_eq!(f.state()["default_source"], "mic");
    assert_eq!(f.state()["default_sink"], "speakers");
    f.request(Request::Shutdown);
    f.wait_exit();
}

#[test]
fn login_waits_for_late_audio_server_without_early_capture() {
    let mut f = Fixture::new();
    fs::write(f.dir.join("server-unavailable"), b"").unwrap();
    let mut c = Config::default();
    c.input.device = "mic".into();
    c.input.processing.denoise = false;
    c.startup.microphone = true;
    f.request(Request::Configure {
        config: Box::new(c),
    });
    f.request(Request::Startup);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!f.request(Request::Status).status.unwrap().input.running);
    fs::remove_file(f.dir.join("server-unavailable")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if f.request(Request::Status).status.unwrap().input.running {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(f.state()["default_source"], "mic");
    f.request(Request::Shutdown);
    f.wait_exit();
}
