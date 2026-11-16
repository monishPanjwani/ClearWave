//! Original stereo DSP. Cookbook peaking/high-pass biquads, mid/side width,
//! linked feed-forward compression and instantaneous peak limiting at 48 kHz.
use crate::settings::{Processing, FREQUENCIES};
const FS: f64 = 48000.0;
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Default for Biquad {
    fn default() -> Self {
        Self {
            b: [1.0, 0.0, 0.0],
            a: [0.0; 2],
            z: [0.0; 2],
        }
    }
}
impl Biquad {
    fn peak(f: f32, db: f32) -> Self {
        let a = 10_f64.powf(db as f64 / 40.0);
        let w = std::f64::consts::TAU * f as f64 / FS;
        let alpha = w.sin() / 2.0;
        let a0 = 1.0 + alpha / a;
        Self {
            b: [
                (1.0 + alpha * a) / a0,
                -2.0 * w.cos() / a0,
                (1.0 - alpha * a) / a0,
            ],
            a: [-2.0 * w.cos() / a0, (1.0 - alpha / a) / a0],
            z: [0.0; 2],
        }
    }
    fn highpass(f: f32) -> Self {
        if f < 1.0 {
            return Self::default();
        }
        let w = std::f64::consts::TAU * f as f64 / FS;
        let c = w.cos();
        let alpha = w.sin() / 2_f64.sqrt();
        let a0 = 1.0 + alpha;
        Self {
            b: [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }
    fn sample(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x as f64 + self.z[0];
        self.z[0] = self.b[1] * x as f64 - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x as f64 - self.a[1] * y;
        y as f32
    }
}
#[derive(Clone)]
struct Bank {
    filters: [[Biquad; 8]; 2],
}
impl Bank {
    fn new(p: &Processing) -> Self {
        let mut b = Self {
            filters: [[Biquad::default(); 8]; 2],
        };
        for channel in &mut b.filters {
            for i in 0..5 {
                channel[i] = Biquad::peak(FREQUENCIES[i], p.eq_db[i]);
            }
            channel[5] = Biquad::peak(90.0, p.bass_db);
            channel[6] = Biquad::peak(3500.0, p.clarity_db);
            channel[7] = Biquad::highpass(p.low_cut_hz);
        }
        b
    }
    fn frame(&mut self, mut x: [f32; 2]) -> [f32; 2] {
        for (c, y) in x.iter_mut().enumerate() {
            for f in &mut self.filters[c] {
                *y = f.sample(*y);
            }
        }
        x
    }
}
pub struct Enhancer {
    current: Bank,
    previous: Bank,
    transition: usize,
    settings: Processing,
    pending: Option<Processing>,
    target_gain: f32,
    gain: f32,
    width: f32,
    threshold: f32,
    ratio: f32,
    compression: f32,
    limiter: f32,
}
impl Enhancer {
    pub fn new(p: Processing) -> Self {
        Self {
            current: Bank::new(&p),
            previous: Bank::new(&p),
            transition: 960,
            settings: p,
            pending: None,
            target_gain: 10_f32.powf(p.gain_db / 20.0),
            gain: 10_f32.powf(p.gain_db / 20.0),
            width: p.width,
            threshold: p.threshold_db,
            ratio: p.ratio,
            compression: 1.0,
            limiter: 1.0,
        }
    }
    pub fn update(&mut self, p: Processing) {
        if p == self.settings && self.pending.is_none() {
            return;
        }
        if p.validate().is_err() {
            return;
        }
        if self.transition < 960 {
            self.pending = Some(p);
            return;
        }
        self.target_gain = 10_f32.powf(p.gain_db / 20.0);
        if p.eq_db != self.settings.eq_db
            || p.bass_db != self.settings.bass_db
            || p.clarity_db != self.settings.clarity_db
            || p.low_cut_hz != self.settings.low_cut_hz
        {
            self.previous = self.current.clone();
            self.current = Bank::new(&p);
            self.transition = 0;
        }
        self.settings = p;
    }
    pub fn frame(&mut self, x: [f32; 2]) -> [f32; 2] {
        if self.transition >= 960 {
            if let Some(p) = self.pending.take() {
                self.update(p);
            }
        }
        let x = x.map(|x| {
            if x.is_finite() {
                x.clamp(-16.0, 16.0)
            } else {
                0.0
            }
        });
        let mut y = self.current.frame(x);
        if self.transition < 960 {
            let old = self.previous.frame(x);
            let t = self.transition as f32 / 960.0;
            for c in 0..2 {
                y[c] = old[c] * (1.0 - t) + y[c] * t;
            }
            self.transition += 1;
        }
        self.gain += (self.target_gain - self.gain) * 0.001;
        self.width += (self.settings.width - self.width) * 0.001;
        self.threshold += (self.settings.threshold_db - self.threshold) * 0.001;
        self.ratio += (self.settings.ratio - self.ratio) * 0.001;
        let mid = (y[0] + y[1]) * 0.5;
        let side = (y[0] - y[1]) * 0.5 * self.width;
        y = [(mid + side) * self.gain, (mid - side) * self.gain];
        let peak = y[0].abs().max(y[1].abs());
        let db = 20.0 * peak.max(1e-12).log10();
        let reduction = (db - self.threshold).max(0.0) * (1.0 - 1.0 / self.ratio);
        let target = 10_f32.powf(-reduction / 20.0);
        self.compression += (target - self.compression)
            * if target < self.compression {
                0.004
            } else {
                0.00015
            };
        for v in &mut y {
            *v *= self.compression;
        }
        let peak = y[0].abs().max(y[1].abs());
        let target = if peak > 0.98 { 0.98 / peak } else { 1.0 };
        if target < self.limiter {
            self.limiter = target;
        } else {
            self.limiter += (target - self.limiter) * 0.0005;
        }
        y.map(|v| {
            if v.is_finite() {
                (v * self.limiter).clamp(-0.98, 0.98)
            } else {
                0.0
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rms(p: Processing, f: f32) -> f32 {
        let mut e = Enhancer::new(p);
        let mut sum = 0.0;
        for i in 0..48000 {
            let y = e.frame([
                (std::f32::consts::TAU * f * i as f32 / 48000.0).sin() * 0.01,
                0.0,
            ]);
            if i > 24000 {
                sum += y[0] * y[0];
            }
            assert!(y[1].abs() < 1e-7);
        }
        (sum / 23999.0).sqrt()
    }
    #[test]
    fn rapid_eq_updates_preserve_continuity() {
        let mut e = Enhancer::new(Processing::default());
        let mut last = 0.0;
        for i in 0..48000 {
            if i % 48 == 0 {
                let mut p = Processing::default();
                p.eq_db[2] = if i % 96 == 0 { 12.0 } else { -12.0 };
                e.update(p);
            }
            let y = e.frame([
                (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.02,
                0.0,
            ]);
            assert!((y[0] - last).abs() < 0.012, "jump at {i}");
            last = y[0];
        }
    }
    #[test]
    fn flat_and_frequency_response() {
        let p = Processing::default();
        assert!((rms(p, 1000.0) - 0.01 / 2_f32.sqrt()).abs() < 1e-5);
        let mut q = p;
        q.eq_db[2] = 6.0;
        assert!(rms(q, 1000.0) / rms(p, 1000.0) > 1.9);
        assert!(rms(q, 80.0) / rms(p, 80.0) < 1.02);
    }
    #[test]
    fn attenuation_compression_and_linked_limiter() {
        let mut p = Processing::default();
        let flat = rms(p, 1000.0);
        p.gain_db = -12.0;
        assert!((rms(p, 1000.0) / flat - 0.2512).abs() < 0.001);
        p.gain_db = 18.0;
        let mut e = Enhancer::new(p);
        for _ in 0..48000 {
            let y = e.frame([0.9, 0.45]);
            assert!(y[0] <= 0.98);
            assert!((y[0] - 2.0 * y[1]).abs() < 1e-6);
        }
        p.gain_db = 0.0;
        p.ratio = 4.0;
        p.threshold_db = -48.0;
        assert!(rms(p, 1000.0) < flat * 0.7);
    }
    #[test]
    fn finite_and_live_transition() {
        let mut e = Enhancer::new(Processing::default());
        assert_eq!(e.frame([f32::NAN, f32::INFINITY]), [0.0; 2]);
        let mut last = 0.1;
        for i in 0..48000 {
            if i == 1000 {
                let p = Processing {
                    gain_db: -18.0,
                    eq_db: [12.0; 5],
                    ..Default::default()
                };
                e.update(p);
            }
            let y = e.frame([0.1, 0.1]);
            assert!(y.iter().all(|v| v.is_finite() && v.abs() <= 0.98));
            assert!((y[0] - last).abs() < 0.03);
            last = y[0];
        }
    }
}
