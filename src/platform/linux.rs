//! PulseAudio protocol backend, also supported by PipeWire's pipewire-pulse.
//!
//! Sessions never write system configuration. Defaults change only through an
//! explicit control action; make_default restores an owned default on stop.
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::{
    dsp::Processor,
    model::{Device, DeviceKind, Inventory, Settings, Telemetry},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const FRAME: usize = 480;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Run a bounded control command without a shell. Drain both pipes while waiting
/// so a large device inventory cannot fill the child's output pipe and deadlock.
fn run(program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("Cannot launch {program}; install the PulseAudio client utilities")
        })?;
    let mut stdout = child.stdout.take().context("Missing command output pipe")?;
    let mut stderr = child.stderr.take().context("Missing command error pipe")?;
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow!("{program} timed out; check your audio server"));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.into());
            }
        }
    };
    let stdout = output
        .join()
        .map_err(|_| anyhow!("Command output reader failed"))??;
    let stderr = errors
        .join()
        .map_err(|_| anyhow!("Command error reader failed"))??;
    let status = status?;
    if !status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(String::from_utf8(stdout)
        .context("Audio server returned invalid UTF-8")?
        .trim()
        .to_owned())
}

fn pactl(args: &[&str]) -> Result<String> {
    run("pactl", args)
}

fn json_command(args: &[&str]) -> Result<Value> {
    serde_json::from_str(&pactl(args)?)
        .context("Cannot read pactl JSON; a recent PulseAudio client is required")
}

fn is_capture_candidate(value: &Value) -> bool {
    let name = value["name"].as_str().unwrap_or("");
    let monitor = &value["monitor_of_sink"];
    let is_monitor = match monitor {
        Value::Number(number) => number
            .as_u64()
            .is_some_and(|index| index != u32::MAX as u64),
        Value::String(name) => !name.is_empty() && name != "n/a" && name != "4294967295",
        _ => false,
    };
    !name.is_empty()
        && !name.starts_with("clearwave_")
        && !name.ends_with(".monitor")
        && !is_monitor
}

fn volume_percent(value: &Value) -> Option<f32> {
    let channels = value["volume"].as_object()?;
    let values: Vec<f32> = channels
        .values()
        .filter_map(|channel| {
            channel["value_percent"]
                .as_str()
                .and_then(|value| value.trim_end_matches('%').parse::<f32>().ok())
                .or_else(|| {
                    channel["value"]
                        .as_f64()
                        .map(|value| (value / 65536.0 * 100.0) as f32)
                })
        })
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f32>() / values.len() as f32 / 100.0)
    }
}

fn parse_devices(value: &Value, kind: DeviceKind, default: &str) -> Result<Vec<Device>> {
    let values = value
        .as_array()
        .context("Audio server device list is not an array")?;
    Ok(values
        .iter()
        .filter(|value| {
            let name = value["name"].as_str().unwrap_or("");
            match kind {
                DeviceKind::Input => is_capture_candidate(value),
                DeviceKind::Output => !name.is_empty() && !name.starts_with("clearwave_"),
            }
        })
        .map(|value| {
            let id = value["name"].as_str().unwrap_or_default().to_owned();
            Device {
                name: value["description"]
                    .as_str()
                    .or_else(|| value["properties"]["device.description"].as_str())
                    .unwrap_or(&id)
                    .to_owned(),
                is_default: id == default,
                volume: volume_percent(value),
                muted: value["mute"].as_bool(),
                id,
                kind,
            }
        })
        .collect())
}

pub fn enumerate() -> Result<Inventory> {
    let info = json_command(&["--format=json", "info"])?;
    let sources = json_command(&["--format=json", "list", "sources"])?;
    let sinks = json_command(&["--format=json", "list", "sinks"])?;
    let mut devices = parse_devices(
        &sources,
        DeviceKind::Input,
        info["default_source_name"].as_str().unwrap_or(""),
    )?;
    devices.extend(parse_devices(
        &sinks,
        DeviceKind::Output,
        info["default_sink_name"].as_str().unwrap_or(""),
    )?);
    let server = info["server_name"]
        .as_str()
        .unwrap_or("PulseAudio protocol server");
    Ok(Inventory {
        backend: server.to_owned(),
        devices,
        notes: vec![
            "Supports devices exposed by PipeWire-Pulse or PulseAudio. Direct ALSA-only and JACK-only sessions need a PulseAudio-compatible server.".into(),
            "Sink monitors and Clearwave's own virtual devices are excluded from microphone selection to prevent feedback.".into(),
        ],
    })
}

