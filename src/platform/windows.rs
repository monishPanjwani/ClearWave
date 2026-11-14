//! Experimental WASAPI backend. A separately installed virtual cable provides the microphone endpoint.
use crate::dsp::{Processor, FRAME_SIZE, SAMPLE_RATE};
use crate::model::{Device, DeviceKind, Inventory, Settings, Telemetry};
use anyhow::{bail, ensure, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_queue::ArrayQueue;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

fn device_id(kind: DeviceKind, name: &str) -> String {
    format!(
        "{}:{name}",
        if kind == DeviceKind::Input {
            "input"
        } else {
            "output"
        }
    )
}

pub fn enumerate() -> Result<Inventory> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    for kind in [DeviceKind::Input, DeviceKind::Output] {
        let default = match kind {
            DeviceKind::Input => host.default_input_device(),
            DeviceKind::Output => host.default_output_device(),
        }
        .and_then(|d| d.name().ok());
        let endpoints: Vec<_> = match kind {
            DeviceKind::Input => host.input_devices()?.collect(),
            DeviceKind::Output => host.output_devices()?.collect(),
        };
        for endpoint in endpoints {
            let name = endpoint
                .name()
                .context("Could not read the audio endpoint name")?;
            devices.push(Device {
                id: device_id(kind, &name),
                is_default: default.as_ref() == Some(&name),
                name,
                kind,
                volume: None,
                muted: None,
            });
        }
    }
    Ok(Inventory {
        backend: "Windows WASAPI (experimental)".into(), devices,
        notes: vec![
            "Select an installed virtual cable's playback endpoint as output, then select its recording endpoint in your calling app.".into(),
            "Windows system volume and default devices are managed in Windows Sound Settings. Clearwave's boost affects only its processed microphone.".into(),
            "Equal-name endpoints must be renamed in Windows Sound Settings before selecting them. This preview uses linear sample-rate conversion.".into(),
        ],
    })
}

pub fn doctor() -> Value {
    match enumerate() {
        Ok(inventory) => json!({"ok": true, "platform": "windows", "inventory": inventory,
            "virtual_device": "An existing virtual audio cable is required; no driver is installed by Clearwave."}),
        Err(error) => json!({"ok": false, "platform": "windows", "error": error.to_string()}),
    }
}

pub fn set_volume(_id: &str, _kind: DeviceKind, _percent: u16) -> Result<()> {
    bail!("Change device volume in Windows Sound Settings. Microphone boost is available inside Clearwave.")
}

pub fn set_default(_id: &str, _kind: DeviceKind) -> Result<()> {
    bail!("Choose the default device in Windows Sound Settings.")
}

fn find_device(id: &str, kind: DeviceKind) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let endpoints: Vec<_> = match kind {
        DeviceKind::Input => host.input_devices()?.collect(),
        DeviceKind::Output => host.output_devices()?.collect(),
    };
    let mut matches = endpoints
        .into_iter()
        .filter(|d| d.name().is_ok_and(|name| device_id(kind, &name) == id));
    let device = matches
        .next()
        .context("The selected audio device is disconnected; refresh devices.")?;
    ensure!(
        matches.next().is_none(),
        "More than one endpoint has this name. Rename duplicate devices in Windows Sound Settings."
    );
    Ok(device)
}

#[derive(Default)]
struct Counters {
    overflow: AtomicU64,
    underrun: AtomicU64,
}

fn fail(telemetry: &Mutex<Telemetry>, running: &AtomicBool, message: String) {
    running.store(false, Ordering::Relaxed);
    if let Ok(mut state) = telemetry.lock() {
        state.running = false;
        state.error = Some(message);
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<ArrayQueue<f32>>,
    counters: Arc<Counters>,
    running: Arc<AtomicBool>,
    telemetry: Arc<Mutex<Telemetry>>,
) -> Result<Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            for frame in data.chunks_exact(channels) {
                let mono =
                    frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32;
                if queue
                    .push(if mono.is_finite() { mono } else { 0.0 })
                    .is_err()
                {
                    counters.overflow.fetch_add(1, Ordering::Relaxed);
                }
            }
        },
        move |error| {
            fail(
                &telemetry,
                &running,
                format!("Microphone stream failed: {error}"),
            )
        },
        None,
    )?)
}

