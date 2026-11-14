//! 48 kHz mono processing. Audio remains in memory; no recording or network I/O.
use nnnoiseless::DenoiseState;

pub const FRAME_SIZE: usize = 480;
pub const SAMPLE_RATE: u32 = 48_000;

pub struct Processor {
    denoiser: Box<DenoiseState<'static>>,
    warmed: bool,
    gain: f32,
    limiter_gain: f32,
    speech_gate: SpeechGate,
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor {
    pub fn new() -> Self {
        Self {
            denoiser: DenoiseState::new(),
            warmed: false,
            gain: 1.0,
            limiter_gain: 1.0,
            speech_gate: SpeechGate::default(),
        }
    }

    pub fn process(
        &mut self,
        input: &[f32; FRAME_SIZE],
        output: &mut [f32; FRAME_SIZE],
        denoise: bool,
        boost_db: f32,
    ) {
        let clean = input.map(|x| {
            if x.is_finite() {
                x.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        });
        if denoise {
            // RNNoise expects float samples on the signed 16-bit PCM scale.
            let scaled = clean.map(|x| x * 32767.0);
            let speech_probability = self.denoiser.process_frame(output, &scaled);
            for sample in output.iter_mut() {
                *sample /= 32767.0;
            }
            if !self.warmed {
                output.fill(0.0);
                self.warmed = true;
            }
            self.speech_gate.process(output, speech_probability);
        } else {
            output.copy_from_slice(&clean);
        }
        let db = if boost_db.is_finite() {
            boost_db.clamp(0.0, 18.0)
        } else {
            0.0
        };
        let target = 10.0_f32.powf(db / 20.0);
        for sample in output.iter_mut() {
            // Smooth boost changes, then an instantaneous-attack, slow-release limiter.
            self.gain += (target - self.gain) * 0.002;
            let boosted = *sample * self.gain;
            let required = if boosted.abs() > 0.98 {
                0.98 / boosted.abs()
            } else {
                1.0
            };
            if required < self.limiter_gain {
                self.limiter_gain = required;
            } else {
                self.limiter_gain += (required - self.limiter_gain) * 0.0005;
            }
            *sample = if boosted.is_finite() {
                (boosted * self.limiter_gain).clamp(-0.98, 0.98)
            } else {
                0.0
            };
        }
    }
}

/// Suppress residual noise between phrases using RNNoise's speech confidence.
/// A frame of lookahead opens the gate before a detected word reaches playback.
struct SpeechGate {
    pending: [f32; FRAME_SIZE],
    open: bool,
    hold_frames: usize,
    gain: f32,
}

impl Default for SpeechGate {
    fn default() -> Self {
        Self {
            pending: [0.0; FRAME_SIZE],
            open: false,
            hold_frames: 0,
            gain: 0.0,
        }
    }
}

impl SpeechGate {
    fn process(&mut self, audio: &mut [f32; FRAME_SIZE], probability: f32) {
        // Different opening/closing thresholds prevent rapid toggling on faint noise.
        let threshold = if self.open { 0.25 } else { 0.55 };
        if probability.is_finite() && probability >= threshold {
            self.open = true;
            self.hold_frames = 20; // Keep word endings and short pauses for 200 ms.
        } else if self.hold_frames > 0 {
            self.hold_frames -= 1;
        } else {
            self.open = false;
        }
        let current = *audio;
        *audio = self.pending;
        self.pending = current;
        for sample in audio {
            // A 2 ms attack and 60 ms release avoid abrupt discontinuities/clicks.
            self.gain = if self.open {
                (self.gain + 1.0 / 96.0).min(1.0)
            } else {
                (self.gain - 1.0 / 2880.0).max(0.0)
            };
            *sample *= self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_noise_is_silent_before_and_after_speech() {
        let mut gate = SpeechGate::default();
        for _ in 0..50 {
            let mut noise = [0.003; FRAME_SIZE];
            gate.process(&mut noise, 0.05);
            assert!(noise.iter().all(|sample| *sample == 0.0));
        }
        for _ in 0..5 {
            gate.process(&mut [0.1; FRAME_SIZE], 0.9);
        }
        let mut noise = [0.003; FRAME_SIZE];
        for _ in 0..40 {
            noise.fill(0.003);
            gate.process(&mut noise, 0.05);
        }
        assert!(noise.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn quiet_speech_and_short_pauses_are_preserved() {
        let mut gate = SpeechGate::default();
        let mut audio = [0.0005; FRAME_SIZE];
        gate.process(&mut audio, 0.9);
        audio.fill(0.0005);
        gate.process(&mut audio, 0.9);
        assert!(audio.iter().all(|sample| (*sample - 0.0005).abs() < 1e-7));
        for _ in 0..10 {
            audio.fill(0.0005);
            gate.process(&mut audio, 0.1);
            assert!(audio.iter().all(|sample| (*sample - 0.0005).abs() < 1e-7));
        }
    }

    #[test]
    fn gate_release_is_smooth_and_lookahead_preserves_onset() {
        let mut gate = SpeechGate::default();
        gate.process(&mut [0.1; FRAME_SIZE], 0.0);
        let mut audio = [0.1; FRAME_SIZE];
        gate.process(&mut audio, 0.9);
        assert!(audio[0] > 0.0);
        assert!((audio[FRAME_SIZE - 1] - 0.1).abs() < 1e-6);
        let mut previous = audio[FRAME_SIZE - 1];
        for _ in 0..40 {
            audio.fill(0.1);
            gate.process(&mut audio, 0.0);
            for sample in audio {
                assert!((sample - previous).abs() < 0.0001);
                previous = sample;
            }
        }
        assert_eq!(previous, 0.0);
    }

    #[test]
    fn bypass_preserves_quiet_signal() {
        let input = std::array::from_fn(|i| (i as f32 * 0.2).sin() * 0.1);
        let mut output = [0.0; FRAME_SIZE];
        Processor::new().process(&input, &mut output, false, 0.0);
        for (a, b) in input.iter().zip(output) {
            assert!((*a - b).abs() < 0.00001);
        }
    }

    #[test]
    fn boost_increases_quiet_audio_and_limits_loud_audio() {
        let mut processor = Processor::new();
        let mut output = [0.0; FRAME_SIZE];
        for _ in 0..20 {
            processor.process(&[0.01; FRAME_SIZE], &mut output, false, 18.0);
        }
        assert!(output[479] > 0.07);
        processor.process(&[0.8; FRAME_SIZE], &mut output, false, 18.0);
        assert!(output.iter().all(|v| v.abs() <= 0.98));
    }

    #[test]
    fn invalid_samples_and_silence_stay_finite() {
        for denoise in [false, true] {
            let mut processor = Processor::new();
            let mut output = [0.0; FRAME_SIZE];
            for _ in 0..10 {
                processor.process(&[f32::NAN; FRAME_SIZE], &mut output, denoise, f32::NAN);
                assert!(output.iter().all(|x| x.is_finite() && x.abs() < 0.00001));
            }
        }
    }
}
