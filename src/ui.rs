//! The window sends audio operations to one worker; it never owns an audio stream.
use crate::model::{DeviceKind, Inventory, Settings, Telemetry};
use crate::platform;
use eframe::egui::{self, Color32, RichText};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const ACCENT: Color32 = Color32::from_rgb(91, 220, 189);
const BACKGROUND: Color32 = Color32::from_rgb(14, 21, 30);
const PANEL: Color32 = Color32::from_rgb(23, 32, 44);
const MUTED: Color32 = Color32::from_rgb(151, 167, 183);
const WARNING: Color32 = Color32::from_rgb(255, 191, 115);

enum Command {
    Refresh,
    Start(Settings),
    Stop,
    #[cfg(target_os = "linux")]
    Volume(String, u16),
    #[cfg(target_os = "linux")]
    Default(String, DeviceKind),
    #[cfg(target_os = "linux")]
    MakeDefault,
    #[cfg(target_os = "linux")]
    TestOutput(String),
    Shutdown,
}

enum Event {
    Inventory(Inventory),
    Started(String),
    Stopped,
    Error(String),
    #[cfg(target_os = "linux")]
    Message(String),
}

pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Clearwave")
            .with_inner_size([640.0, 790.0])
            .with_min_inner_size([480.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "clearwave",
        options,
        Box::new(|cc| Ok(Box::new(Clearwave::new(&cc.egui_ctx)?))),
    )
    .map_err(|error| anyhow::anyhow!("Could not open the window: {error}"))
}

struct Clearwave {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    telemetry: Arc<Mutex<Telemetry>>,
    inventory: Option<Inventory>,
    input: String,
    output: String,
    denoise: bool,
    boost_db: f32,
    monitor: bool,
    #[cfg(target_os = "linux")]
    speaker_volume: u16,
    active: bool,
    busy: bool,
    route: String,
    error: Option<String>,
    notice: Option<String>,
    show_shortcuts: bool,
}

