use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

const RING: usize = 1 << 14;
pub const FFT_SIZE: usize = 4096;

const F_LO: f32 = 20.0;
const F_HI: f32 = 16_000.0;
const FLOOR_DB: f32 = -54.0;
const CEIL_DB: f32 = -12.0;
const ATTACK: f32 = 0.035;
const FALL: f32 = 1.4;
const LIGHT_AT: f32 = 0.7;
const OUT_AT: f32 = 0.15;
const STALE: Duration = Duration::from_millis(200);

pub struct Tap {
    buf: Box<[AtomicU32]>,
    left: Box<[AtomicU32]>,
    right: Box<[AtomicU32]>,
    pos: AtomicUsize,
    rate: AtomicU32,
}

impl Default for Tap {
    fn default() -> Self {
        Tap::new()
    }
}

impl Tap {
    pub fn new() -> Tap {
        let ring = || (0..RING).map(|_| AtomicU32::new(0)).collect();
        Tap { buf: ring(), left: ring(), right: ring(), pos: AtomicUsize::new(0), rate: AtomicU32::new(48_000) }
    }

    pub fn set_rate(&self, rate: u32) {
        self.rate.store(rate.max(1), Ordering::Relaxed);
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn writer(&self) -> TapWriter<'_> {
        TapWriter { tap: self, pos: self.pos.load(Ordering::Relaxed) }
    }

    pub fn written(&self) -> usize {
        self.pos.load(Ordering::Acquire)
    }

    pub fn latest(&self, out: &mut [f32]) {
        Self::read(&self.buf, self.pos.load(Ordering::Acquire), out);
    }

    pub fn stereo_until(&self, end: usize, left: &mut [f32], right: &mut [f32]) {
        let now = self.pos.load(Ordering::Acquire);
        let end = end.min(now).max(now.saturating_sub(RING - left.len().max(right.len()).min(RING)));
        Self::read(&self.left, end, left);
        Self::read(&self.right, end, right);
    }

    fn read(buf: &[AtomicU32], end: usize, out: &mut [f32]) {
        let n = out.len().min(RING);
        for (i, o) in out.iter_mut().enumerate() {
            *o = match (end + i).checked_sub(n) {
                Some(idx) if i < n => f32::from_bits(buf[idx & (RING - 1)].load(Ordering::Relaxed)),
                _ => 0.0,
            };
        }
    }
}

pub struct TapWriter<'a> {
    tap: &'a Tap,
    pos: usize,
}

impl TapWriter<'_> {
    #[inline]
    pub fn push(&mut self, sample: f32) {
        self.push_stereo(sample, sample, sample);
    }

    #[inline]
    pub fn push_stereo(&mut self, mono: f32, left: f32, right: f32) {
        let i = self.pos & (RING - 1);
        self.tap.buf[i].store(mono.to_bits(), Ordering::Relaxed);
        self.tap.left[i].store(left.to_bits(), Ordering::Relaxed);
        self.tap.right[i].store(right.to_bits(), Ordering::Relaxed);
        self.pos = self.pos.wrapping_add(1);
    }
}

