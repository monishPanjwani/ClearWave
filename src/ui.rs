//! An asynchronous control surface. The daemon owns every audio resource.
use clearwave::{
    engine::{self, Request, Status},
    model::{DeviceKind, Inventory},
    settings::{Config, Processing, SavedPreset, FREQUENCIES},
    theme::{Mode, ThemeManager, FOLLOW_OMARCHY},
};
use eframe::egui::{self, Color32, RichText};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Output,
    Input,
    Advanced,
}
enum Job {
    Reconnect,
    Request(Request),
    Close(bool),
}
enum Event {
    Initial(Status),
    Reply(Status),
    Inventory(Inventory),
    Error(String),
    Tick(Status),
}
pub fn run() -> anyhow::Result<()> {
    eframe::run_native(
        "ClearWave",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("ClearWave")
                .with_inner_size(if std::env::var_os("CLEARWAVE_CAPTURE_MINIMUM").is_some() {
                    [520.0, 620.0]
                } else {
                    [720.0, 810.0]
                })
                .with_min_inner_size([520.0, 620.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
    .map_err(|e| anyhow::anyhow!("Cannot open ClearWave: {e}"))
}
struct App {
    jobs: mpsc::Sender<Job>,
    events: mpsc::Receiver<Event>,
    page: Page,
    config: Config,
    status: Status,
    inventory: Inventory,
    ready: bool,
    busy: bool,
    error: Option<String>,
    dirty: Option<Instant>,
    preset_name: String,
    meters: [f32; 4],
    last_frame: Instant,
    closed: bool,
    last_sent: Instant,
    worker: Option<thread::JoinHandle<()>>,
    theme: ThemeManager,
    capture_stage: usize,
    capture_requested: bool,
}
impl App {
    fn new(ctx: &egui::Context) -> Self {
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(12.0, 10.0);
            s.spacing.button_padding = egui::vec2(14.0, 9.0);
            s.spacing.interact_size.y = 32.0;
        });
        let (jobs, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = thread::spawn(move || worker(rx, tx, ctx));
        Self {
            jobs,
            events,
            page: Page::Output,
            config: Config::default(),
            status: Status::default(),
            inventory: Inventory::default(),
            ready: false,
            busy: true,
            error: None,
            dirty: None,
            preset_name: String::new(),
            meters: [0.0; 4],
            last_frame: Instant::now(),
            closed: false,
            last_sent: Instant::now(),
            worker: Some(worker),
            theme: ThemeManager::new(None),
            capture_stage: 0,
            capture_requested: false,
        }
    }
    fn submit(&mut self, request: Request) {
        if self.jobs.send(Job::Request(request)).is_ok() {
            self.busy = true;
            self.error = None;
        } else {
            self.error = Some("Control worker stopped. Reopen ClearWave.".into());
        }
    }
    fn flush(&mut self) {
        self.last_sent = Instant::now();
        if self.dirty.take().is_some() {
            self.submit(Request::Configure {
                config: Box::new(self.config.clone()),
            });
        }
    }
    fn poll(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            match e {
                Event::Initial(s) => {
                    self.error = None;
                    self.config = s.config.clone();
                    self.theme = ThemeManager::new(self.config.theme.as_deref());
                    self.status = s;
                    self.ready = true;
                    self.busy = false;
                }
                Event::Reply(s) => {
                    self.status = s;
                    self.busy = false;
                }
                Event::Tick(s) => self.status = s,
                Event::Inventory(i) => {
                    self.inventory = i;
                    for (input, channel) in [
                        (true, &mut self.config.input),
                        (false, &mut self.config.output),
                    ] {
                        if channel.device.is_empty() {
                            let kind = if input {
                                DeviceKind::Input
                            } else {
                                DeviceKind::Output
                            };
                            if let Some(d) = self
                                .inventory
                                .devices
                                .iter()
                                .find(|d| d.kind == kind && d.is_default)
                                .or_else(|| self.inventory.devices.iter().find(|d| d.kind == kind))
                            {
                                channel.device = d.id.clone();
                                self.dirty = Some(Instant::now());
                            }
                        }
                    }
                    self.busy = false;
                }
                Event::Error(e) => {
                    self.error = Some(e);
                    self.busy = false;
                }
            }
        }
    }
    fn channel_page(&mut self, ui: &mut egui::Ui, input: bool) {
        let running = if input {
            self.status.input.running
        } else {
            self.status.output.running
        };
        ui.label(
            RichText::new(if input {
                "A clearer voice."
            } else {
                "Hear the detail."
            })
            .size(28.0)
            .strong(),
        );
        ui.label(
            RichText::new(if input {
                "Private microphone cleanup, shaped for your voice."
            } else {
                "Shape your music, films and everyday listening."
            })
            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
        ui.add_space(12.0);
        let label = if running {
            if input {
                "Stop input"
            } else {
                "Stop output"
            }
        } else if input {
            "Enable input"
        } else {
            "Enable output"
        };
        if ui
            .add_enabled(
                self.ready && !self.busy,
                egui::Button::new(RichText::new(label).color(button_text(ui)).strong())
                    .fill(ui.visuals().selection.stroke.color)
                    .min_size(egui::vec2(ui.available_width(), 44.0)),
            )
            .clicked()
        {
            self.flush();
            self.submit(if running {
                Request::Stop { input }
            } else {
                Request::Start { input }
            });
        }
        let before = self.config.clone();
        let channel = if input {
            &mut self.config.input
        } else {
            &mut self.config.output
        };
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(if input { "MICROPHONE" } else { "PLAY THROUGH" })
                    .small()
                    .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
            );
            picker(
                ui,
                if input { "input" } else { "output" },
                &self.inventory,
                if input {
                    DeviceKind::Input
                } else {
                    DeviceKind::Output
                },
                &mut channel.device,
            );
            ui.add_space(4.0);
            let presets: &[&str] = if input {
                &["Natural", "Clear Speech", "Warm Voice", "Noisy Room"]
            } else {
                &["Neutral", "Music", "Movies", "Voice"]
            };
            egui::ComboBox::from_id_salt("preset")
                .selected_text(&channel.preset)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for name in presets {
                        if ui
                            .selectable_label(channel.preset == *name, *name)
                            .clicked()
                        {
                            channel.preset = (*name).into();
                            channel.processing = Processing::preset(name, input);
                        }
                    }
                    for preset in self.config.presets.iter().filter(|p| p.input == input) {
                        if ui
                            .selectable_label(channel.preset == preset.name, &preset.name)
                            .clicked()
                        {
                            channel.preset = preset.name.clone();
                            channel.processing = preset.processing;
                        }
                    }
                });
            let previous = channel.processing;
            ui.add_space(5.0);
            slider(
                ui,
                "Processed gain",
                &mut channel.processing.gain_db,
                -36.0..=18.0,
                " dB",
            );
            if input {
                ui.checkbox(&mut channel.processing.denoise, "Remove background noise");
            }
            if channel.processing != previous {
                channel.preset = "Custom".into();
            }
            let offset = if input { 0 } else { 2 };
            meter(
                ui,
                if input { "Original input" } else { "Incoming" },
                self.meters[offset],
            );
            meter(ui, "Processed", self.meters[offset + 1]);
        });
        ui.add_space(10.0);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("EQUALIZER")
                    .small()
                    .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
            );
            let before = channel.processing;
            ui.columns(5, |columns| {
                for ((ui, f), gain) in columns
                    .iter_mut()
                    .zip(FREQUENCIES)
                    .zip(channel.processing.eq_db.iter_mut())
                {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(if f >= 1000.0 {
                                format!("{}k", f / 1000.0)
                            } else {
                                format!("{f}")
                            })
                            .small()
                            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
                        );
                        ui.spacing_mut().slider_width = 95.0;
                        ui.add(
                            egui::Slider::new(gain, -12.0..=12.0)
                                .vertical()
                                .show_value(false),
                        );
                        ui.label(RichText::new(format!("{gain:+.1}")).small());
                    });
                }
            });
            if !input {
                ui.separator();
                slider(
                    ui,
                    "Bass",
                    &mut channel.processing.bass_db,
                    0.0..=9.0,
                    " dB",
                );
                slider(
                    ui,
                    "Clarity",
                    &mut channel.processing.clarity_db,
                    0.0..=6.0,
                    " dB",
                );
                slider(
                    ui,
                    "Stereo width",
                    &mut channel.processing.width,
                    0.0..=2.0,
                    "×",
                );
            }
            if channel.processing != before {
                channel.preset = "Custom".into();
            }
            if ui.small_button("Reset to neutral").clicked() {
                channel.processing = Processing::default();
                channel.preset = "Neutral".into();
            }
        });
        if self.config != before {
            self.dirty = Some(Instant::now());
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(if input {
                "Choose “Clearwave Microphone” in your call app after enabling."
            } else {
                "Choose “ClearWave Output” in your app or use the routing controls in Advanced."
            })
            .small()
            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
    }
    fn advanced(&mut self, ui: &mut egui::Ui) {
        ui.heading("Make it yours.");
        ui.label(
            RichText::new("Fine adjustments, routing and background behavior.")
                .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
        ui.add_space(10.0);
        let before = self.config.clone();
        card(ui).show(ui,|ui|{ui.set_width(ui.available_width());ui.strong("Dynamics & rumble");
    for(input,channel) in [(false,&mut self.config.output),(true,&mut self.config.input)] {ui.push_id(input,|ui|{ui.label(if input{"Input"}else{"Output"});let old=channel.processing;slider(ui,"Compressor threshold",&mut channel.processing.threshold_db,-48.0..=0.0," dB");slider(ui,"Ratio",&mut channel.processing.ratio,1.0..=10.0,":1");slider(ui,"Low cut",&mut channel.processing.low_cut_hz,0.0..=300.0," Hz");if channel.processing!=old{channel.preset="Custom".into();}});}
    ui.label(RichText::new("Ratio 1:1 bypasses compression. A linked peak limiter always protects processed samples.").small().color(ui.visuals().widgets.noninteractive.fg_stroke.color));
  });
        ui.add_space(10.0);
        card(ui).show(ui,|ui|{ui.set_width(ui.available_width());ui.strong("Monitoring & background");ui.checkbox(&mut self.config.monitor,"Listen to the processed microphone");picker(ui,"headphones",&self.inventory,DeviceKind::Output,&mut self.config.headphones);ui.label(RichText::new("Explicitly choose headphones to avoid feedback. Monitoring starts only with input.").small().color(ui.visuals().widgets.noninteractive.fg_stroke.color));
    ui.checkbox(&mut self.config.background,"Keep audio running when this window closes");ui.label(RichText::new("Opening ClearWave never starts microphone capture. Login service setup is opt-in; see README.").small().color(ui.visuals().widgets.noninteractive.fg_stroke.color));
  });
        ui.add_space(10.0);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Save a preset");
            ui.text_edit_singleline(&mut self.preset_name);
            ui.horizontal_wrapped(|ui| {
                for input in [false, true] {
                    if ui
                        .add_enabled(
                            !self.preset_name.trim().is_empty()
                                && self.preset_name.len() <= 64
                                && self.config.presets.len() < 32,
                            egui::Button::new(if input { "Save input" } else { "Save output" }),
                        )
                        .clicked()
                    {
                        let name = self.preset_name.trim().to_owned();
                        let processing = if input {
                            self.config.input.processing
                        } else {
                            self.config.output.processing
                        };
                        self.config
                            .presets
                            .retain(|p| p.name != name || p.input != input);
                        self.config.presets.push(SavedPreset {
                            name,
                            input,
                            processing,
                        });
                    }
                }
            });
            let mut delete = None;
            for (i, p) in self.config.presets.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} · {}",
                        p.name,
                        if p.input { "Input" } else { "Output" }
                    ));
                    if ui.small_button("Delete").clicked() {
                        delete = Some(i);
                    }
                });
            }
            if let Some(i) = delete {
                self.config.presets.remove(i);
            }
        });
        if self.config != before {
            self.dirty = Some(Instant::now());
        }
        ui.add_space(10.0);
        card(ui).show(ui,|ui|{ui.set_width(ui.available_width());ui.strong("Routing & diagnostics");ui.label(format!("Audio server: {}",self.inventory.backend));
    ui.add_enabled_ui(!self.busy,|ui|{ui.horizontal_wrapped(|ui|{if ui.add_enabled(self.status.output.running,egui::Button::new("Use processed output by default")).clicked(){self.flush();self.submit(Request::MakeDefault{input:false});}
    if ui.add_enabled(self.status.input.running,egui::Button::new("Use processed input by default")).clicked(){self.flush();self.submit(Request::MakeDefault{input:true});}});
    if ui.button("Refresh devices").clicked(){self.submit(Request::Devices);}});
    ui.label(RichText::new("Defaults restore on stop only while still owned by ClearWave. Existing apps may need explicit selection or restart.").small().color(ui.visuals().widgets.noninteractive.fg_stroke.color));
    ui.label(format!("Microphone clipped samples: {}",self.status.input.clipped_samples));ui.label(RichText::new("Physical input clipping cannot be repaired. Lower hardware microphone volume in your system mixer; processed gain is separate.").small().color(ui.visuals().warn_fg_color));
    if !self.status.output_route.is_empty(){ui.label(format!("Output route: {}",self.status.output_route));}
    if !self.status.input_route.is_empty(){ui.label(format!("Input route: {}",self.status.input_route));}
    if ui.add_enabled(!self.busy,egui::Button::new("Quit / stop background engine")).clicked(){self.closed=true;self.flush();self.submit(Request::Shutdown);ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);}
  });
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.theme.poll();
        apply_theme(ctx, &self.theme);
        if self.theme.is_following() {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
        if self.theme.transition_progress() < 1.0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        // Developer-only: capture this application framebuffer, never the desktop.
        if let Some(directory) = std::env::var_os("CLEARWAVE_CAPTURE_UI") {
            ctx.request_repaint_after(Duration::from_millis(100));
            let events = ctx.input(|i| i.events.clone());
            for event in events {
                if let egui::Event::Screenshot { image, .. } = event {
                    let names = ["output", "input", "advanced"];
                    if self.capture_stage < 3 {
                        let directory = std::path::PathBuf::from(&directory);
                        let result = (|| -> anyhow::Result<()> {
                            std::fs::create_dir_all(&directory)?;
                            let pixels: Vec<u8> =
                                image.pixels.iter().flat_map(|p| p.to_array()).collect();
                            image::save_buffer_with_format(
                                directory.join(format!("{}.png", names[self.capture_stage])),
                                &pixels,
                                image.width() as u32,
                                image.height() as u32,
                                image::ColorType::Rgba8,
                                image::ImageFormat::Png,
                            )?;
                            Ok(())
                        })();
                        if let Err(e) = result {
                            self.error = Some(format!("UI capture failed: {e:#}"));
                        }
                        self.capture_stage += 1;
                        self.capture_requested = false;
                        self.page = match self.capture_stage {
                            1 => Page::Input,
                            _ => Page::Advanced,
                        };
                    }
                }
            }
            if self.capture_stage >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if self.ready
                && !self.busy
                && !self.capture_requested
                && !self.inventory.devices.is_empty()
                && self.dirty.is_none()
                && self.last_sent.elapsed() > Duration::from_millis(500)
            {
                self.capture_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let values = [
            self.status.input.input_peak,
            self.status.input.output_peak,
            self.status.output.input_peak,
            self.status.output.output_peak,
        ];
        for (m, v) in self.meters.iter_mut().zip(values) {
            *m = v.max(*m * (-dt / 0.22).exp());
        }
        if self.dirty.is_some()
            && self.last_sent.elapsed() > Duration::from_millis(80)
            && !self.busy
            && self.ready
        {
            self.flush();
        }
        if self.dirty.is_some()
            || self.status.input.running
            || self.status.output.running
            || self.meters.iter().any(|m| *m > 0.001)
        {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(32.0, 34.0), egui::Sense::hover());
                    for (i, height) in [10.0, 22.0, 30.0, 19.0, 9.0].iter().enumerate() {
                        let x = rect.left() + 3.0 + i as f32 * 6.0;
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, rect.center().y - height / 2.0),
                                egui::pos2(x, rect.center().y + height / 2.0),
                            ],
                            egui::Stroke::new(2.5_f32, ui.visuals().selection.stroke.color),
                        );
                    }
                    ui.label(RichText::new("ClearWave").size(25.0).strong());
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Theme");
                    let mut selected = self.theme.selected_id().to_owned();
                    let name = if selected == FOLLOW_OMARCHY {
                        "Follow Omarchy".to_owned()
                    } else {
                        self.theme
                            .entries()
                            .iter()
                            .find(|e| e.id == selected)
                            .map(|e| e.display_name.clone())
                            .unwrap_or_else(|| "ClearWave Classic".into())
                    };
                    egui::ComboBox::from_id_salt("theme-menu")
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected,
                                FOLLOW_OMARCHY.into(),
                                "Follow Omarchy",
                            );
                            for entry in self.theme.entries() {
                                ui.horizontal(|ui| {
                                    ui.colored_label(entry.palette.accent.to_color32(), "●");
                                    ui.selectable_value(
                                        &mut selected,
                                        entry.id.clone(),
                                        &entry.display_name,
                                    );
                                });
                            }
                        });
                    if selected != self.theme.selected_id() {
                        self.theme.select(&selected);
                        self.config.theme = Some(selected);
                        self.dirty = Some(Instant::now());
                        ui.ctx().request_repaint();
                    }
                    if self.theme.is_following() && !self.theme.omarchy_available() {
                        ui.label(
                            RichText::new("Omarchy unavailable; using fallback palette.").small(),
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (input, state) in [(false, &self.status.output), (true, &self.status.input)]
                    {
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                if input { "Input" } else { "Output" },
                                if state.error.is_some() {
                                    "needs attention"
                                } else if state.running {
                                    "live"
                                } else {
                                    "off"
                                }
                            ))
                            .small()
                            .color(if state.error.is_some() {
                                ui.visuals().warn_fg_color
                            } else if state.running {
                                ui.visuals().selection.stroke.color
                            } else {
                                ui.visuals().widgets.noninteractive.fg_stroke.color
                            }),
                        );
                    }
                    if self.busy {
                        ui.spinner();
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.page, Page::Output, "Output");
                    ui.selectable_value(&mut self.page, Page::Input, "Input");
                    ui.selectable_value(&mut self.page, Page::Advanced, "Advanced");
                });
                ui.add_space(12.0);
                if (!self.ready || self.error.is_some()) && ui.button("Retry connection").clicked()
                {
                    let _ = self.jobs.send(Job::Reconnect);
                    self.busy = true;
                }
                for message in [
                    self.error.as_ref(),
                    self.status.warning.as_ref(),
                    self.status.input.error.as_ref(),
                    self.status.output.error.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    ui.colored_label(ui.visuals().warn_fg_color, message);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(self.ready, |ui| match self.page {
                        Page::Output => self.channel_page(ui, false),
                        Page::Input => self.channel_page(ui, true),
                        Page::Advanced => self.advanced(ui),
                    });
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("Local processing · No uploads · No audio stored")
                            .small()
                            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
                    );
                });
            });
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if !self.closed {
            if self.dirty.is_some() {
                let _ = self.jobs.send(Job::Request(Request::Configure {
                    config: Box::new(self.config.clone()),
                }));
            }
            let _ = self.jobs.send(Job::Close(self.config.background));
        }
        // Complete the close handshake before the GUI process exits.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker(jobs: mpsc::Receiver<Job>, events: mpsc::Sender<Event>, ctx: egui::Context) {
    let send = |event| {
        let _ = events.send(event);
        ctx.request_repaint();
    };
    let initialize = || -> anyhow::Result<()> {
        engine::ensure_running()?;
        if let Some(s) = engine::request(&Request::Status)?.status {
            send(Event::Initial(s));
        }
        if let Some(i) = engine::request(&Request::Devices)?.inventory {
            send(Event::Inventory(i));
        }
        Ok(())
    };
    if let Err(e) = initialize() {
        send(Event::Error(format!("{e:#}")));
    }
    let mut active = false;
    let mut last_error = String::new();
    let mut previous = None;
    loop {
        let job = match jobs.recv_timeout(if active {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(1)
        }) {
            Ok(j) => Some(j),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => None,
        };
        if let Some(Job::Close(background)) = job {
            if !background {
                let _ = engine::request(&Request::Shutdown);
            }
            break;
        }
        if matches!(job, Some(Job::Reconnect)) {
            if let Err(e) = initialize() {
                send(Event::Error(format!("{e:#}")));
            }
            continue;
        }
        let is_request = job.is_some();
        let request = match job {
            Some(Job::Request(r)) => r,
            _ => Request::Status,
        };
        let shutdown = matches!(request, Request::Shutdown);
        match engine::request(&request) {
            Ok(r) => {
                last_error.clear();
                if let Some(i) = r.inventory {
                    send(Event::Inventory(i));
                }
                if let Some(s) = r.status {
                    active = s.input.running || s.output.running;
                    let changed = previous.as_ref() != Some(&s);
                    previous = Some(s.clone());
                    if is_request {
                        send(Event::Reply(s));
                    } else if changed {
                        send(Event::Tick(s));
                    }
                }
            }
            Err(e) => {
                let e = format!("{e:#}");
                if is_request || e != last_error {
                    send(Event::Error(e.clone()));
                    last_error = e;
                }
            }
        }
        if shutdown {
            break;
        }
    }
}
fn picker(
    ui: &mut egui::Ui,
    id: &str,
    inventory: &Inventory,
    kind: DeviceKind,
    selected: &mut String,
) {
    let name = inventory
        .devices
        .iter()
        .find(|d| d.id == *selected && d.kind == kind)
        .map(|d| d.name.as_str())
        .unwrap_or("Choose a device");
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().min(580.0))
        .selected_text(name)
        .show_ui(ui, |ui| {
            for d in inventory.devices.iter().filter(|d| d.kind == kind) {
                ui.selectable_value(selected, d.id.clone(), &d.name);
            }
        });
}
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) {
    ui.horizontal(|ui| {
        ui.add_sized([155.0, 22.0], egui::Label::new(label));
        ui.add(egui::Slider::new(value, range).suffix(suffix));
    });
}
fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .inner_margin(18)
        .corner_radius(12)
        .stroke(egui::Stroke::new(1.0_f32, ui.visuals().window_stroke.color))
}
fn meter(ui: &mut egui::Ui, label: &str, peak: f32) {
    let v = if peak.is_finite() && peak > 0.0 {
        ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ui.horizontal(|ui| {
        ui.add_sized(
            [110.0, 16.0],
            egui::Label::new(
                RichText::new(label)
                    .small()
                    .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
            ),
        );
        ui.add(
            egui::ProgressBar::new(v)
                .desired_width((ui.available_width() - 60.0).max(40.0))
                .desired_height(7.0)
                .fill(if peak >= 0.98 {
                    ui.visuals().warn_fg_color
                } else {
                    ui.visuals().selection.stroke.color
                }),
        );
        ui.label(
            RichText::new(if peak < 0.001 {
                "— dBFS".into()
            } else {
                format!("{:.0} dBFS", 20.0 * peak.log10())
            })
            .small()
            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
    });
}

fn button_text(ui: &egui::Ui) -> Color32 {
    let c = ui.visuals().selection.stroke.color;
    let rgb = clearwave::theme::Rgb::new(c.r(), c.g(), c.b());
    if rgb.luminance() > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}
fn apply_theme(ctx: &egui::Context, theme: &ThemeManager) {
    let mut v = if theme.palette().mode == Mode::Light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let color =
        |f: fn(&clearwave::theme::Palette) -> clearwave::theme::Rgb| theme.blend(f).to_color32();
    v.panel_fill = color(|p| p.background);
    v.window_fill = color(|p| p.surface);
    v.extreme_bg_color = color(|p| p.background);
    v.faint_bg_color = color(|p| p.surface);
    v.override_text_color = Some(color(|p| p.text));
    v.warn_fg_color = color(|p| p.warning);
    v.error_fg_color = color(|p| p.error);
    v.hyperlink_color = color(|p| p.accent);
    v.window_stroke.color = color(|p| p.border);
    v.selection.bg_fill = color(|p| p.selection);
    v.selection.stroke = egui::Stroke::new(1.5_f32, color(|p| p.accent));
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.bg_fill = color(|p| p.surface);
        widget.weak_bg_fill = color(|p| p.surface);
        widget.fg_stroke.color = color(|p| p.text);
        widget.bg_stroke.color = color(|p| p.border);
    }
    let surface = color(|p| p.surface);
    let accent = color(|p| p.accent);
    let control_fill = Color32::from_rgb(
        (surface.r() as f32 * 0.85 + accent.r() as f32 * 0.15) as u8,
        (surface.g() as f32 * 0.85 + accent.g() as f32 * 0.15) as u8,
        (surface.b() as f32 * 0.85 + accent.b() as f32 * 0.15) as u8,
    );
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.bg_fill = control_fill;
        widget.weak_bg_fill = control_fill;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, color(|p| p.secondary));
    }
    v.widgets.hovered.bg_stroke.color = color(|p| p.accent);
    v.widgets.active.bg_stroke.color = color(|p| p.accent);
    v.widgets.noninteractive.fg_stroke.color = color(|p| p.secondary);
    ctx.set_visuals(v);
}