impl Clearwave {
    fn new(ctx: &egui::Context) -> anyhow::Result<Self> {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = PANEL;
        visuals.selection.bg_fill = Color32::from_rgb(35, 96, 85);
        visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
        visuals.widgets.active.bg_fill = Color32::from_rgb(35, 96, 85);
        ctx.set_visuals(visuals);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(12.0, 10.0);
            style.spacing.button_padding = egui::vec2(14.0, 9.0);
            style.spacing.interact_size.y = 32.0;
        });

        let (commands, receive_commands) = mpsc::channel();
        let (send_events, events) = mpsc::channel();
        let telemetry = Arc::new(Mutex::new(Telemetry::default()));
        let worker_telemetry = Arc::clone(&telemetry);
        let worker_ctx = ctx.clone();
        let worker = thread::Builder::new()
            .name("clearwave-controls".into())
            .spawn(move || {
                audio_worker(receive_commands, send_events, worker_telemetry, worker_ctx);
            })?;
        commands.send(Command::Refresh)?;
        Ok(Self {
            commands,
            events,
            worker: Some(worker),
            telemetry,
            inventory: None,
            input: String::new(),
            output: String::new(),
            denoise: true,
            boost_db: 0.0,
            monitor: false,
            #[cfg(target_os = "linux")]
            speaker_volume: 100,
            active: false,
            busy: true,
            route: String::new(),
            error: None,
            notice: None,
            show_shortcuts: false,
        })
    }

    fn submit(&mut self, command: Command) {
        if self.busy {
            return;
        }
        self.error = None;
        self.notice = None;
        match self.commands.send(command) {
            Ok(()) => self.busy = true,
            Err(_) => self.error = Some("The audio worker stopped. Restart Clearwave.".into()),
        }
    }

    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.busy = false;
            match event {
                Event::Inventory(inventory) => {
                    if !self.active {
                        retain_or_default(&inventory, DeviceKind::Input, &mut self.input, true);
                        // Windows destinations need an explicit selection: never send a microphone
                        // to the default speakers automatically.
                        retain_or_default(
                            &inventory,
                            DeviceKind::Output,
                            &mut self.output,
                            cfg!(target_os = "linux"),
                        );
                    }
                    self.inventory = Some(inventory);
                    self.sync_speaker_volume();
                }
                Event::Started(route) => {
                    self.active = true;
                    self.route = route;
                }
                Event::Stopped => {
                    self.active = false;
                    self.route.clear();
                }
                Event::Error(error) => self.error = Some(error),
                #[cfg(target_os = "linux")]
                Event::Message(message) => self.notice = Some(message),
            }
        }
    }

    fn sync_speaker_volume(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(volume) = self.inventory.as_ref().and_then(|inventory| {
            inventory
                .devices
                .iter()
                .find(|device| device.id == self.output && device.kind == DeviceKind::Output)
                .and_then(|device| device.volume)
        }) {
            self.speaker_volume = (volume * 100.0).round().clamp(0.0, 150.0) as u16;
        }
    }

    fn toggle_session(&mut self) {
        if self.active {
            self.submit(Command::Stop);
        } else if !self.input.is_empty()
            && (!cfg!(target_os = "windows") || !self.output.is_empty())
        {
            self.submit(Command::Start(Settings {
                input: self.input.clone(),
                output: (!self.output.is_empty()).then(|| self.output.clone()),
                denoise: self.denoise,
                boost_db: self.boost_db,
                monitor: self.monitor,
            }));
        }
    }

    fn input_card(&mut self, ui: &mut egui::Ui, input_peak: f32, output_peak: f32) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("MICROPHONE").color(MUTED).size(11.0).strong());
            ui.add_enabled_ui(!self.active && !self.busy, |ui| {
                device_picker(
                    ui,
                    "microphone",
                    &self.inventory,
                    DeviceKind::Input,
                    &mut self.input,
                );
            });
            ui.add_space(3.0);
            level_meter(ui, "Original", input_peak, self.active);
            level_meter(ui, "Clean output", output_peak, self.active);
            ui.add_space(5.0);
            ui.separator();
            ui.add_enabled_ui(!self.active && !self.busy, |ui| {
                ui.checkbox(&mut self.denoise, "Remove background noise");
                ui.label(
                    RichText::new("Reduce background noise and silence it between phrases.")
                        .small()
                        .color(MUTED),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Voice boost");
                    ui.add(egui::Slider::new(&mut self.boost_db, 0.0..=18.0).suffix(" dB"));
                });
            });
            if self.active {
                ui.label(
                    RichText::new("Stop to change the microphone or processing.")
                        .small()
                        .color(MUTED),
                );
            }
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let heading = if cfg!(target_os = "windows") { "CLEAN AUDIO DESTINATION" } else { "SPEAKERS / HEADPHONES" };
            ui.label(RichText::new(heading).color(MUTED).size(11.0).strong());
            let mut changed = false;
            ui.add_enabled_ui(!self.active && !self.busy, |ui| {
                changed = device_picker(ui, "output", &self.inventory, DeviceKind::Output, &mut self.output);
            });
            if changed {
                self.sync_speaker_volume();
            }
            #[cfg(target_os = "linux")]
            {
                ui.label(RichText::new("Your apps receive Clearwave Microphone. Listening here is optional.").small().color(MUTED));
                if ui.add_enabled(!self.busy && !self.output.is_empty(), egui::Button::new("Test speaker"))
                    .on_hover_text("Play a short, quiet tone through the selected speaker.")
                    .clicked()
                {
                    self.submit(Command::TestOutput(self.output.clone()));
                }
            }
            #[cfg(target_os = "windows")]
            ui.label(RichText::new("Choose a virtual cable's playback endpoint, then select its recording endpoint in your call app. Headphones play your voice locally.").small().color(MUTED));
        });
    }

    fn inspection(&mut self, ui: &mut egui::Ui, clipped_samples: u64) {
        egui::CollapsingHeader::new("Routing & device settings").show(ui, |ui| {
            if let Some(inventory) = &self.inventory {
                ui.label(format!("Audio system: {}", inventory.backend));
                ui.label(RichText::new("Input source").strong());
                ui.add(egui::Label::new(RichText::new(&self.input).monospace()).wrap());
                ui.label(RichText::new("Selected output").strong());
                ui.add(egui::Label::new(RichText::new(&self.output).monospace()).wrap());
                for device in inventory
                    .devices
                    .iter()
                    .filter(|device| device.id == self.input || device.id == self.output)
                {
                    if device.muted == Some(true) {
                        ui.colored_label(
                            WARNING,
                            format!("{} is muted in system settings.", device.name),
                        );
                    }
                }
                for note in &inventory.notes {
                    ui.label(RichText::new(note).small().color(MUTED));
                }
            }
            if !self.route.is_empty() {
                ui.label(RichText::new("Active clean microphone / route").strong());
                ui.add(egui::Label::new(RichText::new(&self.route).monospace()).wrap());
            }
            ui.label(format!(
                "Input clipping detected: {clipped_samples} samples"
            ));
            if clipped_samples > 0 {
                ui.colored_label(
                    WARNING,
                    "Lower the microphone's hardware gain if your voice sounds distorted.",
                );
            }
            #[cfg(target_os = "linux")]
            {
                ui.separator();
                ui.add_enabled_ui(!self.active && !self.busy, |ui| {
                    ui.checkbox(&mut self.monitor, "Listen through selected headphones");
                    if self.monitor {
                        ui.colored_label(WARNING, "Use headphones to avoid feedback.");
                    }
                });
                ui.add_enabled_ui(!self.busy && !self.output.is_empty(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Speaker volume");
                        ui.add(egui::Slider::new(&mut self.speaker_volume, 0..=150).suffix("%"));
                        if ui.button("Apply").clicked() {
                            self.submit(Command::Volume(self.output.clone(), self.speaker_volume));
                        }
                    });
                    if self.speaker_volume > 100 {
                        ui.label(
                            RichText::new("Above 100% can distort speaker audio.")
                                .small()
                                .color(WARNING),
                        );
                    }
                    if ui.button("Use as system speakers").clicked() {
                        self.submit(Command::Default(self.output.clone(), DeviceKind::Output));
                    }
                });
                ui.add_enabled_ui(!self.busy && self.active && !self.route.is_empty(), |ui| {
                    if ui.button("Use clean mic as system default").clicked() {
                        self.submit(Command::MakeDefault);
                    }
                });
            }
            #[cfg(target_os = "windows")]
            ui.hyperlink_to("Open Windows sound settings", "ms-settings:sound");
            ui.add_enabled_ui(!self.busy, |ui| {
                if ui.button("Refresh devices").clicked() {
                    self.submit(Command::Refresh);
                }
            });
        });
    }
}