pub fn doctor() -> Value {
    let tools: Vec<Value> = ["pactl", "parec", "pacat"]
        .into_iter()
        .map(|program| match run(program, &["--version"]) {
            Ok(version) => json!({"name":program, "available":true, "version":version}),
            Err(error) => json!({"name":program, "available":false, "error":error.to_string()}),
        })
        .collect();
    let tools_ok = tools.iter().all(|tool| tool["available"] == true);
    match enumerate() {
        Ok(inventory) => json!({
            "ok": tools_ok && inventory.devices.iter().any(|device| matches!(device.kind, DeviceKind::Input)),
            "platform":"linux", "tools":tools, "inventory":inventory,
            "hint":"Use a PipeWire-Pulse or PulseAudio desktop session. ALSA-only systems require an audio server. No audio is recorded by this check."
        }),
        Err(error) => json!({
            "ok":false, "platform":"linux", "tools":tools, "error":error.to_string(),
            "hint":"Install PulseAudio client utilities (Arch: libpulse; Debian/Ubuntu: pulseaudio-utils), and run in your normal desktop user's PipeWire-Pulse or PulseAudio session."
        }),
    }
}

fn validate_device(id: &str, kind: DeviceKind) -> Result<()> {
    if enumerate()?
        .devices
        .iter()
        .any(|device| device.id == id && device.kind == kind)
    {
        Ok(())
    } else {
        bail!("Device is unavailable or is a feedback-prone virtual device: {id}")
    }
}

pub fn set_volume(id: &str, kind: DeviceKind, percent: u16) -> Result<()> {
    if percent > 150 {
        bail!("System volume must be between 0 and 150 percent");
    }
    validate_device(id, kind)?;
    let command = match kind {
        DeviceKind::Input => "set-source-volume",
        DeviceKind::Output => "set-sink-volume",
    };
    pactl(&[command, id, &format!("{percent}%")])?;
    Ok(())
}

pub fn set_default(id: &str, kind: DeviceKind) -> Result<()> {
    validate_device(id, kind)?;
    let command = match kind {
        DeviceKind::Input => "set-default-source",
        DeviceKind::Output => "set-default-sink",
    };
    pactl(&[command, id])?;
    Ok(())
}

/// Play a short, faded, low-level tone to the explicitly selected output.
pub fn test_output(id: &str) -> Result<()> {
    validate_device(id, DeviceKind::Output)?;
    let mut child = audio_child("pacat", id, false)?;
    let mut pipe = child.stdin.take().context("Missing speaker test pipe")?;
    let writer = thread::spawn(move || -> std::io::Result<()> {
        let count = 19_200;
        let mut bytes = Vec::with_capacity(count * 4);
        for i in 0..count {
            let fade = ((i.min(count - 1 - i) as f32) / 480.0).min(1.0);
            let sample = (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.08 * fade;
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        pipe.write_all(&bytes)
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow!("Speaker test timed out; check the output device"));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.into());
            }
        }
    };
    let write_result = writer
        .join()
        .map_err(|_| anyhow!("Speaker test writer failed"))?;
    if !status?.success() {
        bail!("Speaker test failed; check the output device and audio server");
    }
    write_result.context("Speaker test stream ended")?;
    Ok(())
}

#[derive(Default)]
struct Resources {
    children: Vec<Child>,
    modules: Vec<u32>,
    default_source: Option<(String, String)>,
    default_sink: Option<(String, String)>,
}

impl Resources {
    fn load_module(&mut self, args: &[&str]) -> Result<()> {
        let id: u32 = pactl(args)?
            .parse()
            .context("Audio server returned an invalid module ID")?;
        self.modules.push(id);
        Ok(())
    }