impl Drop for TapWriter<'_> {
    fn drop(&mut self) {
        self.tap.pos.store(self.pos, Ordering::Release);
    }
}

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    targets: Vec<f32>,
    levels: Vec<f32>,
    lit: Vec<u16>,
    segments: u16,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer::new()
    }
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Analyzer {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; FFT_SIZE / 2 + 1],
            fft,
            window,
            targets: Vec::new(),
            levels: Vec::new(),
            lit: Vec::new(),
            segments: 0,
            last: None,
            last_written: 0,
            quiet_since: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.levels.iter().any(|&l| l > 0.0) || self.lit.iter().any(|&s| s > 0)
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, columns: usize, segments: u16, now: Instant) -> &[u16] {
        self.resize(columns, segments);
        if columns == 0 || segments == 0 {
            return &self.lit;
        }
        let dt = self.last.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32()).min(0.25);
        self.last = Some(now);

        let written = tap.written();
        if written != self.last_written {
            self.last_written = written;
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(now);
        }
        let stale = self.quiet_since.is_some_and(|t| now.saturating_duration_since(t) >= STALE);

        if stale {
            self.targets.iter_mut().for_each(|t| *t = 0.0);
        } else {
            self.measure(tap);
        }

        let segs = segments as f32;
        for i in 0..columns {
            let (t, l) = (self.targets[i], &mut self.levels[i]);
            if t > *l {
                *l += (t - *l) * (1.0 - (-dt / ATTACK).exp());
            } else {
                *l = (*l - FALL * dt).max(t);
            }
            let v = *l * segs;
            let reached = (v + 1.0 - LIGHT_AT).floor().clamp(0.0, segs) as u16;
            let held = (v + 1.0 - OUT_AT).floor().clamp(0.0, segs) as u16;
            let lit = &mut self.lit[i];
            if reached > *lit {
                *lit = reached;
            } else if held < *lit {
                *lit = held;
            }
        }
        &self.lit
    }

    #[cfg(test)]
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    fn resize(&mut self, columns: usize, segments: u16) {
        if columns != self.levels.len() {
            let old = std::mem::take(&mut self.levels);
            self.levels = (0..columns)
                .map(|i| if old.is_empty() { 0.0 } else { old[i * old.len() / columns.max(1)] })
                .collect();
            self.targets = vec![0.0; columns];
            self.lit = vec![0; columns];
        }
        if segments != self.segments {
            self.segments = segments;
            self.lit.iter_mut().for_each(|s| *s = 0);
        }
    }

    fn measure(&mut self, tap: &Tap) {
        tap.latest(&mut self.input);
        for (x, w) in self.input.iter_mut().zip(&self.window) {
            *x *= w;
        }
        if self.fft.process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch).is_err() {
            return;
        }
        for (p, c) in self.power.iter_mut().zip(&self.spectrum) {
            *p = c.norm_sqr();
        }

        let rate = tap.rate() as f32;
        let df = rate / FFT_SIZE as f32;
        let hi = top_frequency(rate as u32);
        let n = self.targets.len();
        let reference = 3.0 / 32.0 * (FFT_SIZE as f32).powi(2);
        let width_db = 10.0 * (n as f32 / 32.0).log10();
        let ratio = (hi / F_LO).powf(1.0 / n as f32);
        for (b, target) in self.targets.iter_mut().enumerate() {
            let lo = F_LO * ratio.powi(b as i32);
            let up = lo * ratio;
            let k0 = (lo / df).ceil() as usize;
            let k1 = ((up / df).ceil() as usize).min(self.power.len());
            let energy = if k1 > k0 {
                self.power[k0..k1].iter().sum::<f32>()
            } else {
                let x = (lo * up).sqrt() / df;
                let k = (x.floor() as usize).min(self.power.len() - 2);
                let t = x - k as f32;
                (self.power[k] * (1.0 - t) + self.power[k + 1] * t) * (up - lo) / df
            };
            let center = (lo * up).sqrt();
            let db = 10.0 * (energy / reference).max(1e-12).log10() + width_db + tilt_db(center);
            *target = ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0);
        }
    }
}

pub fn top_frequency(rate: u32) -> f32 {
    F_HI.min(rate as f32 * 0.45)
}

pub fn column_of(freq: f32, columns: usize, rate: u32) -> Option<usize> {
    let hi = top_frequency(rate);
    if columns == 0 || freq < F_LO || freq > hi * 1.0001 {
        return None;
    }
    let x = (freq / F_LO).ln() / (hi / F_LO).ln() * columns as f32;
    Some((x as usize).min(columns - 1))
}

fn tilt_db(f: f32) -> f32 {
    let octaves = (f / 1000.0).log2();
    if octaves > 0.0 { 3.0 * octaves } else { 1.5 * octaves }
}