impl eframe::App for Clearwave {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_millis(50));
        if !ctx.wants_keyboard_input() {
            if ctx.input(|input| input.key_pressed(egui::Key::Space)) && !self.busy {
                self.toggle_session();
            }
            if ctx.input(|input| input.key_pressed(egui::Key::F1)) {
                self.show_shortcuts = !self.show_shortcuts;
            }
        }
        let (input_peak, output_peak, clipped_samples, audio_error) = self
            .telemetry
            .lock()
            .map(|state| {
                (
                    state.input_peak,
                    state.output_peak,
                    state.clipped_samples,
                    state.error.clone(),
                )
            })
            .unwrap_or((
                0.0,
                0.0,
                0,
                Some("Audio status unavailable. Restart Clearwave.".into()),
            ));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(24))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        brand_mark(ui);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Clearwave").size(28.0).strong());
                            ui.label(RichText::new("Your voice, a little clearer.").color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (label, color) = if self.busy {
                                ("Working…", MUTED)
                            } else if self.active {
                                ("● Live", ACCENT)
                            } else {
                                ("● Stopped", MUTED)
                            };
                            ui.label(RichText::new(label).color(color));
                        });
                    });
                    ui.add_space(18.0);
                    self.input_card(ui, input_peak, output_peak);
                    ui.add_space(5.0);
                    self.output_card(ui);
                    ui.add_space(8.0);

                    let enabled = !self.busy
                        && (self.active
                            || (!self.input.is_empty()
                                && (!cfg!(target_os = "windows") || !self.output.is_empty())));
                    let label = if self.busy {
                        "Please wait…"
                    } else if self.active {
                        "Stop microphone"
                    } else {
                        "Start microphone"
                    };
                    let button =
                        egui::Button::new(RichText::new(label).size(16.0).color(BACKGROUND))
                            .fill(ACCENT)
                            .min_size(egui::vec2(ui.available_width(), 46.0));
                    if ui.add_enabled(enabled, button).clicked() {
                        self.toggle_session();
                    }
                    if self.active {
                        #[cfg(target_os = "linux")]
                        ui.label(
                            RichText::new(
                                "Choose “Clearwave Microphone” in your calling or recording app.",
                            )
                            .color(ACCENT),
                        );
                        #[cfg(target_os = "windows")]
                        ui.label(
                            RichText::new("Audio is flowing to your selected destination.")
                                .color(ACCENT),
                        );
                    } else if !self.busy && self.input.is_empty() {
                        ui.label(
                            RichText::new("Connect a microphone, then refresh devices below.")
                                .color(MUTED),
                        );
                    }

                    if let Some(error) = self.error.as_ref().or(audio_error.as_ref()) {
                        egui::Frame::new()
                            .fill(Color32::from_rgb(58, 36, 33))
                            .inner_margin(12)
                            .corner_radius(6)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Needs attention").strong().color(WARNING));
                                ui.add(egui::Label::new(error).wrap());
                            });
                    }
                    if let Some(message) = &self.notice {
                        ui.label(RichText::new(message).small().color(ACCENT));
                    }
                    ui.add_space(4.0);
                    self.inspection(ui, clipped_samples);
                    ui.add_space(10.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Local processing · No uploads · No recording")
                                .size(11.0)
                                .color(MUTED),
                        );
                        if ui.small_button("Keys").clicked() {
                            self.show_shortcuts = !self.show_shortcuts;
                        }
                    });
                });
            });
        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.show_shortcuts)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Space — start or stop the microphone");
                ui.label("F1 — show or hide this guide");
                ui.label("Tab / Shift+Tab — move between controls");
                ui.label("Closing the window stops processing.");
            });
    }
}

