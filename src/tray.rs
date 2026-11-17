//! SNI service owned by the daemon, independent of any control window.
use crate::engine::{Request, Status};
use ksni::{
    blocking::TrayMethods,
    menu::{CheckmarkItem, StandardItem},
    MenuItem,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc, Mutex,
};
#[derive(Clone)]
pub struct Indicator {
    state: Status,
    commands: SyncSender<Request>,
    online: Arc<AtomicBool>,
}
impl Indicator {
    fn send(&self, r: Request) {
        if self.commands.try_send(r).is_err() {
            eprintln!("ClearWave: tray command queue full or engine unavailable");
        }
    }
    fn item(&self, label: String, r: Request) -> MenuItem<Self> {
        StandardItem {
            label,
            activate: Box::new(move |t: &mut Self| t.send(r.clone())),
            ..Default::default()
        }
        .into()
    }
}
pub fn summary(s: &Status) -> String {
    format!(
        "ClearWave — Output {} · Microphone {}{}",
        if s.output.running { "on" } else { "off" },
        if s.input.running { "on" } else { "off" },
        if s.warning.is_some() || s.input.error.is_some() || s.output.error.is_some() {
            " · Error"
        } else {
            ""
        }
    )
}
impl ksni::Tray for Indicator {
    fn id(&self) -> String {
        "clearwave".into()
    }
    fn title(&self) -> String {
        summary(&self.state)
    }
    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }
    fn icon_name(&self) -> String {
        // An unresolved theme name can take precedence over IconPixmap in panels.
        // Always use our self-contained logo, including standalone/AppImage runs.
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static LOGO: std::sync::OnceLock<Vec<ksni::Icon>> = std::sync::OnceLock::new();
        let mut icons = LOGO
            .get_or_init(|| {
                let logo = image::load_from_memory(include_bytes!("../assets/clearwave.png"))
                    .expect("bundled ClearWave logo must be a valid PNG");
                [32, 64]
                    .into_iter()
                    .map(|size| {
                        let rgba = logo
                            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                            .into_rgba8();
                        // SNI requires network-order ARGB32; image returns RGBA.
                        let data = rgba
                            .pixels()
                            .flat_map(|p| [p[3], p[0], p[1], p[2]])
                            .collect();
                        ksni::Icon {
                            width: size as i32,
                            height: size as i32,
                            data,
                        }
                    })
                    .collect()
            })
            .clone();
        let error = self.state.warning.is_some()
            || self.state.input.error.is_some()
            || self.state.output.error.is_some();
        let running = self.state.input.running || self.state.output.running;
        if error || running {
            for icon in &mut icons {
                let size = icon.width as usize;
                for y in 0..size / 3 {
                    for x in size * 2 / 3..size {
                        // Square error vs round processing badge: status isn't color-only.
                        let dx = x as f32 - size as f32 * 5.0 / 6.0;
                        let dy = y as f32 - size as f32 / 6.0;
                        if error || dx * dx + dy * dy < (size as f32 / 6.0).powi(2) {
                            let i = (y * size + x) * 4;
                            icon.data[i..i + 4].copy_from_slice(if error {
                                &[255, 245, 160, 70]
                            } else {
                                &[255, 101, 229, 210]
                            });
                        }
                    }
                }
            }
        }
        icons
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: summary(&self.state),
            description: self
                .state
                .warning
                .clone()
                .or(self.state.input.error.clone())
                .or(self.state.output.error.clone())
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Request::Open);
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Release);
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Release);
        // Recreate through spawn's host-availability check after watcher loss.
        false
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            self.item("Open ClearWave".into(), Request::Open),
            StandardItem {
                label: format!(
                    "Output: {}",
                    if self.state.output.running {
                        "On"
                    } else {
                        "Off"
                    }
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: format!(
                    "Microphone: {}",
                    if self.state.input.running {
                        "On"
                    } else {
                        "Off"
                    }
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            self.item(
                format!(
                    "{} output",
                    if self.state.output.running {
                        "Stop"
                    } else {
                        "Start"
                    }
                ),
                if self.state.output.running {
                    Request::Stop { input: false }
                } else {
                    Request::Start { input: false }
                },
            ),
            self.item(
                format!(
                    "{} microphone",
                    if self.state.input.running {
                        "Stop"
                    } else {
                        "Start"
                    }
                ),
                if self.state.input.running {
                    Request::Stop { input: true }
                } else {
                    Request::Start { input: true }
                },
            ),
            self.item("Stop all processing".into(), Request::StopAll),
            CheckmarkItem {
                label: "Start at login".into(),
                checked: self.state.autostart.enabled,
                activate: Box::new(|t: &mut Self| t.send(Request::ToggleAutostart)),
                ..Default::default()
            }
            .into(),
            self.item("Quit ClearWave".into(), Request::Shutdown),
        ]
    }
}
pub struct Service {
    pub online: Arc<AtomicBool>,
    pub snapshot: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
}
impl Service {
    pub fn start(commands: SyncSender<Request>) -> Self {
        let online = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Status::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (o, s, q) = (online.clone(), snapshot.clone(), stop.clone());
        std::thread::spawn(move || {
            while !q.load(Ordering::Acquire) {
                o.store(false, Ordering::Release);
                let tray = Indicator {
                    state: s.lock().unwrap().clone(),
                    commands: commands.clone(),
                    online: o.clone(),
                };
                match tray.spawn() {
                    Ok(handle) => {
                        o.store(true, Ordering::Release);
                        let mut previous = None;
                        while !q.load(Ordering::Acquire) && !handle.is_closed() {
                            let state = s.lock().unwrap().clone();
                            let key = (
                                summary(&state),
                                state.autostart.clone(),
                                state.warning.clone(),
                                state.input.error.clone(),
                                state.output.error.clone(),
                            );
                            if previous.as_ref() != Some(&key) {
                                handle.update(|t| t.state = state);
                                previous = Some(key);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(250));
                        }
                        handle.shutdown().wait();
                    }
                    Err(e) => {
                        o.store(false, Ordering::Release);
                        eprintln!("ClearWave tray unavailable: {e}");
                    }
                }
                o.store(false, Ordering::Release);
                for _ in 0..20 {
                    if q.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        });
        Self {
            online,
            snapshot,
            stop,
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_is_active_and_status_truthful() {
        let (tx, _) = std::sync::mpsc::sync_channel(1);
        let mut t = Indicator {
            state: Status::default(),
            commands: tx,
            online: Arc::default(),
        };
        assert!(summary(&t.state).contains("Microphone off"));
        t.state.input.running = true;
        assert!(summary(&t.state).contains("Microphone on"));
        assert_eq!(ksni::Tray::menu(&t).len(), 8);
        assert_eq!(ksni::Tray::status(&t), ksni::Status::Active);
    }
}