fn output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<ArrayQueue<f32>>,
    counters: Arc<Counters>,
    running: Arc<AtomicBool>,
    telemetry: Arc<Mutex<Telemetry>>,
) -> Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            for frame in data.chunks_exact_mut(channels) {
                let sample = queue.pop().unwrap_or_else(|| {
                    counters.underrun.fetch_add(1, Ordering::Relaxed);
                    0.0
                });
                frame.fill(T::from_sample(sample));
            }
        },
        move |error| {
            fail(
                &telemetry,
                &running,
                format!("Output stream failed: {error}"),
            )
        },
        None,
    )?)
}

pub struct Session {
    input: Option<Stream>,
    output: Option<Stream>,
    running: Arc<AtomicBool>,
    telemetry: Arc<Mutex<Telemetry>>,
    worker: Option<JoinHandle<()>>,
    destination: String,
}

impl Session {
    pub fn start(settings: Settings, telemetry: Arc<Mutex<Telemetry>>) -> Result<Self> {
        settings.validate()?;
        ensure!(
            !settings.monitor,
            "Windows monitoring is not implemented. Select a virtual cable output instead."
        );
        let output_id = settings.output.as_deref().filter(|s| !s.is_empty())
            .context("Select a virtual cable's playback endpoint. Clearwave never automatically sends your microphone to speakers.")?;
        let input_device = find_device(&settings.input, DeviceKind::Input)?;
        let output_device = find_device(output_id, DeviceKind::Output)?;
        let input_supported = input_device.default_input_config()?;
        let output_supported = output_device.default_output_config()?;
        let input_config = input_supported.config();
        let output_config = output_supported.config();
        for config in [&input_config, &output_config] {
            ensure!(config.channels > 0 && (8_000..=192_000).contains(&config.sample_rate.0),
                "This preview supports endpoint rates from 8 to 192 kHz and nonzero channel counts.");
        }
        let input_rate = input_config.sample_rate.0;
        let output_rate = output_config.sample_rate.0;
        let capture = Arc::new(ArrayQueue::new(input_rate as usize / 2));
        let playback = Arc::new(ArrayQueue::new(output_rate as usize / 2));
        // Small initial headroom absorbs callback scheduling differences.
        for _ in 0..output_rate * 30 / 1_000 {
            let _ = playback.push(0.0);
        }
        let counters = Arc::new(Counters::default());
        let running = Arc::new(AtomicBool::new(true));
        macro_rules! build {
            ($function:ident, $device:expr, $config:expr, $format:expr, $queue:expr) => {
                match $format {
                    SampleFormat::F32 => $function::<f32>($device, $config, $queue.clone(), counters.clone(), running.clone(), telemetry.clone()),
                    SampleFormat::I16 => $function::<i16>($device, $config, $queue.clone(), counters.clone(), running.clone(), telemetry.clone()),
                    SampleFormat::U16 => $function::<u16>($device, $config, $queue.clone(), counters.clone(), running.clone(), telemetry.clone()),
                    other => bail!("Unsupported endpoint format {other:?}; select 16-bit PCM or 32-bit float in Windows Sound Settings."),
                }
            };
        }
        let input = build!(
            input_stream,
            &input_device,
            &input_config,
            input_supported.sample_format(),
            capture
        )?;
        let output = build!(
            output_stream,
            &output_device,
            &output_config,
            output_supported.sample_format(),
            playback
        )?;
        if let Ok(mut state) = telemetry.lock() {
            *state = Telemetry::default();
        }
        let mut session = Self {
            input: Some(input),
            output: Some(output),
            running: running.clone(),
            telemetry: telemetry.clone(),
            worker: None,
            destination: output_id.to_owned(),
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        session.worker = Some(thread::Builder::new().name("clearwave-dsp".into()).spawn(move || {
            let mut ready = Some(ready_tx);
            let mut processor = Processor::new();
            let mut to_model = Resampler::new(input_rate, SAMPLE_RATE);
            let mut to_output = Resampler::new(SAMPLE_RATE, output_rate);
            let mut frame = [0.0; FRAME_SIZE];
            let mut result = [0.0; FRAME_SIZE];
            let mut used = 0;
            let mut last_sample = Instant::now();
            let started = Instant::now();
            let target_depth = f64::from(output_rate) * 0.03;
            while running.load(Ordering::Relaxed) {
                if let Some(sample) = capture.pop() {
                    last_sample = Instant::now();
                    to_model.push(sample, |sample| {
                        frame[used] = sample;
                        used += 1;
                        if used != FRAME_SIZE { return; }
                        used = 0;
                        processor.process(&frame, &mut result, settings.denoise, settings.boost_db);
                        // Gently steer the playback queue toward 30 ms to tolerate independent clocks.
                        let relative_depth = (playback.len() as f64 - target_depth) / target_depth;
                        to_output.set_correction((relative_depth * 0.002).clamp(-0.005, 0.005));
                        for sample in result {
                            to_output.push(sample, |converted| {
                                if playback.push(converted).is_err() {
                                    counters.overflow.fetch_add(1, Ordering::Relaxed);
                                }
                            });
                        }
                        if let Ok(mut state) = telemetry.try_lock() {
                            state.input_peak = frame.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
                            state.output_peak = result.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
                            state.clipped_samples += frame.iter().filter(|x| x.abs() >= 1.0).count() as u64;
                        }
                        if let Some(sender) = ready.take() {
                            let _ = sender.send(());
                        }
                    });
                } else {
                    if last_sample.elapsed() > Duration::from_secs(3) {
                        fail(&telemetry, &running, "No microphone samples arrived for three seconds; check permissions or reconnect the device.".into());
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                let overflow = counters.overflow.load(Ordering::Relaxed);
                let underrun = counters.underrun.load(Ordering::Relaxed);
                if overflow > 0 || (started.elapsed() > Duration::from_secs(1) && underrun > u64::from(output_rate) / 4) {
                    fail(&telemetry, &running, format!("Audio buffering could not keep up ({overflow} dropped samples, {underrun} silence samples). Restart or choose different devices."));
                }
            }
            if let Ok(mut state) = telemetry.lock() { state.running = false; }
        }).context("Could not start the audio processing worker")?);
        session
            .output
            .as_ref()
            .unwrap()
            .play()
            .context("Could not start output playback")?;
        session
            .input
            .as_ref()
            .unwrap()
            .play()
            .context("Could not start microphone capture")?;
        ready_rx.recv_timeout(Duration::from_secs(4))
            .context("Microphone produced no processed audio; check Windows microphone permissions and the selected devices")?;
        ensure!(
            session.running.load(Ordering::Relaxed),
            "Audio stream failed during startup; reconnect the selected devices."
        );
        if let Ok(mut state) = session.telemetry.lock() {
            state.running = session.running.load(Ordering::Relaxed);
        }
        Ok(session)
    }

    pub fn virtual_source(&self) -> &str {
        &self.destination
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.input.take();
        self.output.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut state) = self.telemetry.lock() {
            state.running = false;
            state.input_peak = 0.0;
            state.output_peak = 0.0;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Streaming linear interpolation with a fractional phase across callback boundaries.
/// It is intentionally simple; a band-limited resampler is a future quality improvement.
struct Resampler {
    previous: Option<f32>,
    next: f64,
    base_step: f64,
    step: f64,
}

impl Resampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        let step = f64::from(input_rate) / f64::from(output_rate);
        Self {
            previous: None,
            next: 0.0,
            base_step: step,
            step,
        }
    }

    fn set_correction(&mut self, correction: f64) {
        self.step = self.base_step * (1.0 + correction);
    }

    fn push(&mut self, sample: f32, mut emit: impl FnMut(f32)) {
        if let Some(previous) = self.previous {
            while self.next <= 1.0 {
                emit(previous + (sample - previous) * self.next as f32);
                self.next += self.step;
            }
            self.next -= 1.0;
        } else {
            emit(sample);
            self.next = self.step;
        }
        self.previous = Some(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_preserves_equal_rate_and_phase() {
        let mut resampler = Resampler::new(48_000, 48_000);
        let mut result = Vec::new();
        for sample in [0.0, 0.25, -0.25, 0.5] {
            resampler.push(sample, |x| result.push(x));
        }
        assert_eq!(result, [0.0, 0.25, -0.25, 0.5]);
    }

    #[test]
    fn resampler_changes_rate_without_boundary_discontinuities() {
        for (input, output) in [(44_100, 48_000), (96_000, 48_000), (48_000, 44_100)] {
            let mut resampler = Resampler::new(input, output);
            let mut result = Vec::new();
            for _ in 0..input {
                resampler.push(0.25, |x| result.push(x));
            }
            assert!((result.len() as i64 - i64::from(output)).abs() <= 2);
            assert!(result.iter().all(|x| (*x - 0.25).abs() < 0.00001));
        }
    }

    #[test]
    fn ids_do_not_depend_on_enumeration_order() {
        assert_eq!(device_id(DeviceKind::Input, "USB Mic"), "input:USB Mic");
        assert_ne!(
            device_id(DeviceKind::Input, "Cable"),
            device_id(DeviceKind::Output, "Cable")
        );
    }
}