impl Drop for Clearwave {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn audio_worker(
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    telemetry: Arc<Mutex<Telemetry>>,
    ctx: egui::Context,
) {
    let mut session: Option<platform::Session> = None;
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let failure = telemetry
                    .lock()
                    .map(|state| state.error.clone())
                    .unwrap_or_else(|_| {
                        Some("Audio status unavailable. Restart Clearwave.".into())
                    });
                if let Some(error) = failure.filter(|_| session.is_some()) {
                    if let Some(mut old) = session.take() {
                        old.stop();
                    }
                    let _ = events.send(Event::Stopped);
                    let _ = events.send(Event::Error(error));
                    ctx.request_repaint();
                }
                continue;
            }
        };
        let result: anyhow::Result<Event> = match command {
            Command::Refresh => platform::enumerate().map(Event::Inventory),
            Command::Start(settings) => {
                if let Some(mut old) = session.take() {
                    old.stop();
                }
                if let Ok(mut state) = telemetry.lock() {
                    *state = Telemetry::default();
                }
                platform::Session::start(settings, Arc::clone(&telemetry)).map(|new_session| {
                    let route = new_session.virtual_source().to_owned();
                    session = Some(new_session);
                    Event::Started(route)
                })
            }
            Command::Stop => {
                if let Some(mut old) = session.take() {
                    old.stop();
                }
                Ok(Event::Stopped)
            }
            #[cfg(target_os = "linux")]
            Command::Volume(id, percent) => platform::set_volume(&id, DeviceKind::Output, percent)
                .and_then(|()| platform::enumerate())
                .map(Event::Inventory),
            #[cfg(target_os = "linux")]
            Command::Default(id, kind) => platform::set_default(&id, kind)
                .and_then(|()| platform::enumerate())
                .map(Event::Inventory),
            #[cfg(target_os = "linux")]
            Command::MakeDefault => match session.as_mut() {
                Some(active) => active.make_default().map(|()| {
                    Event::Message(
                        "Clean microphone is the system default until processing stops.".into(),
                    )
                }),
                None => Err(anyhow::anyhow!("Start the microphone first.")),
            },
            #[cfg(target_os = "linux")]
            Command::TestOutput(id) => platform::test_output(&id).map(|()| {
                Event::Message(
                    "Speaker test finished. If you heard the tone, this output is working.".into(),
                )
            }),
            Command::Shutdown => break,
        };
        let event = result.unwrap_or_else(|error| Event::Error(format!("{error:#}")));
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
    if let Some(mut old) = session.take() {
        old.stop();
    }
}