    fn cleanup(&mut self) {
        // Kill all processes first: the worker may be blocked on either audio pipe.
        for child in &mut self.children {
            let _ = child.kill();
        }
        for mut child in self.children.drain(..) {
            let _ = child.wait();
        }
        if let Some((owned, previous)) = self.default_source.take() {
            if pactl(&["get-default-source"]).is_ok_and(|current| current == owned) {
                if let Err(error) = pactl(&["set-default-source", &previous]) {
                    eprintln!(
                        "{}",
                        json!({"event":"cleanup_warning", "error":format!("Could not restore previous microphone: {error}")})
                    );
                }
            }
        }
        if let Some((owned, previous)) = self.default_sink.take() {
            if pactl(&["get-default-sink"]).is_ok_and(|current| current == owned) {
                if let Err(error) = pactl(&["set-default-sink", &previous]) {
                    eprintln!("Could not restore output: {error}");
                }
            }
        }
        for module in self.modules.drain(..).rev() {
            if let Err(error) = pactl(&["unload-module", &module.to_string()]) {
                eprintln!(
                    "{}",
                    json!({"event":"cleanup_warning", "module":module, "error":error.to_string()})
                );
            }
        }
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn audio_child(program: &str, device: &str, recording: bool) -> Result<Child> {
    audio_child_channels(program, device, recording, false)
}
fn audio_child_channels(
    program: &str,
    device: &str,
    recording: bool,
    stereo: bool,
) -> Result<Child> {
    Command::new(program)
        .args([
            "--raw",
            "--format=float32le",
            "--rate=48000",
            if stereo {
                "--channels=2"
            } else {
                "--channels=1"
            },
            if stereo {
                "--channel-map=front-left,front-right"
            } else {
                "--channel-map=mono"
            },
            "--latency-msec=30",
            "--process-time-msec=10",
            "--client-name=Clearwave",
            "--stream-name=Clearwave audio",
            "--device",
            device,
        ])
        .stdin(if recording {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(if recording {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        // Stream failures are reported by the worker as structured application errors.
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Cannot launch {program}; install PulseAudio client utilities"))
}

pub struct Session {
    resources: Arc<Mutex<Resources>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
    telemetry: Arc<Mutex<Telemetry>>,
    virtual_source: String,
    controls: Arc<Mutex<crate::settings::Processing>>,
}

impl Session {
    pub fn start(settings: Settings, telemetry: Arc<Mutex<Telemetry>>) -> Result<Self> {
        let processing = crate::settings::Processing {
            denoise: settings.denoise,
            gain_db: settings.boost_db,
            ..Default::default()
        };
        Self::start_processed(settings, processing, telemetry)
    }
    pub fn start_processed(
        settings: Settings,
        processing: crate::settings::Processing,
        telemetry: Arc<Mutex<Telemetry>>,
    ) -> Result<Self> {
        settings.validate()?;
        processing.validate()?;
        let controls = Arc::new(Mutex::new(processing));
        let worker_controls = controls.clone();
        validate_device(&settings.input, DeviceKind::Input)?;
        if settings.monitor {
            validate_device(
                settings
                    .output
                    .as_deref()
                    .context("Choose speakers or headphones before enabling monitoring")?,
                DeviceKind::Output,
            )?;
        }
        let sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = format!("clearwave_{}_{}", std::process::id(), sequence);
        let sink = format!("{base}_sink");
        let source = format!("{base}_mic");
        let mut resources = Resources::default();
        resources.load_module(&[
            "load-module",
            "module-null-sink",
            &format!("sink_name={sink}"),
            "rate=48000",
            "channels=1",
            "channel_map=mono",
            "sink_properties=device.description='Clearwave Processing'",
        ])?;
        resources.load_module(&[
            "load-module",
            "module-remap-source",
            &format!("master={sink}.monitor"),
            &format!("source_name={source}"),
            "rate=48000",
            "channels=1",
            "channel_map=mono",
            "source_properties=device.description='Clearwave Microphone'",
        ])?;
        let mut capture = audio_child("parec", &settings.input, true)?;
        let capture_pid = capture.id();
        let capture_pipe = capture.stdout.take().context("Missing capture pipe")?;
        resources.children.push(capture);
        let mut playback = audio_child("pacat", &sink, false)?;
        let playback_pipe = playback.stdin.take().context("Missing playback pipe")?;
        resources.children.push(playback);
        // Monitoring uses the server's loopback rather than a second blocking
        // playback pipe, so unplugging headphones cannot block the clean mic.
        if settings.monitor {
            let output = settings
                .output
                .as_deref()
                .context("Missing monitor output")?;
            // Device names come from validated server inventory. Quote and escape
            // values for the module's own argument parser, never a shell.
            resources.load_module(&[
                "load-module",
                "module-loopback",
                &format!("source={source}"),
                &format!("sink={}", quote_module_value(output)),
                "latency_msec=30",
                "source_dont_move=true",
                "sink_dont_move=true",
            ])?;
        }
        let resources = Arc::new(Mutex::new(resources));
        let stop = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let worker_progress = Arc::clone(&progress);
        let selected_source = settings.input.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_resources = Arc::clone(&resources);
        let worker_stop = Arc::clone(&stop);
        let worker_telemetry = Arc::clone(&telemetry);
        if let Ok(mut status) = telemetry.lock() {
            status.error = None;
            status.running = false;
            status.input_peak = 0.0;
            status.output_peak = 0.0;
            status.clipped_samples = 0;
        }
        let worker = thread::Builder::new()
            .name("clearwave-audio".into())
            .spawn(move || {
                let result = process_audio(
                    capture_pipe,
                    playback_pipe,
                    &worker_controls,
                    &worker_stop,
                    &worker_telemetry,
                    &worker_progress,
                    ready_tx,
                );
                if let Err(error) = result {
                    if !worker_stop.load(Ordering::Acquire) {
                        if let Ok(mut status) = worker_telemetry.lock() {
                            status.error = Some(format!("Audio stopped: {error:#}"));
                        }
                    }
                }
                worker_stop.store(true, Ordering::Release);
                if let Ok(mut status) = worker_telemetry.lock() {
                    status.running = false;
                    status.input_peak = 0.0;
                    status.output_peak = 0.0;
                }
                if let Ok(mut resources) = worker_resources.lock() {
                    resources.cleanup();
                }
            })
            .context("Cannot create audio processing thread")?;
        let mut session = Self {
            resources,
            stop,
            worker: Some(worker),
            watchdog: None,
            telemetry,
            virtual_source: source,
            controls,
        };
        if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
            session.stop();
            let detail = session
                .telemetry
                .lock()
                .ok()
                .and_then(|status| status.error.clone());
            bail!(
                "{}",
                detail.unwrap_or_else(|| format!(
                    "Audio did not start: {error}; check the selected device and audio server"
                ))
            );
        }
        let watchdog_resources = Arc::clone(&session.resources);
        let watchdog_stop = Arc::clone(&session.stop);
        let watchdog_telemetry = Arc::clone(&session.telemetry);
        session.watchdog = Some(
            thread::Builder::new()
                .name("clearwave-route-watch".into())
                .spawn(move || {
                    watch_capture(
                        selected_source,
                        capture_pid,
                        progress,
                        watchdog_stop,
                        watchdog_telemetry,
                        watchdog_resources,
                    );
                })
                .context("Cannot create audio routing watchdog")?,
        );
        Ok(session)
    }

    pub fn update(&self, processing: crate::settings::Processing) -> Result<()> {
        processing.validate()?;
        *self
            .controls
            .lock()
            .map_err(|_| anyhow!("DSP controls unavailable"))? = processing;
        Ok(())
    }
    pub fn virtual_source(&self) -> &str {
        &self.virtual_source
    }

    /// The default is restored on stop only if it still points to this session.
    /// Explicit changes by the user or another program are left intact.
    pub fn make_default(&mut self) -> Result<()> {
        let mut resources = self
            .resources
            .lock()
            .map_err(|_| anyhow!("Audio resources unavailable"))?;
        if resources.modules.is_empty() {
            bail!("The audio session has already stopped");
        }
        if resources.default_source.is_none() {
            let previous = pactl(&["get-default-source"])?;
            pactl(&["set-default-source", &self.virtual_source])?;
            resources.default_source = Some((self.virtual_source.clone(), previous));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut resources) = self.resources.lock() {
            resources.cleanup();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        if let Ok(mut status) = self.telemetry.lock() {
            status.running = false;
            status.input_peak = 0.0;
            status.output_peak = 0.0;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn numeric_id(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// Match the live stream, not just the device label selected in the interface.
/// PulseAudio may otherwise rescue a disconnected capture stream onto another mic.
fn verify_capture_route(
    sources: &Value,
    streams: &Value,
    selected: &str,
    capture_pid: u32,
) -> Result<()> {
    let sources = sources
        .as_array()
        .context("Invalid source inventory from audio server")?;
    let source = sources
        .iter()
        .find(|source| source["name"].as_str() == Some(selected))
        .with_context(|| format!("Selected microphone disconnected: {selected}"))?;
    let expected_index =
        numeric_id(&source["index"]).context("Selected microphone has no source index")?;
    let streams = streams
        .as_array()
        .context("Invalid capture stream inventory from audio server")?;
    let mut matching = streams.iter().filter(|stream| {
        numeric_id(&stream["properties"]["application.process.id"]) == Some(capture_pid as u64)
    });
    let stream = matching
        .next()
        .context("Microphone capture stream disappeared")?;
    if matching.next().is_some() {
        bail!("Cannot unambiguously inspect the microphone capture route");
    }
    let actual_index =
        numeric_id(&stream["source"]).context("Capture stream has no source index")?;
    if actual_index != expected_index {
        bail!("The audio server moved capture away from the selected microphone; choose an input and restart");
    }
    Ok(())
}

fn watch_capture(
    selected: String,
    capture_pid: u32,
    progress: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    telemetry: Arc<Mutex<Telemetry>>,
    resources: Arc<Mutex<Resources>>,
) {
    let mut last_progress = progress.load(Ordering::Acquire);
    let mut last_frame = Instant::now();
    while !stop.load(Ordering::Acquire) {
        // Small sleeps make ordinary shutdown prompt; each server command also
        // has a fixed timeout and runs without holding the resources mutex.
        for _ in 0..10 {
            if stop.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let result = (|| -> Result<()> {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let sources = json_command(&["--format=json", "list", "sources"])?;
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let streams = json_command(&["--format=json", "list", "source-outputs"])?;
            verify_capture_route(&sources, &streams, &selected, capture_pid)?;
            let current = progress.load(Ordering::Acquire);
            if current != last_progress {
                last_progress = current;
                last_frame = Instant::now();
            } else if last_frame.elapsed() >= Duration::from_secs(3) {
                bail!("The microphone stopped delivering audio for three seconds; reconnect it and restart");
            }
            Ok(())
        })();
        if stop.load(Ordering::Acquire) {
            return;
        }
        if let Err(error) = result {
            // Set the shared stop flag before killing children so their broken
            // pipes cannot overwrite the more useful watchdog diagnosis.
            stop.store(true, Ordering::Release);
            if let Ok(mut status) = telemetry.lock() {
                status.error = Some(format!("Audio stopped: {error:#}"));
                status.running = false;
            }
            if let Ok(mut resources) = resources.lock() {
                for child in &mut resources.children {
                    let _ = child.kill();
                }
            }
            return;
        }
    }
}

fn quote_module_value(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn process_audio(
    mut capture: impl Read,
    mut playback: impl Write,
    controls: &Mutex<crate::settings::Processing>,
    stop: &AtomicBool,
    telemetry: &Mutex<Telemetry>,
    progress: &AtomicU64,
    ready: mpsc::SyncSender<()>,
) -> Result<()> {
    let mut processor = Processor::new();
    let mut settings = *controls
        .lock()
        .map_err(|_| anyhow!("DSP controls unavailable"))?;
    let mut enhance = crate::enhance::Enhancer::new(settings);
    let mut bytes = [0_u8; FRAME * 4];
    let mut input = [0.0_f32; FRAME];
    let mut output = [0.0_f32; FRAME];
    let mut started = false;
    while !stop.load(Ordering::Acquire) {
        capture
            .read_exact(&mut bytes)
            .context("Microphone disconnected or capture stream ended")?;
        for (sample, bytes) in input.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
            *sample = f32::from_le_bytes(*bytes);
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        let input_peak = input
            .iter()
            .fold(0.0_f32, |peak, value| peak.max(value.abs()));
        let clipped = input.iter().filter(|value| value.abs() >= 0.999).count() as u64;
        if let Ok(next) = controls.try_lock() {
            settings = *next;
        }
        enhance.update(settings);
        processor.process(&input, &mut output, settings.denoise, 0.0);
        for sample in &mut output {
            *sample = enhance.frame([*sample, *sample])[0];
        }
        let output_peak = output
            .iter()
            .fold(0.0_f32, |peak, value| peak.max(value.abs()));
        for (sample, bytes) in output.iter().zip(bytes.as_chunks_mut::<4>().0.iter_mut()) {
            bytes.copy_from_slice(&sample.to_le_bytes());
        }
        playback
            .write_all(&bytes)
            .context("Clean microphone playback stream ended")?;
        progress.fetch_add(1, Ordering::Release);
        if let Ok(mut status) = telemetry.lock() {
            status.running = true;
            status.input_peak = input_peak;
            status.output_peak = output_peak;
            status.clipped_samples += clipped;
        }
        if !started {
            let _ = ready.send(());
            started = true;
        }
    }
    Ok(())
}

/// Independent stereo playback session. Does not move existing application streams.
pub struct OutputSession {
    resources: Arc<Mutex<Resources>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
    controls: Arc<Mutex<crate::settings::Processing>>,
    telemetry: Arc<Mutex<Telemetry>>,
    sink: String,
}
impl OutputSession {
    pub fn start(
        destination: &str,
        processing: crate::settings::Processing,
        telemetry: Arc<Mutex<Telemetry>>,
    ) -> Result<Self> {
        processing.validate()?;
        validate_device(destination, DeviceKind::Output)?;
        let sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let sink = format!("clearwave_{}_{}_output", std::process::id(), sequence);
        let mut resources = Resources::default();
        resources.load_module(&[
            "load-module",
            "module-null-sink",
            &format!("sink_name={sink}"),
            "rate=48000",
            "channels=2",
            "channel_map=front-left,front-right",
            "sink_properties=device.description='ClearWave Output'",
        ])?;
        let mut capture = audio_child_channels("parec", &format!("{sink}.monitor"), true, true)?;
        let capture_pipe = capture
            .stdout
            .take()
            .context("Missing output capture pipe")?;
        resources.children.push(capture);
        let mut playback = audio_child_channels("pacat", destination, false, true)?;
        let playback_pipe = playback
            .stdin
            .take()
            .context("Missing output playback pipe")?;
        let playback_pid = playback.id();
        resources.children.push(playback);
        let resources = Arc::new(Mutex::new(resources));
        let stop = Arc::new(AtomicBool::new(false));
        let controls = Arc::new(Mutex::new(processing));
        *telemetry
            .lock()
            .map_err(|_| anyhow!("Audio status unavailable"))? = Telemetry {
            running: false,
            ..Default::default()
        };
        let (r, s, c, t) = (
            resources.clone(),
            stop.clone(),
            controls.clone(),
            telemetry.clone(),
        );
        let (ready, received) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("clearwave-output".into())
            .spawn(move || {
                let result = process_stereo(capture_pipe, playback_pipe, &c, &s, &t, Some(ready));
                if let Ok(mut status) = t.lock() {
                    status.running = false;
                    if !s.load(Ordering::Acquire) {
                        status.error = Some(format!(
                            "Output stopped: {}",
                            result.err().map(|e| format!("{e:#}")).unwrap_or_default()
                        ));
                    }
                }
                s.store(true, Ordering::Release);
                if let Ok(mut r) = r.lock() {
                    r.cleanup();
                }
            })?;
        let mut session = Self {
            resources,
            stop,
            worker: Some(worker),
            watchdog: None,
            controls,
            telemetry,
            sink,
        };
        if let Err(error) = received.recv_timeout(Duration::from_secs(5)) {
            session.stop();
            bail!("Output did not start: {error}; check audio server and destination");
        }
        let sinks = json_command(&["--format=json", "list", "sinks"])?;
        let streams = json_command(&["--format=json", "list", "sink-inputs"])?;
        verify_playback_route(&sinks, &streams, destination, playback_pid)?;
        let (r, s, t) = (
            session.resources.clone(),
            session.stop.clone(),
            session.telemetry.clone(),
        );
        let destination = destination.to_owned();
        session.watchdog = Some(
            thread::Builder::new()
                .name("clearwave-output-route".into())
                .spawn(move || {
                    while !s.load(Ordering::Acquire) {
                        for _ in 0..10 {
                            if s.load(Ordering::Acquire) {
                                return;
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                        let result = (|| -> Result<()> {
                            let sinks = json_command(&["--format=json", "list", "sinks"])?;
                            let streams = json_command(&["--format=json", "list", "sink-inputs"])?;
                            verify_playback_route(&sinks, &streams, &destination, playback_pid)
                        })();
                        if let Err(error) = result {
                            if s.load(Ordering::Acquire) {
                                return;
                            }
                            s.store(true, Ordering::Release);
                            if let Ok(mut status) = t.lock() {
                                status.running = false;
                                status.error = Some(format!("Output stopped: {error:#}"));
                            }
                            if let Ok(mut r) = r.lock() {
                                for child in &mut r.children {
                                    let _ = child.kill();
                                }
                            }
                            return;
                        }
                    }
                })?,
        );
        Ok(session)
    }
    pub fn virtual_sink(&self) -> &str {
        &self.sink
    }
    pub fn update(&self, processing: crate::settings::Processing) -> Result<()> {
        processing.validate()?;
        *self
            .controls
            .lock()
            .map_err(|_| anyhow!("DSP controls unavailable"))? = processing;
        Ok(())
    }
    pub fn make_default(&mut self) -> Result<()> {
        let mut r = self
            .resources
            .lock()
            .map_err(|_| anyhow!("Output resources unavailable"))?;
        if r.modules.is_empty() {
            bail!("Output is stopped");
        }
        if r.default_sink.is_none() {
            let previous = pactl(&["get-default-sink"])?;
            pactl(&["set-default-sink", &self.sink])?;
            r.default_sink = Some((self.sink.clone(), previous));
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut r) = self.resources.lock() {
            r.cleanup();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(w) = self.watchdog.take() {
            let _ = w.join();
        }
        if let Ok(mut t) = self.telemetry.lock() {
            t.running = false;
            t.input_peak = 0.0;
            t.output_peak = 0.0;
        }
    }
}
impl Drop for OutputSession {
    fn drop(&mut self) {
        self.stop();
    }
}
fn verify_playback_route(sinks: &Value, streams: &Value, selected: &str, pid: u32) -> Result<()> {
    let sink = sinks
        .as_array()
        .context("Invalid sink inventory")?
        .iter()
        .find(|v| v["name"].as_str() == Some(selected))
        .context("Selected output disconnected")?;
    let index = numeric_id(&sink["index"]).context("Missing output index")?;
    let mut streams = streams
        .as_array()
        .context("Invalid playback inventory")?
        .iter()
        .filter(|v| numeric_id(&v["properties"]["application.process.id"]) == Some(pid as u64));
    let stream = streams.next().context("Playback stream disappeared")?;
    anyhow::ensure!(
        streams.next().is_none() && numeric_id(&stream["sink"]) == Some(index),
        "Playback moved away from selected output; restart after choosing a destination"
    );
    Ok(())
}
fn process_stereo(
    mut capture: impl Read,
    mut playback: impl Write,
    controls: &Mutex<crate::settings::Processing>,
    stop: &AtomicBool,
    telemetry: &Mutex<Telemetry>,
    mut ready: Option<mpsc::SyncSender<()>>,
) -> Result<()> {
    let mut settings = *controls
        .lock()
        .map_err(|_| anyhow!("DSP controls unavailable"))?;
    let mut dsp = crate::enhance::Enhancer::new(settings);
    let mut bytes = [0_u8; 480 * 8];
    while !stop.load(Ordering::Acquire) {
        capture
            .read_exact(&mut bytes)
            .context("Output capture stream ended")?;
        if let Ok(p) = controls.try_lock() {
            settings = *p;
        }
        dsp.update(settings);
        let (mut input_peak, mut output_peak) = (0.0_f32, 0.0_f32);
        let mut clipped = 0;
        for frame in bytes.as_chunks_mut::<8>().0.iter_mut() {
            let l = f32::from_le_bytes(frame[..4].try_into()?);
            let r = f32::from_le_bytes(frame[4..].try_into()?);
            for v in [l, r] {
                if v.is_finite() {
                    input_peak = input_peak.max(v.abs());
                    if v.abs() >= 0.999 {
                        clipped += 1;
                    }
                }
            }
            let y = dsp.frame([l, r]);
            output_peak = output_peak.max(y[0].abs()).max(y[1].abs());
            frame[..4].copy_from_slice(&y[0].to_le_bytes());
            frame[4..].copy_from_slice(&y[1].to_le_bytes());
        }
        playback
            .write_all(&bytes)
            .context("Physical output stream ended")?;
        if let Ok(mut t) = telemetry.try_lock() {
            t.running = true;
            t.input_peak = input_peak;
            t.output_peak = output_peak;
            t.clipped_samples += clipped;
        }
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_inventory_excludes_monitors_and_own_devices() {
        let raw = json!([
            {"name":"alsa_input.usb", "description":"USB microphone", "monitor_of_sink":null, "mute":false,
             "volume":{"mono":{"value_percent":"75%"}}},
            {"name":"alsa_output.usb.monitor", "monitor_of_sink":7},
            {"name":"renamed_monitor", "monitor_of_sink":0},
            {"name":"clearwave_100_0_mic", "monitor_of_sink":null},
            {"name":"bluetooth_input", "monitor_of_sink":4294967295_u32}
        ]);
        let devices = parse_devices(&raw, DeviceKind::Input, "alsa_input.usb").unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "USB microphone");
        assert!(devices[0].is_default);
        assert_eq!(devices[0].volume, Some(0.75));
        assert_eq!(devices[0].muted, Some(false));
    }

    #[test]
    fn output_inventory_keeps_speakers_and_averages_channels() {
        let raw = json!([
            {"name":"speakers", "volume":{"left":{"value":65536},"right":{"value":32768}}},
            {"name":"clearwave_2_0_sink"}
        ]);
        let devices = parse_devices(&raw, DeviceKind::Output, "speakers").unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].volume, Some(0.75));
        assert_eq!(devices[0].name, "speakers");
    }

    #[test]
    fn malformed_inventory_is_an_error() {
        assert!(parse_devices(&json!({}), DeviceKind::Input, "").is_err());
    }

    #[test]
    fn audio_pipeline_reports_clipping_and_sanitizes_nonfinite_input() {
        let mut samples = [0.05_f32; FRAME];
        samples[0] = 1.0;
        samples[1] = f32::NAN;
        samples[2] = f32::INFINITY;
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut output = Vec::new();
        let settings = Settings {
            input: "test".into(),
            output: None,
            denoise: false,
            boost_db: 0.0,
            monitor: false,
        };
        let telemetry = Mutex::new(Telemetry::default());
        let (ready, received) = mpsc::sync_channel(1);
        let result = process_audio(
            std::io::Cursor::new(bytes),
            &mut output,
            &Mutex::new(crate::settings::Processing {
                denoise: settings.denoise,
                ..Default::default()
            }),
            &AtomicBool::new(false),
            &telemetry,
            &AtomicU64::new(0),
            ready,
        );
        // EOF is reported, never retried in a busy loop; the complete frame was delivered.
        assert!(result.is_err());
        assert_eq!(output.len(), FRAME * 4);
        assert!(received.try_recv().is_ok());
        assert!(output.as_chunks::<4>().0.iter().all(|bytes| {
            let value = f32::from_le_bytes(*bytes);
            value.is_finite() && value.abs() <= 1.0
        }));
        let status = telemetry.lock().unwrap();
        assert_eq!(status.clipped_samples, 1);
        assert_eq!(status.input_peak, 1.0);
    }

    #[test]
    fn route_inspection_matches_the_capture_process_and_actual_source() {
        let sources = json!([{ "name":"usb_mic", "index":14 }, { "name":"laptop_mic", "index":7 }]);
        let streams = json!([
            { "source":7, "properties":{ "application.process.id":"123" } },
            { "source":14, "properties":{ "application.process.id":"456" } }
        ]);
        assert!(verify_capture_route(&sources, &streams, "usb_mic", 456).is_ok());
        assert!(verify_capture_route(&sources, &streams, "usb_mic", 123).is_err());
        assert!(verify_capture_route(&sources, &streams, "usb_mic", 999).is_err());
        assert!(verify_capture_route(&sources, &streams, "unplugged_mic", 456).is_err());
    }

    #[test]
    fn route_inspection_handles_numeric_pids_and_rejects_ambiguous_streams() {
        let sources = json!([{ "name":"mic", "index":"14" }]);
        let stream = json!({ "source":"14", "properties":{ "application.process.id":456 } });
        assert!(verify_capture_route(&sources, &json!([stream.clone()]), "mic", 456).is_ok());
        assert!(
            verify_capture_route(&sources, &json!([stream.clone(), stream]), "mic", 456).is_err()
        );
        assert!(verify_capture_route(&json!({}), &json!([]), "mic", 456).is_err());
    }

    #[test]
    fn module_values_are_escaped() {
        assert_eq!(
            quote_module_value("device with \"quote\""),
            "\"device with \\\"quote\\\"\""
        );
    }

    #[test]
    fn invalid_volume_is_rejected_before_contacting_server() {
        assert!(set_volume("unused", DeviceKind::Output, 151).is_err());
    }
}
