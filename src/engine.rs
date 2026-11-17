//! Per-user engine. A private Unix socket is the only control surface.
use crate::{
    model::{Inventory, Settings, Telemetry},
    platform::{self, OutputSession, Session},
    settings::{self, Config},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
const LIMIT: u64 = 65536;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Open,
    Startup,
    StopAll,
    SetAutostart { enabled: bool },
    ToggleAutostart,
    Devices,
    Configure { config: Box<Config> },
    Start { input: bool },
    Stop { input: bool },
    MakeDefault { input: bool },
    Shutdown,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub autostart: crate::autostart::State,
    pub tray_available: bool,
    pub config: Config,
    pub input: Telemetry,
    pub output: Telemetry,
    pub input_route: String,
    pub output_route: String,
    pub warning: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub error: Option<String>,
    pub status: Option<Status>,
    pub inventory: Option<Inventory>,
}
fn private_dir(path: &Path) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let m = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no preconditions and does not modify memory.
    ensure!(
        m.is_dir()
            && !m.file_type().is_symlink()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "Unsafe runtime directory {}: require owned directory mode 0700",
        path.display()
    );
    Ok(())
}
pub fn runtime_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    // SAFETY: geteuid has no preconditions.
    let path = base
        .map(|p| p.join("clearwave"))
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/clearwave-{}", unsafe { libc::geteuid() })));
    private_dir(&path)?;
    Ok(path)
}
struct SocketOwner {
    path: PathBuf,
    _lock: File,
}
impl SocketOwner {
    fn bind(dir: &Path) -> Result<(Self, UnixListener)> {
        private_dir(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(dir.join("engine.lock"))?;
        // SAFETY: valid owned descriptor; lock held for the lifetime of SocketOwner.
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "ClearWave engine is already running"
        );
        let path = dir.join("engine.sock");
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok((Self { path, _lock: lock }, listener))
    }
}
impl Drop for SocketOwner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn read_message(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let deadline = Instant::now() + stream.read_timeout()?.unwrap_or(Duration::from_secs(2));
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("IPC message deadline exceeded")?;
        stream.set_read_timeout(Some(remaining))?;
        let n = stream.read(&mut buffer)?;
        ensure!(n > 0, "Incomplete IPC message");
        let end = buffer[..n].iter().position(|b| *b == b'\n');
        bytes.extend_from_slice(&buffer[..end.unwrap_or(n)]);
        ensure!(bytes.len() as u64 <= LIMIT, "IPC message exceeds 64 KiB");
        if end.is_some() {
            return Ok(bytes);
        }
    }
}
fn write_message(stream: &mut UnixStream, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() as u64 <= LIMIT, "IPC response exceeds 64 KiB");
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
pub fn request_at(path: &Path, request: &Request, timeout: Duration) -> Result<Response> {
    let mut socket = UnixStream::connect(path).context("ClearWave engine is not available")?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    write_message(&mut socket, request)?;
    let response: Response = serde_json::from_slice(&read_message(&mut socket)?)?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "Engine operation failed".into())
        );
    }
    Ok(response)
}
pub fn request(request: &Request) -> Result<Response> {
    request_at(
        &runtime_dir()?.join("engine.sock"),
        request,
        Duration::from_secs(90),
    )
}
pub fn ensure_running() -> Result<()> {
    let path = runtime_dir()?.join("engine.sock");
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(crate::autostart::executable()?);
    // SAFETY: setsid is async-signal-safe; no allocation or shared state in pre_exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // Detach a reaper rather than leaving a zombie if the child loses the singleton race.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..100 {
        if UnixStream::connect(&path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    bail!("Engine did not start; run `clearwave daemon` for diagnostics")
}
struct Engine {
    status: Status,
    input: Option<Session>,
    output: Option<OutputSession>,
    input_levels: Arc<Mutex<Telemetry>>,
    output_levels: Arc<Mutex<Telemetry>>,
    path: PathBuf,
    save_at: Option<Instant>,
    startup_pending: [bool; 2],
    startup_attempts: u8,
    startup_at: Option<Instant>,
}
impl Engine {
    fn new(path: PathBuf) -> Self {
        let (config, warning) = match Config::load(&path) {
            Ok(c) => (c, None),
            Err(e) => (
                Config::default(),
                Some(format!("{}: {e:#}", path.display())),
            ),
        };
        Self {
            status: Status {
                config,
                warning,
                ..Default::default()
            },
            input: None,
            output: None,
            input_levels: Arc::default(),
            output_levels: Arc::default(),
            path,
            save_at: None,
            startup_pending: [false; 2],
            startup_attempts: 0,
            startup_at: None,
        }
    }
    fn snapshot(&mut self) -> Status {
        self.status.autostart = crate::autostart::inspect();
        self.status.input = self
            .input_levels
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        self.status.output = self
            .output_levels
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        self.status.clone()
    }
    fn start(&mut self, input: bool) -> Result<()> {
        if input {
            self.status.input_route.clear();
            if let Some(mut old) = self.input.take() {
                old.stop();
            }
            let c = &self.status.config;
            let settings = Settings {
                input: c.input.device.clone(),
                output: (!c.headphones.is_empty()).then(|| c.headphones.clone()),
                denoise: c.input.processing.denoise,
                boost_db: c.input.processing.gain_db,
                monitor: c.monitor,
            };
            let s =
                Session::start_processed(settings, c.input.processing, self.input_levels.clone())?;
            self.status.input_route = s.virtual_source().into();
            self.input = Some(s);
        } else {
            self.status.output_route.clear();
            if let Some(mut old) = self.output.take() {
                old.stop();
            }
            let c = &self.status.config;
            let s = OutputSession::start(
                &c.output.device,
                c.output.processing,
                self.output_levels.clone(),
            )?;
            self.status.output_route = s.virtual_sink().into();
            self.output = Some(s);
        }
        Ok(())
    }
    fn stop(&mut self, input: bool) {
        self.startup_pending[usize::from(input)] = false;
        if input {
            if let Some(mut s) = self.input.take() {
                s.stop();
            }
            self.status.input_route.clear();
        } else {
            if let Some(mut s) = self.output.take() {
                s.stop();
            }
            self.status.output_route.clear();
        }
    }
    fn persist(&mut self) {
        if self.save_at.take().is_some() {
            if let Err(e) = self.status.config.save(&self.path) {
                self.status.warning = Some(format!("Cannot save settings: {e:#}"));
            }
        }
    }
    fn configure(&mut self, c: Config) -> Result<()> {
        c.validate()?;
        self.save_at = Some(Instant::now() + Duration::from_millis(500));
        let restart_input = self.input.is_some()
            && (self.status.config.input.device != c.input.device
                || self.status.config.monitor != c.monitor
                || self.status.config.headphones != c.headphones);
        let restart_output =
            self.output.is_some() && self.status.config.output.device != c.output.device;
        let input_dsp_changed = self.status.config.input.processing != c.input.processing;
        let output_dsp_changed = self.status.config.output.processing != c.output.processing;
        self.status.config = c;
        self.status.warning = None;
        // A device restart affects only its own pipeline. Failures are visible in telemetry.
        if restart_input {
            if let Err(e) = self.start(true) {
                self.input_levels
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Status unavailable"))?
                    .error = Some(format!("Input restart failed: {e:#}"));
                return Err(e);
            }
        } else if input_dsp_changed {
            if let Some(s) = &self.input {
                s.update(self.status.config.input.processing)?;
            }
        }
        if restart_output {
            if let Err(e) = self.start(false) {
                self.output_levels
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Status unavailable"))?
                    .error = Some(format!("Output restart failed: {e:#}"));
                return Err(e);
            }
        } else if output_dsp_changed {
            if let Some(s) = &self.output {
                s.update(self.status.config.output.processing)?;
            }
        }
        Ok(())
    }
    fn startup_tick(&mut self) {
        if !self.startup_at.is_some_and(|t| Instant::now() >= t) {
            return;
        }
        self.startup_attempts += 1;
        for input in [false, true] {
            let index = usize::from(input);
            if !self.startup_pending[index] {
                continue;
            }
            let id = if input {
                &self.status.config.input.device
            } else {
                &self.status.config.output.device
            };
            let readiness = (|| -> Result<()> {
                ensure!(
                    !id.is_empty(),
                    "Select and save a {} device before enabling automatic processing",
                    if input { "microphone" } else { "output" }
                );
                let inventory = platform::enumerate()?;
                ensure!(
                    inventory.devices.iter().any(|d| d.id == *id
                        && d.kind
                            == if input {
                                crate::model::DeviceKind::Input
                            } else {
                                crate::model::DeviceKind::Output
                            }),
                    "Saved {} device is unavailable: {}",
                    if input { "microphone" } else { "output" },
                    id
                );
                Ok(())
            })();
            if let Err(e) = readiness {
                let levels = if input {
                    &self.input_levels
                } else {
                    &self.output_levels
                };
                if let Ok(mut t) = levels.lock() {
                    t.error = Some(format!("Login startup: {e:#}"));
                }
                continue;
            }
            // Only readiness is retried. Processing/default failures are terminal and visible.
            self.startup_pending[index] = false;
            let result = self.start(input).and_then(|()| {
                if input && self.status.config.startup.default_input {
                    self.input
                        .as_mut()
                        .context("Input unavailable")?
                        .make_default()?;
                }
                if !input && self.status.config.startup.default_output {
                    self.output
                        .as_mut()
                        .context("Output unavailable")?
                        .make_default()?;
                }
                Ok(())
            });
            if let Err(e) = result {
                let levels = if input {
                    &self.input_levels
                } else {
                    &self.output_levels
                };
                if let Ok(mut t) = levels.lock() {
                    t.error = Some(format!("Login startup failed: {e:#}"));
                }
            }
        }
        if self.startup_pending.iter().any(|p| *p) && self.startup_attempts < 20 {
            self.startup_at = Some(Instant::now() + Duration::from_secs(2));
        } else {
            self.startup_at = None;
            self.startup_pending = [false; 2];
        }
    }
    fn handle(&mut self, request: Request) -> Result<Option<Inventory>> {
        match request {
            Request::Status => {}
            Request::Open => {
                let mut child = std::process::Command::new(crate::autostart::executable()?)
                    .arg("open")
                    .env("CLEARWAVE_EXISTING_ENGINE_ONLY", "1")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Request::SetAutostart { enabled } => crate::autostart::set(enabled)?,
            Request::ToggleAutostart => {
                crate::autostart::set(!crate::autostart::inspect().enabled)?;
            }
            Request::Startup => {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                std::env::var("XDG_SESSION_ID")
                    .unwrap_or_default()
                    .hash(&mut hash);
                std::env::var("DBUS_SESSION_BUS_ADDRESS")
                    .unwrap_or_default()
                    .hash(&mut hash);
                let marker = runtime_dir()?.join(format!("startup-{:x}", hash.finish()));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(marker)
                {
                    Ok(_) => {
                        self.startup_pending = [
                            self.status.config.startup.output,
                            self.status.config.startup.microphone,
                        ];
                        self.startup_attempts = 0;
                        self.startup_at = Some(Instant::now());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Request::StopAll => {
                self.stop(true);
                self.stop(false);
            }

            Request::Devices => return Ok(Some(platform::enumerate()?)),
            Request::Configure { config } => self.configure(*config)?,
            Request::Start { input } => {
                self.startup_pending[usize::from(input)] = false;
                if let Err(e) = self.start(input) {
                    let levels = if input {
                        &self.input_levels
                    } else {
                        &self.output_levels
                    };
                    if let Ok(mut t) = levels.lock() {
                        t.error = Some(format!("Start failed: {e:#}"));
                    }
                    return Err(e);
                }
            }
            Request::Stop { input } => self.stop(input),
            Request::MakeDefault { input } => {
                if input {
                    self.input
                        .as_mut()
                        .context("Start input first")?
                        .make_default()?;
                } else {
                    self.output
                        .as_mut()
                        .context("Start output first")?
                        .make_default()?;
                }
            }
            Request::Shutdown => {
                self.stop(true);
                self.stop(false);
            }
        }
        Ok(None)
    }
}
pub fn run_daemon() -> Result<()> {
    let dir = runtime_dir()?;
    let (_owner, listener) = SocketOwner::bind(&dir)?;
    let mut engine = Engine::new(settings::config_path()?);
    #[cfg(feature = "tray")]
    let (tray_tx, tray_rx) = std::sync::mpsc::sync_channel(16);
    #[cfg(feature = "tray")]
    let tray = crate::tray::Service::start(tray_tx);

    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = interrupted.clone();
    let socket = dir.join("engine.sock");
    ctrlc::set_handler(move || {
        signal.store(true, Ordering::Release);
        let _ = UnixStream::connect(&socket);
    })?;
    loop {
        if interrupted.load(Ordering::Acquire) {
            break;
        }
        #[cfg(feature = "tray")]
        {
            engine.status.tray_available = tray.online.load(Ordering::Acquire);
            while let Ok(command) = tray_rx.try_recv() {
                let quit = matches!(command, Request::Shutdown);
                if let Err(e) = engine.handle(command) {
                    engine.status.warning = Some(format!("Tray action failed: {e:#}"));
                }
                if quit {
                    interrupted.store(true, Ordering::Release);
                    break;
                }
            }
            if let Ok(mut state) = tray.snapshot.lock() {
                *state = engine.snapshot();
            }
        }
        if interrupted.load(Ordering::Acquire) {
            break;
        }
        engine.startup_tick();
        if engine.save_at.is_some_and(|t| Instant::now() >= t) {
            engine.persist();
        }
        let timeout = 250;
        let mut fd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid descriptor and valid writable pollfd pointer.
        let result = unsafe { libc::poll(&mut fd, 1, timeout) };
        if result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(std::io::Error::last_os_error().into());
        }
        if result == 0 {
            continue;
        }
        let stream = listener.accept().map(|(s, _)| s);
        if interrupted.load(Ordering::Acquire) {
            break;
        }
        let mut stream = stream?;
        stream.set_read_timeout(Some(Duration::from_millis(250)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let parsed =
            read_message(&mut stream).and_then(|b| Ok(serde_json::from_slice::<Request>(&b)?));
        let shutdown = matches!(parsed, Ok(Request::Shutdown));
        let result = parsed.and_then(|request| engine.handle(request));
        let (inventory, error) = match result {
            Ok(i) => (i, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let response = Response {
            ok: error.is_none(),
            error,
            status: Some(engine.snapshot()),
            inventory,
        };
        let _ = write_message(&mut stream, &response);
        if shutdown {
            break;
        }
    }
    if let Ok(socket) = std::os::unix::net::UnixDatagram::unbound() {
        let _ = socket.send_to(b"quit", dir.join("gui.sock"));
    }
    engine.persist();
    engine.stop(true);
    engine.stop(false);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("clearwave-ipc-test-{}", std::process::id()));
        private_dir(&p).unwrap();
        p
    }
    #[test]
    fn malformed_settings_keep_engine_idle() {
        let path = std::env::temp_dir().join(format!(
            "clearwave-engine-malformed-{}.json",
            std::process::id()
        ));
        fs::write(&path, b"{bad}").unwrap();
        let mut engine = Engine::new(path.clone());
        let status = engine.snapshot();
        assert!(status.warning.is_some());
        assert!(!status.input.running && !status.output.running);
        assert!(engine.input.is_none() && engine.output.is_none());
        assert_eq!(fs::read(&path).unwrap(), b"{bad}");
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn message_deadline_cannot_be_extended_by_dribbling_bytes() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        b.set_read_timeout(Some(Duration::from_millis(40))).unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..30 {
                if a.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        assert!(read_message(&mut b).is_err());
        assert!(started.elapsed() < Duration::from_millis(150));
        drop(b);
        writer.join().unwrap();
    }
    #[test]
    fn singleton_cleanup_and_timeout() {
        let dir = dir();
        let (owner, listener) = SocketOwner::bind(&dir).unwrap();
        assert!(SocketOwner::bind(&dir).is_err());
        let path = dir.join("engine.sock");
        let t = std::thread::spawn(move || {
            let (_s, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        assert!(request_at(&path, &Request::Status, Duration::from_millis(20)).is_err());
        t.join().unwrap();
        drop(owner);
        assert!(!path.exists());
        let (owner, _) = SocketOwner::bind(&dir).unwrap();
        drop(owner);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn bounded_and_malformed_messages() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let t = std::thread::spawn(move || {
            let mut a = a;
            let _ = a.write_all(&vec![b'x'; 65538]);
        });
        assert!(read_message(&mut b).is_err());
        drop(b);
        t.join().unwrap();
        assert!(serde_json::from_str::<Request>(
            r#"{"command":"configure","config":{"version":9}}"#
        )
        .unwrap()
        .validate_for_test()
        .is_err());
    }
    impl Request {
        fn validate_for_test(&self) -> Result<()> {
            match self {
                Request::Configure { config } => config.validate(),
                _ => Ok(()),
            }
        }
    }
}