fn retain_or_default(
    inventory: &Inventory,
    kind: DeviceKind,
    selected: &mut String,
    auto_select: bool,
) {
    if inventory
        .devices
        .iter()
        .any(|device| device.kind == kind && device.id == *selected)
    {
        return;
    }
    *selected = if auto_select {
        inventory
            .devices
            .iter()
            .find(|device| device.kind == kind && device.is_default)
            .or_else(|| inventory.devices.iter().find(|device| device.kind == kind))
            .map(|device| device.id.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };
}

fn device_picker(
    ui: &mut egui::Ui,
    id: &str,
    inventory: &Option<Inventory>,
    kind: DeviceKind,
    selected: &mut String,
) -> bool {
    let name = inventory
        .as_ref()
        .and_then(|inventory| {
            inventory
                .devices
                .iter()
                .find(|device| device.kind == kind && device.id == *selected)
        })
        .map(|device| device.name.as_str())
        .unwrap_or("Select a device");
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(name)
        .show_ui(ui, |ui| {
            if let Some(inventory) = inventory {
                for device in inventory
                    .devices
                    .iter()
                    .filter(|device| device.kind == kind)
                {
                    let label = if device.is_default {
                        format!("{} · system default", device.name)
                    } else {
                        device.name.clone()
                    };
                    changed |= ui
                        .selectable_value(selected, device.id.clone(), label)
                        .clicked();
                }
            }
        });
    changed
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(10)
        .inner_margin(18)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(37, 49, 64)))
}

fn meter_fraction(peak: f32) -> f32 {
    if !peak.is_finite() || peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn level_meter(ui: &mut egui::Ui, label: &str, peak: f32, active: bool) {
    let peak = if active && peak.is_finite() {
        peak.max(0.0)
    } else {
        0.0
    };
    ui.horizontal(|ui| {
        ui.add_sized(
            [92.0, 18.0],
            egui::Label::new(RichText::new(label).small().color(MUTED)),
        );
        let color = if peak >= 0.98 { WARNING } else { ACCENT };
        let width = (ui.available_width() - 78.0).max(30.0);
        ui.add(
            egui::ProgressBar::new(meter_fraction(peak))
                .desired_width(width)
                .desired_height(8.0)
                .fill(color),
        );
        let value = if peak < 0.001 {
            "— dB".to_owned()
        } else {
            format!("{:.0} dB", 20.0 * peak.log10())
        };
        ui.label(RichText::new(value).small().color(MUTED));
    });
}

fn brand_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
    for (index, height) in [12.0, 25.0, 36.0, 22.0, 10.0].iter().enumerate() {
        let x = rect.left() + 6.0 + index as f32 * 7.0;
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.center().y - height / 2.0),
                egui::pos2(x, rect.center().y + height / 2.0),
            ],
            egui::Stroke::new(3.0_f32, ACCENT),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::meter_fraction;

    #[test]
    fn meter_handles_silence_invalid_samples_and_clipping() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_fraction(f32::NAN), 0.0);
        assert_eq!(meter_fraction(-1.0), 0.0);
        assert_eq!(meter_fraction(2.0), 1.0);
        assert!((meter_fraction(0.1) - 2.0 / 3.0).abs() < 0.001);
    }
}
