use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::spectrum::{Tap, FFT_SIZE};
use crate::theme::{BokehColors, BOKEH_HUES};

pub const CLASSES: usize = BOKEH_HUES;

pub const BAND_EDGES: [f32; CLASSES + 1] = [20.0, 60.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0, 20_000.0];

pub fn top_of_range(rate: u32) -> f32 {
    BAND_EDGES[CLASSES].min(rate as f32 * 0.45)
}

const TILT_HI: f32 = 2.5;
const TILT_LO: f32 = 1.0;
const SPAN_DB: f32 = 54.0;
const ABS_CEIL: f32 = -16.0;
const ADAPT: f32 = 0.7;
const HEADROOM: f32 = 6.0;
const BAND_GAMMA: f32 = 1.3;
const LEVEL_ATTACK: f32 = 2.0;
const LEVEL_RELEASE: f32 = 10.0;
const REL_TAU: f32 = 1.5;
const REL_SPAN: f32 = 12.0;
pub(crate) const SONG_SHARE: f32 = 0.15;
const RMS_WINDOW: usize = 2048;
pub(crate) const STALE: Duration = Duration::from_millis(200);
const SILENCE_PEAK: f32 = 1.0e-4;
const SILENCE_TAIL: usize = 1024;
pub(crate) const SILENCE_HOLD: f32 = 0.12;
pub(crate) const FADE_OUT: f32 = 2.5;
pub(crate) const FADE_IN: f32 = 0.35;
pub(crate) const MIN_STEP: f32 = 0.028;

pub(crate) struct Bands {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    rate: u32,
    k0: [usize; CLASSES],
    k1: [usize; CLASSES],
    octaves: [f32; CLASSES],
    tilt: [f32; CLASSES],
    live: [bool; CLASSES],
    pub(crate) db: [f32; CLASSES],
    mean_db: [f32; CLASSES],
    have_mean: bool,
    level: f32,
    rms_mem: f32,
    pub(crate) target: [f32; CLASSES],
    pub(crate) loudness: f32,
}

impl Bands {
    pub(crate) fn new() -> Bands {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos()).collect();
        Bands {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; FFT_SIZE / 2 + 1],
            fft,
            window,
            rate: 0,
            k0: [0; CLASSES],
            k1: [0; CLASSES],
            octaves: [1.0; CLASSES],
            tilt: [0.0; CLASSES],
            live: [false; CLASSES],
            db: [-120.0; CLASSES],
            mean_db: [-120.0; CLASSES],
            have_mean: false,
            level: -30.0,
            rms_mem: -30.0,
            target: [0.0; CLASSES],
            loudness: 0.0,
        }
    }

    fn layout(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let df = rate as f32 / FFT_SIZE as f32;
        let top = top_of_range(rate);
        let bins = FFT_SIZE / 2 + 1;
        for c in 0..CLASSES {
            let lo = BAND_EDGES[c];
            let hi = BAND_EDGES[c + 1].min(top);
            self.live[c] = hi > lo * 1.05;
            if !self.live[c] {
                (self.k0[c], self.k1[c], self.octaves[c], self.tilt[c]) = (0, 0, 1.0, 0.0);
                continue;
            }
            let mut k0 = (lo / df).ceil() as usize;
            let mut k1 = ((hi / df).ceil() as usize).min(bins);
            if k1 <= k0 {
                k0 = (((lo * hi).sqrt() / df).round() as usize).min(bins - 2);
                k1 = k0 + 1;
            }
            self.k0[c] = k0;
            self.k1[c] = k1;
            self.octaves[c] = (hi / lo).log2().max(0.25);
            let octaves_from_1k = ((lo * hi).sqrt() / 1000.0).log2();
            self.tilt[c] = if octaves_from_1k > 0.0 { TILT_HI * octaves_from_1k } else { TILT_LO * octaves_from_1k };
        }
        self.have_mean = false;
    }

    pub(crate) fn update(&mut self, tap: &Tap, dt: f32, stale: bool) -> bool {
        self.layout(tap.rate().max(1));
        if !stale {
            tap.latest(&mut self.input);
        }
        let quiet = stale || self.input[FFT_SIZE - SILENCE_TAIL..].iter().all(|x| x.abs() < SILENCE_PEAK);
        if quiet {
            self.target = [0.0; CLASSES];
            self.loudness = 0.0;
            self.have_mean = false;
            return true;
        }

        let tail = &self.input[FFT_SIZE - RMS_WINDOW..];
        let rms = (tail.iter().map(|x| x * x).sum::<f32>() / RMS_WINDOW as f32).sqrt();
        let rms_db = 20.0 * (rms + 1.0e-7).log10();
        let tau = if rms_db > self.rms_mem { LEVEL_ATTACK } else { LEVEL_RELEASE };
        self.rms_mem += (rms_db - self.rms_mem) * (1.0 - (-dt / tau).exp());
        self.loudness = (0.72 + (rms_db - self.rms_mem) / 14.0).clamp(0.0, 1.0);

        for (x, w) in self.input.iter_mut().zip(&self.window) {
            *x *= w;
        }
        if self.fft.process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch).is_err() {
            return false;
        }
        for (p, c) in self.power.iter_mut().zip(&self.spectrum) {
            *p = c.norm_sqr();
        }

        let scale = 1.0 / (FFT_SIZE as f32 * 0.25).powi(2);
        for c in 0..CLASSES {
            self.db[c] = if self.live[c] {
                let sum: f32 = self.power[self.k0[c]..self.k1[c]].iter().sum();
                10.0 * (sum * scale / self.octaves[c] + 1.0e-12).log10() + self.tilt[c]
            } else {
                -120.0
            };
        }

        let mut sorted = self.db;
        sorted.sort_unstable_by(|a, b| b.total_cmp(a));
        let top = (sorted[0] + sorted[1]) / 2.0;
        let tau = if top > self.level { LEVEL_ATTACK } else { LEVEL_RELEASE };
        self.level += (top - self.level) * (1.0 - (-dt / tau).exp());
        let ceil = ADAPT * self.level.max(-45.0) + (1.0 - ADAPT) * ABS_CEIL + HEADROOM;
        let floor = ceil - SPAN_DB;

        if !self.have_mean {
            self.mean_db = self.db;
            self.have_mean = true;
        }
        let k = 1.0 - (-dt / REL_TAU).exp();
        for c in 0..CLASSES {
            self.mean_db[c] += (self.db[c] - self.mean_db[c]) * k;
            let level = ((self.db[c] - floor) / SPAN_DB).clamp(0.0, 1.0);
            let hit = (0.35 + (self.db[c] - self.mean_db[c]) / REL_SPAN).clamp(0.0, 1.0);
            self.target[c] = level.powf(BAND_GAMMA) * (0.55 + 0.45 * hit);
        }
        false
    }
}

const TEMPO_RATE: f32 = 30.0;
const TEMPO_WINDOW: usize = 180;
const TEMPO_EVERY: f32 = 0.5;
const BPM_LOW: f32 = 70.0;
const BPM_HIGH: f32 = 180.0;
const TEMPO_CLEAR: f32 = 0.20;
pub(crate) const BPM_REF: f32 = 120.0;
const PACE_MIN: f32 = 0.5;
const PACE_MAX: f32 = 1.8;
pub(crate) const PACE_TAU: f32 = 2.5;

pub(crate) struct Tempo {
    prev: [f32; CLASSES],
    primed: bool,
    flux: std::collections::VecDeque<f32>,
    pending: f32,
    owed: f32,
    since: f32,
    pub(crate) bpm: f32,
}

impl Tempo {
    pub(crate) fn new() -> Tempo {
        Tempo { prev: [0.0; CLASSES], primed: false, flux: std::collections::VecDeque::with_capacity(TEMPO_WINDOW + 1), pending: 0.0, owed: 0.0, since: 0.0, bpm: 0.0 }
    }

    pub(crate) fn update(&mut self, dt: f32, db: &[f32; CLASSES], quiet: bool, silent: bool) {
        if silent {
            self.flux.clear();
        }
        let onset = if quiet || !self.primed {
            0.0
        } else {
            db.iter().zip(&self.prev).map(|(d, p)| (d - p).clamp(0.0, 12.0)).sum::<f32>() / CLASSES as f32
        };
        self.primed = !quiet;
        self.prev = *db;
        self.pending = self.pending.max(onset);
        self.owed += dt * TEMPO_RATE;
        let n = (self.owed.floor() as usize).min(8);
        self.owed = (self.owed - self.owed.floor()).min(1.0);
        for _ in 0..n {
            if self.flux.len() == TEMPO_WINDOW {
                self.flux.pop_front();
            }
            self.flux.push_back(self.pending);
            self.pending = 0.0;
        }
        self.since += dt;
        if self.since >= TEMPO_EVERY {
            self.since = 0.0;
            self.estimate();
        }
    }

    fn at(r: &[f32], lag: f32) -> f32 {
        let (i, f) = (lag.floor() as usize, lag.fract());
        match (r.get(i), r.get(i + 1)) {
            (Some(a), Some(b)) => a + (b - a) * f,
            (Some(a), None) => *a,
            _ => 0.0,
        }
    }

    fn estimate(&mut self) {
        let n = self.flux.len();
        if n < TEMPO_WINDOW / 2 {
            return;
        }
        let mean = self.flux.iter().sum::<f32>() / n as f32;
        let x: Vec<f32> = self.flux.iter().map(|v| v - mean).collect();
        let r0 = x.iter().map(|v| v * v).sum::<f32>() / n as f32;
        if r0 < 0.05 {
            return;
        }
        let longest = (2.0 * 60.0 * TEMPO_RATE / BPM_LOW).ceil() as usize + 2;
        let r: Vec<f32> = (0..=longest.min(n - 1)).map(|lag| x[..n - lag].iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>() / (n - lag) as f32 / r0).collect();
        let mut best = (0.0f32, 0.0f32);
        let mut bpm = BPM_LOW;
        while bpm <= BPM_HIGH {
            let lag = 60.0 * TEMPO_RATE / bpm;
            let score = Self::at(&r, lag) + 0.5 * Self::at(&r, 2.0 * lag);
            if score > best.0 {
                best = (score, bpm);
            }
            bpm += 0.5;
        }
        if best.1 > 0.0 && Self::at(&r, 60.0 * TEMPO_RATE / best.1) >= TEMPO_CLEAR {
            self.bpm = best.1;
        }
    }
}

pub(crate) fn pace_for(bpm: f32) -> f32 {
    if bpm <= 0.0 {
        1.0
    } else {
        (bpm / BPM_REF).clamp(PACE_MIN, PACE_MAX)
    }
}

const RADIUS: [f32; CLASSES] = [6.0, 4.4, 3.2, 2.3, 1.7, 1.25, 0.9, 0.66, 0.48, 0.36];
const DOTTED: usize = 5;
const ALPHA: [f32; CLASSES] = [0.66, 0.70, 0.75, 0.80, 0.86, 0.92, 0.96, 1.0, 1.0, 0.92];
const DENSITY: [f32; CLASSES] = [2.0, 2.0, 5.0, 9.0, 14.0, 22.0, 28.0, 34.0, 38.0, 42.0];
const DOT: f32 = 0.5;
const DOTS: [(usize, usize, u8); 8] =
    [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (0, 3, 0x40), (1, 0, 0x08), (1, 1, 0x10), (1, 2, 0x20), (1, 3, 0x80)];
const BRAILLE: u32 = 0x2800;
const MIX: [Style; 10] = [
    Style::Ring,
    Style::Solid,
    Style::Soft,
    Style::Ring,
    Style::Solid,
    Style::Ring,
    Style::Soft,
    Style::Solid,
    Style::Ring,
    Style::Soft,
];
const STYLE_GAIN: [f32; 3] = [1.0, 0.74, 0.90];
const DIMMEST: f32 = 0.62;
const RING_MORE: f32 = 0.4;
const BODY_LOW: f32 = 0.12;
const BODY_HIGH: f32 = 0.32;
const BODY_PULSE: f32 = 0.20;
const GLOW_LOW: f32 = 0.9;
const GLOW_HIGH: f32 = 1.7;
const GLOW_SPREAD: f32 = 0.4;
const GROW: f32 = 0.40;
const READY_SPREAD: f32 = 0.12;
const FLUTTER: f32 = 0.10;
const WEIGHT_LIFT: f32 = 0.08;
const WEIGHT_BIAS: [f32; CLASSES] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.05, 0.03, 0.0, -0.03, -0.05];
const EASE_IN: f32 = 5.0;
const EASE_OUT: f32 = 25.0;
const GONE: f32 = 0.08;
const BRIGHT_REST: f32 = 0.10;
const SHADE_FROM: f32 = 0.22;
const SHADE_TO: f32 = 0.80;
const PEAK_BASE: f32 = 0.55;
const OVERLAP: f32 = 0.45;
const T_FULL: f32 = 0.62;
const T_MID: f32 = 0.30;
const T_LOW: f32 = 0.10;
const HYST: f32 = 0.035;
pub(crate) const PULSE_ATTACK: f32 = 0.035;
pub(crate) const PULSE_FALL: f32 = 2.6;
const LEADERS: usize = 3;
const LEAD_MIN: f32 = 0.15;
const LEAD_STICKY: f32 = 0.05;
const LEAD_KEEP: f32 = 0.6;
const LEAD_RISE: f32 = 20.0;
const LEAD_FALL: f32 = 8.0;
const LEAD_LIFT: f32 = 0.34;
const SWAY: [f32; 2] = [1.0, 0.6];
const SWAY_BEATS: [f32; 2] = [8.0, 12.0];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Style {
    #[default]
    Ring,
    Solid,
    Soft,
}

#[derive(Clone, Copy, Default)]
struct Circle {
    class: u8,
    home: [f32; 2],
    size: f32,
    alpha: f32,
    breath: [f32; 2],
    sway: [f32; 2],
    style: Style,
    shape: [f32; 2],
    at: [f32; 2],
    radius: f32,
    rank: f32,
    vis: f32,
    feel: f32,
    weight: f32,
}

impl Circle {
    fn thickness(&self, d: f32, pulse: f32) -> f32 {
        let r = self.radius;
        if d > r {
            return 0.0;
        }
        match self.style {
            Style::Solid => 1.0,
            Style::Ring => {
                let rim = (DOT + RING_MORE * self.shape[0]).min(0.3 * r).max(DOT);
                if d > r - rim {
                    1.0
                } else {
                    BODY_LOW + (BODY_HIGH - BODY_LOW) * self.shape[1] + BODY_PULSE * pulse
                }
            }
            Style::Soft => {
                let u = d / r;
                let falloff = (GLOW_LOW + (GLOW_HIGH - GLOW_LOW) * self.shape[0]) * (1.0 - GLOW_SPREAD * pulse);
                (0.85 + 0.15 * self.shape[1]) * (1.0 - u * u).powf(falloff)
            }
        }
    }
}

pub(crate) fn hash01(seed: u64, a: u64, b: u64) -> f32 {
    let mut z = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(a.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(b.wrapping_mul(0x94D0_49BB_1331_11EB).wrapping_add(0x2545_F491_4F6C_DD1D));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64 / (1u64 << 53) as f64) as f32
}

fn scale_for(rows: u16) -> f32 {
    (rows as f32 / 12.0).clamp(0.5, 1.6)
}

fn counts(columns: usize, rows: u16) -> [usize; CLASSES] {
    let s = scale_for(rows);
    let area = columns as f32 * 2.0 * rows as f32 / 2040.0 / (s * s);
    std::array::from_fn(|c| (DENSITY[c] * area).round() as usize)
}

fn make_circles(columns: usize, rows: u16, seed: u64) -> Vec<Circle> {
    let n = counts(columns, rows);
    let mut out = Vec::new();
    for (c, &count) in n.iter().enumerate() {
        let start = (hash01(seed, (c * 1000 + 999) as u64, 0) * MIX.len() as f32) as usize;
        let team = |k: u64| hash01(seed, (c * 1000 + 998) as u64, k);
        let breath = [3.0 + 5.0 * team(6), TAU * team(7)];
        let sway = [TAU * team(10), TAU * team(11)];
        for j in 0..count {
            let id = (c * 1000 + j) as u64;
            let h = |k: u64| hash01(seed, id, k);
            out.push(Circle {
                class: c as u8,
                home: [h(1), h(2)],
                size: 0.85 + 0.30 * h(3),
                alpha: if c < DOTTED { DIMMEST + (1.0 - DIMMEST) * h(4) } else { 0.80 + 0.20 * h(4) },
                breath,
                sway,
                style: if c < DOTTED { MIX[(start + j) % MIX.len()] } else { Style::Solid },
                shape: [h(8), h(9)],
                at: [0.0; 2],
                radius: 0.0,
                rank: h(50),
                vis: 0.0,
                feel: 0.0,
                weight: 0.0,
            });
        }
    }
    out
}

fn fold(v: f32, lo: f32, hi: f32) -> f32 {
    let span = hi - lo;
    if span <= 0.0 {
        return lo;
    }
    let m = (v - lo).rem_euclid(2.0 * span);
    lo + if m > span { 2.0 * span - m } else { m }
}

pub(crate) fn fade_curve(presence: f32) -> f32 {
    let p = presence.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

pub(crate) const AT_REST: f32 = 0.004;

pub(crate) const FG_LEVELS: usize = 9;
const BRIGHT_MAX: f32 = 1.35;
const HEAT_REST: f32 = 0.13;

fn glow(k: f32, pulse: f32) -> f32 {
    let x = ((pulse - SHADE_FROM) / (SHADE_TO - SHADE_FROM)).clamp(0.0, 1.0);
    let (rest, peak) = (BRIGHT_REST * k, PEAK_BASE + (1.0 - PEAK_BASE) * k);
    rest + (peak - rest) * x
}

const GLYPHS: [char; 4] = [' ', '◌', '○', '●'];

struct Look {
    ground: Color,
    fg: [[Color; FG_LEVELS + 1]; CLASSES],
}

impl Look {
    fn new(colors: &BokehColors) -> Look {
        Look {
            ground: colors.at(0, 0.0),
            fg: std::array::from_fn(|c| std::array::from_fn(|i| colors.at(c, i as f32 / FG_LEVELS as f32))),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Acc {
    weight: f32,
    best: f32,
    class: u8,
    bright: f32,
    dots: u8,
    light: f32,
    most: f32,
    dclass: u8,
    dbright: f32,
}

#[derive(Clone, Copy, Default)]
struct Shown {
    glyph: u8,
    level: u8,
}

fn add(a: &mut Acc, class: usize, weight: f32, bright: f32) {
    a.weight += weight;
    if weight > a.best {
        a.best = weight;
        a.class = class as u8;
        a.bright = bright * (0.55 + 0.45 * (weight / T_FULL).min(1.0));
    }
}

fn add_dots(a: &mut Acc, class: usize, mask: u8, n: u32, bright: f32) {
    a.dots |= mask;
    let light = n as f32 * bright;
    a.light += light;
    if light > a.most {
        a.most = light;
        a.dclass = class as u8;
        a.dbright = bright;
    }
}

fn glyph_for(weight: f32, last: u8) -> u8 {
    let line = |t: f32, g: u8| if last >= g { t - HYST } else { t + HYST };
    if weight >= line(T_FULL, 3) {
        3
    } else if weight >= line(T_MID, 2) {
        2
    } else { u8::from(weight >= line(T_LOW, 1)) }
}

pub(crate) fn sticky(shown: &mut u8, want: usize, top: usize) -> usize {
    let have = *shown as usize;
    let now = if want == 0 || want == top || want.abs_diff(have) >= 2 { want } else { have };
    *shown = now as u8;
    now
}

fn dither(x: usize, y: usize) -> f32 {
    const QUARTER: [[u32; 2]; 2] = [[0, 2], [3, 1]];
    let mut v = 0;
    for bit in 0..3 {
        v = v * 4 + QUARTER[(y >> bit) & 1][(x >> bit) & 1];
    }
    (v as f32 + 0.5) / 64.0
}

fn braille(mask: u8) -> char {
    char::from_u32(BRAILLE + mask as u32).unwrap_or(' ')
}

fn cell_of(at: [f32; 2], cols: usize, rows: usize) -> Option<(usize, usize)> {
    let (i, j) = (at[0].floor(), (at[1] / 2.0).floor());
    (i >= 0.0 && j >= 0.0 && (i as usize) < cols && (j as usize) < rows).then_some((i as usize, j as usize))
}

fn stamp(acc: &mut [Acc], (cols, rows): (usize, usize), c: &Circle, pulse: f32, shows: f32, inside: &[bool]) {
    let class = c.class as usize;
    if class < DOTTED {
        return stamp_dots(acc, (cols, rows), c, pulse, shows);
    }
    if let Some((i, j)) = cell_of(c.at, cols, rows).filter(|&(i, j)| !inside.get(j * cols + i).copied().unwrap_or(false)) {
        let bright = glow(ALPHA[class] * c.alpha, c.feel) * shows;
        add(&mut acc[j * cols + i], class, c.weight, bright);
    }
}

fn mark_inside(inside: &mut [bool], (cols, rows): (usize, usize), c: &Circle) {
    let r = c.radius;
    let i0 = ((c.at[0] - r).floor() as isize).max(0);
    let i1 = ((c.at[0] + r).ceil() as isize).min(cols as isize - 1);
    let j0 = (((c.at[1] - r) / 2.0).floor() as isize).max(0);
    let j1 = (((c.at[1] + r) / 2.0).ceil() as isize).min(rows as isize - 1);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let nx = c.at[0].clamp(i as f32, i as f32 + 1.0);
            let ny = c.at[1].clamp(2.0 * j as f32, 2.0 * j as f32 + 2.0);
            let (dx, dy) = (nx - c.at[0], ny - c.at[1]);
            if dx * dx + dy * dy < r * r {
                inside[j as usize * cols + i as usize] = true;
            }
        }
    }
}

fn stamp_dots(acc: &mut [Acc], (cols, rows): (usize, usize), c: &Circle, pulse: f32, shows: f32) {
    let class = c.class as usize;
    let bright = glow(ALPHA[class] * c.alpha * STYLE_GAIN[c.style as usize], pulse) * shows;
    let r = c.radius;
    let i0 = ((c.at[0] - r).floor() as isize).max(0);
    let i1 = ((c.at[0] + r).ceil() as isize).min(cols as isize - 1);
    let j0 = (((c.at[1] - r) / 2.0).floor() as isize).max(0);
    let j1 = (((c.at[1] + r) / 2.0).ceil() as isize).min(rows as isize - 1);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let (mut mask, mut n) = (0u8, 0u32);
            for &(across, down, bit) in &DOTS {
                let (x, y) = (2 * i as usize + across, 4 * j as usize + down);
                let (dx, dy) = ((x as f32 + 0.5) * DOT - c.at[0], (y as f32 + 0.5) * DOT - c.at[1]);
                if c.thickness((dx * dx + dy * dy).sqrt(), pulse) > dither(x, y) {
                    mask |= bit;
                    n += 1;
                }
            }
            if n > 0 {
                add_dots(&mut acc[j as usize * cols + i as usize], class, mask, n, bright);
            }
        }
    }
}

pub struct Bokeh {
    bands: Bands,
    columns: usize,
    rows: u16,
    seed: u64,
    made_for: Option<(usize, u16, u64)>,
    circles: Vec<Circle>,
    order: Vec<usize>,
    taken: Vec<bool>,
    inside: Vec<bool>,
    showing: Vec<f32>,
    pulse: [f32; CLASSES],
    leading: [bool; CLASSES],
    lead: [f32; CLASSES],
    time: f32,
    motion_time: f32,
    pace: f32,
    tempo: Tempo,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
    look: Option<(usize, Look)>,
    shown: Vec<Shown>,
    acc: Vec<Acc>,
}

impl Default for Bokeh {
    fn default() -> Self {
        Bokeh::new()
    }
}

impl Bokeh {
    pub fn new() -> Bokeh {
        Bokeh {
            bands: Bands::new(),
            columns: 0,
            rows: 0,
            seed: 0,
            made_for: None,
            circles: Vec::new(),
            order: Vec::new(),
            taken: Vec::new(),
            inside: Vec::new(),
            showing: Vec::new(),
            pulse: [0.0; CLASSES],
            leading: [false; CLASSES],
            lead: [0.0; CLASSES],
            time: 0.0,
            motion_time: 0.0,
            pace: 1.0,
            tempo: Tempo::new(),
            last: None,
            last_written: 0,
            quiet_since: None,
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
            look: None,
            shown: Vec::new(),
            acc: Vec::new(),
        }
    }

    pub fn use_theme(&mut self, key: usize, colors: impl FnOnce() -> BokehColors) {
        if self.look.as_ref().map(|l| l.0) != Some(key) {
            self.look = Some((key, Look::new(&colors())));
        }
    }

    pub fn is_active(&self) -> bool {
        let fading = if self.silent { self.presence > 0.0 } else { self.presence < 1.0 };
        fading || self.pulse.iter().chain(&self.lead).any(|&p| p > AT_REST)
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        let (columns, rows) = (columns.max(1), rows.max(1));
        if self.made_for != Some((columns, rows, seed)) {
            self.circles = make_circles(columns, rows, seed);
            self.order = (0..self.circles.len()).filter(|&k| self.circles[k].class as usize >= DOTTED).collect();
            let circles = &self.circles;
            self.order.sort_by(|&a, &b| circles[a].rank.total_cmp(&circles[b].rank));
            self.made_for = Some((columns, rows, seed));
            self.shown.clear();
            (self.columns, self.rows) = (columns, rows);
            self.move_circles(f32::INFINITY);
        }
        (self.columns, self.rows, self.seed) = (columns, rows, seed);
        let dt = self.last.map(|t| now.saturating_duration_since(t).as_secs_f32());
        if dt.is_some_and(|dt| dt < MIN_STEP) {
            return;
        }
        let dt = dt.unwrap_or(0.0).min(0.25);
        self.last = Some(now);

        let written = tap.written();
        if written != self.last_written {
            self.last_written = written;
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(now);
        }
        let stale = written == 0 || self.quiet_since.is_some_and(|t| now.saturating_duration_since(t) >= STALE);

        let quiet = self.bands.update(tap, dt, stale);
        self.quiet_for = if quiet { (self.quiet_for + dt).min(10.0) } else { 0.0 };
        self.silent = self.quiet_for >= SILENCE_HOLD;
        self.tempo.update(dt, &self.bands.db, quiet, self.silent);
        self.fade(dt);
        self.step(dt);
    }

    fn fade(&mut self, dt: f32) {
        if self.silent {
            self.presence = (self.presence - dt / FADE_OUT).max(0.0);
        } else {
            self.presence = (self.presence + dt / FADE_IN).min(1.0);
        }
    }

    fn step(&mut self, dt: f32) {
        self.time += dt;
        self.pace += (pace_for(self.tempo.bpm) - self.pace) * (1.0 - (-dt / PACE_TAU).exp());
        self.motion_time += dt * self.pace;
        let up = 1.0 - (-dt / PULSE_ATTACK).exp();
        for c in 0..CLASSES {
            let want = ((1.0 - SONG_SHARE) * self.bands.target[c] + SONG_SHARE * self.bands.loudness).clamp(0.0, 1.0);
            let p = self.pulse[c];
            self.pulse[c] = if want > p { p + (want - p) * up } else { want.max(p - PULSE_FALL * dt) };
        }
        self.choose_leaders();
        for c in 0..CLASSES {
            let (to, rate) = if self.leading[c] { (1.0, LEAD_RISE) } else { (0.0, LEAD_FALL) };
            self.lead[c] += (to - self.lead[c]) * (1.0 - (-rate * dt).exp());
        }
        self.move_circles(dt);
    }

    fn choose_leaders(&mut self) {
        let score = |c: usize| self.pulse[c] + if self.leading[c] { LEAD_STICKY } else { 0.0 };
        let mut order: [usize; CLASSES] = std::array::from_fn(|c| c);
        order.sort_by(|&a, &b| score(b).total_cmp(&score(a)));
        let mut leading = [false; CLASSES];
        let mut taken = 0;
        for &c in &order {
            let need = if self.leading[c] { LEAD_MIN * LEAD_KEEP } else { LEAD_MIN };
            if self.bands.target[c] >= need {
                leading[c] = true;
                taken += 1;
                if taken == LEADERS {
                    break;
                }
            }
        }
        self.leading = leading;
    }

    fn move_circles(&mut self, dt: f32) {
        let (w, h) = (self.columns as f32, 2.0 * self.rows as f32);
        let s = scale_for(self.rows);
        let t = self.time;
        let beats = 2.0 * self.motion_time;
        for c in &mut self.circles {
            let class = c.class as usize;
            let character = class >= DOTTED;
            let radius = (RADIUS[class] * s * c.size).max(0.35);
            let mut off = [0.0f32; 2];
            if !character {
                for (a, o) in off.iter_mut().enumerate() {
                    *o = SWAY[a] * s * (TAU * beats / SWAY_BEATS[a] + c.sway[a]).sin();
                }
            }
            c.at = [
                fold(c.home[0] * w + off[0], -0.3 * radius, w + 0.3 * radius),
                fold(c.home[1] * h + off[1], -0.3 * radius, h + 0.3 * radius),
            ];
            let breath = 1.0 + 0.05 * (TAU * t / c.breath[0] + c.breath[1]).sin();
            let pulse = self.pulse[class];
            if character {
                let own = READY_SPREAD * (c.shape[0] - 0.5) + FLUTTER * (TAU * t / c.breath[0] + c.breath[1]).sin();
                c.feel = (pulse + own).clamp(0.0, 1.0);
                c.radius = radius * breath;
            } else {
                c.radius = (radius * breath * (1.0 + GROW * pulse)).max(0.8);
            }
        }
        self.give_way(dt);
    }

    fn give_way(&mut self, dt: f32) {
        let (cols, rows) = (self.columns, self.rows as usize);
        self.taken.clear();
        self.taken.resize(cols * rows, false);
        self.inside.clear();
        self.inside.resize(cols * rows, false);
        for c in self.circles.iter().filter(|c| (c.class as usize) < DOTTED) {
            mark_inside(&mut self.inside, (cols, rows), c);
        }
        self.showing.clear();
        self.showing.resize(cols * rows, 0.0);
        for &k in &self.order {
            let c = &self.circles[k];
            if let Some((i, j)) = cell_of(c.at, cols, rows) {
                let v = &mut self.showing[j * cols + i];
                *v = v.max(c.vis);
            }
        }
        for &k in &self.order {
            let c = &mut self.circles[k];
            let cell = cell_of(c.at, cols, rows);
            let around = |(i, j): (usize, usize)| {
                let (j0, j1) = (j.saturating_sub(1), (j + 1).min(rows - 1));
                (j0..=j1).flat_map(move |y| {
                    let reach = if y == j { 2 } else { 1 };
                    (i.saturating_sub(reach)..=(i + reach).min(cols - 1)).map(move |x| y * cols + x)
                })
            };
            let wanted = cell.filter(|&(i, j)| !self.inside[j * cols + i]).filter(|&at| around(at).all(|n| !self.taken[n]));
            if let Some((i, j)) = wanted {
                self.taken[j * cols + i] = true;
            }
            let clear = wanted.filter(|&at| c.vis > GONE || around(at).all(|n| self.showing[n] <= GONE));
            let (to, rate) = if clear.is_some() { (1.0, EASE_IN) } else { (0.0, EASE_OUT) };
            c.vis += (to - c.vis) * (1.0 - (-rate * dt).exp());
            c.weight = (c.feel + WEIGHT_LIFT + WEIGHT_BIAS[c.class as usize]).max(0.0) * c.vis;
        }
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        let Some((_, look)) = &self.look else { return };
        if area.is_empty() || area.width as usize != self.columns || area.height != self.rows {
            return;
        }
        let (cols, rows) = (self.columns, area.height as usize);
        let mut shown = std::mem::take(&mut self.shown);
        if shown.len() != cols * rows {
            shown = vec![Shown::default(); cols * rows];
        }
        let width = buf.area.width as usize;
        let mut put = |x: usize, y: usize, glyph: char, fg: Color, bg: Color| {
            let cell = &mut buf.content[(y - buf.area.y as usize) * width + (x - buf.area.x as usize)];
            cell.reset();
            if glyph != ' ' {
                cell.set_char(glyph);
            }
            cell.fg = fg;
            cell.bg = bg;
        };

        let shows = fade_curve(self.presence);
        if shows <= 0.0 {
            for y in 0..rows {
                for c in 0..cols {
                    put(area.x as usize + c, area.y as usize + y, ' ', look.ground, look.ground);
                }
            }
            shown.fill(Shown::default());
            self.shown = shown;
            return;
        }

        let mut acc = std::mem::take(&mut self.acc);
        acc.clear();
        acc.resize(cols * rows, Acc::default());
        for circle in &self.circles {
            stamp(&mut acc, (cols, rows), circle, self.pulse[circle.class as usize], shows, &self.inside);
        }

        let level = |v: f32, steps: usize| ((v * steps as f32 + 0.5) as usize).min(steps);
        let lead = self.lead;
        let lifted = |class: u8| LEAD_LIFT * lead[class as usize] * shows;
        let floor = HEAT_REST * shows;
        for y in 0..rows {
            for x in 0..cols {
                let a = &acc[y * cols + x];
                let last = &mut shown[y * cols + x];
                let glyph = glyph_for(a.weight, last.glyph);
                last.glyph = glyph;
                let (mark, class, heat) = if glyph > 0 {
                    (GLYPHS[glyph as usize], a.class, floor + (1.0 - HEAT_REST) * (a.bright + OVERLAP * (a.weight - a.best).clamp(0.0, 1.0)) / BRIGHT_MAX + lifted(a.class))
                } else if a.dots != 0 {
                    let more = ((a.light - a.most) / 8.0).clamp(0.0, 1.0);
                    (braille(a.dots), a.dclass, floor + (1.0 - HEAT_REST) * (a.dbright + OVERLAP * more) / BRIGHT_MAX + lifted(a.dclass))
                } else {
                    (' ', 0, 0.0)
                };
                let step = if mark == ' ' { 0 } else { sticky(&mut last.level, level(heat.min(1.0), FG_LEVELS), FG_LEVELS) };
                if step > 0 {
                    put(area.x as usize + x, area.y as usize + y, mark, look.fg[class as usize][step], look.ground);
                } else {
                    last.level = 0;
                    put(area.x as usize + x, area.y as usize + y, ' ', look.ground, look.ground);
                }
            }
        }
        self.acc = acc;
        self.shown = shown;
    }

    #[cfg(test)]
    pub fn pulses(&self) -> [f32; CLASSES] {
        self.pulse
    }

    #[cfg(test)]
    pub fn tempo(&self) -> (f32, f32) {
        (self.tempo.bpm, self.pace)
    }

    #[cfg(test)]
    pub fn leading(&self) -> [bool; CLASSES] {
        self.leading
    }

    #[cfg(test)]
    pub fn leads(&self) -> [f32; CLASSES] {
        self.lead
    }

    #[cfg(test)]
    pub fn presence(&self) -> f32 {
        self.presence
    }

    #[cfg(test)]
    pub fn circles(&self) -> Vec<(usize, f32, f32, f32)> {
        self.circles.iter().map(|c| (c.class as usize, c.at[0], c.at[1], c.radius)).collect()
    }

    #[cfg(test)]
    pub fn weights(&self) -> Vec<(usize, f32)> {
        self.order.iter().map(|&k| (self.circles[k].class as usize, self.circles[k].weight)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn music(t: f32) -> f32 {
        let beat = t % 0.5;
        let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 70.0 / 18.0 * (1.0 - (-18.0 * beat).exp()))).sin();
        let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
        let chord: f32 = [220.0f32, 277.2, 329.6].iter().flat_map(|&f| (1..4).map(move |k| 0.05 / k as f32 * (TAU * f * k as f32 * t).sin())).sum();
        let vib = 880.0 + 6.0 * (TAU * 5.0 * t).sin();
        let lead = 0.05 * (TAU * vib * t).sin() + 0.03 * (TAU * 2.0 * vib * t).sin() + 0.02 * (TAU * 3.0 * vib * t).sin();
        let hat_t = t % 0.25;
        let hat = 0.18 * (-hat_t * 40.0).exp() * (noise(t) - noise(t - 1.0 / 48_000.0));
        (kick + bass + chord + lead + hat).clamp(-1.0, 1.0)
    }

    fn noise(t: f32) -> f32 {
        let n = (t * 48_000.0).round() as i64 as u64;
        hash01(99, n, 0) - 0.5
    }

    fn sine(hz: f32, rate: u32, t: f32) -> f32 {
        let n = (t as f64 * rate as f64).round();
        (std::f64::consts::TAU * hz as f64 * n / rate as f64).sin() as f32
    }

    #[allow(clippy::too_many_arguments)]
    fn run(b: &mut Bokeh, tap: &Tap, (columns, rows): (usize, u16), seed: u64, start: f32, secs: f32, signal: impl Fn(f32) -> f32, clock: &mut Instant) {
        let rate = tap.rate() as f32;
        let per = (rate / 30.0) as usize;
        for f in 0..(secs * 30.0) as usize {
            let mut w = tap.writer();
            for i in 0..per {
                w.push(signal(start + (f * per + i) as f32 / rate));
            }
            drop(w);
            *clock += Duration::from_millis(33);
            b.update(tap, columns, rows, seed, *clock);
        }
    }

    fn painted(b: &mut Bokeh, theme: usize, (columns, rows): (usize, u16)) -> Buffer {
        b.use_theme(theme, || crate::theme::THEMES[theme].bokeh_colors());
        let area = Rect::new(0, 0, columns as u16, rows);
        let mut buf = Buffer::empty(area);
        b.paint(&mut buf, area);
        buf
    }

    fn glyphs(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| if buf[(x, y)].symbol().is_empty() { " " } else { buf[(x, y)].symbol() }).collect::<String>()).collect()
    }

    fn dracula() -> usize {
        crate::theme::index_of("Dracula").unwrap()
    }

    fn tone_pulses(rate: u32, hz: f32) -> [f32; CLASSES] {
        let tap = Tap::new();
        tap.set_rate(rate);
        let mut b = Bokeh::new();
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut b, &tap, (85, 12), 3, 0.0, 1.5, |t| 0.3 * sine(hz, rate, t), &mut clock);
        b.pulses()
    }

    #[test]
    fn each_range_of_the_spectrum_moves_its_own_size_of_circle() {
        for rate in [22_050u32, 44_100, 48_000, 96_000] {
            let top = top_of_range(rate);
            for c in 0..CLASSES {
                let (lo, hi) = (BAND_EDGES[c], BAND_EDGES[c + 1].min(top));
                if hi < lo * 1.5 {
                    continue;
                }
                let hz = (lo * hi).sqrt();
                let p = tone_pulses(rate, hz);
                let best = (0..CLASSES).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap();
                assert_eq!(best, c, "{hz:.0} Hz at {rate} Hz lit size {best}, not {c}: {p:?}");
            }
        }
    }

    #[test]
    fn the_lower_the_sound_the_bigger_the_circle() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut b, &tap, (85, 12), 3, 0.0, 1.0, music, &mut clock);
        let (mut mean, mut n) = ([0.0f32; CLASSES], [0usize; CLASSES]);
        for (c, _, _, r) in b.circles() {
            mean[c] += r;
            n[c] += 1;
        }
        for c in 0..CLASSES {
            assert!(n[c] > 0, "no circles of size {c}");
            mean[c] /= n[c] as f32;
        }
        for c in 1..CLASSES {
            assert!(mean[c] < mean[c - 1], "size {c} is not smaller than size {}: {mean:?}", c - 1);
        }
        assert!(counts(200, 40).iter().sum::<usize>() > counts(85, 12).iter().sum::<usize>());
    }

    #[test]
    fn circles_throb_with_the_music_and_settle_after() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let bass = |level: f32| move |t: f32| level * (TAU * 60.0 * t).sin();
        run(&mut b, &tap, (85, 12), 3, 0.0, 3.0, bass(0.02), &mut clock);
        let quiet = b.pulses()[1];
        run(&mut b, &tap, (85, 12), 3, 3.0, 0.3, bass(0.6), &mut clock);
        let loud = b.pulses()[1];
        assert!(loud > quiet + 0.2, "{quiet} -> {loud}");
        run(&mut b, &tap, (85, 12), 3, 3.3, 2.0, bass(0.02), &mut clock);
        assert!(b.pulses()[1] < loud - 0.2, "still at {}", b.pulses()[1]);
    }

    #[test]
    fn only_the_three_characters_and_braille_dots_are_drawn() {
        let dot = |c: char| ('\u{2801}'..='\u{28FF}').contains(&c);
        let mut seen = std::collections::BTreeSet::new();
        for size in [(85usize, 12u16), (40, 6), (120, 30)] {
            let (tap, mut b) = (Tap::new(), Bokeh::new());
            let mut clock = Instant::now() + Duration::from_secs(5);
            run(&mut b, &tap, size, 3, 0.0, 3.0, |t| (music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            for line in glyphs(&painted(&mut b, dracula(), size)) {
                seen.extend(line.chars());
            }
        }
        assert!(seen.iter().all(|&c| " ◌○●".contains(c) || dot(c)), "{seen:?}");
        for c in ['◌', '○', '●'] {
            assert!(seen.contains(&c), "never drew {c}");
        }
        assert!(seen.iter().any(|&c| dot(c)), "never drew a dot");
    }

    #[test]
    fn small_circles_stay_outside_the_big_ones() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let size = (85usize, 12u16);
        let mut drawn = 0;
        for k in 0..40 {
            run(&mut b, &tap, size, 3, k as f32 * 0.25, 0.25, |t| (music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            let pic = painted(&mut b, dracula(), size);
            let mut inside = vec![false; size.0 * size.1 as usize];
            for c in b.circles.iter().filter(|c| (c.class as usize) < DOTTED) {
                mark_inside(&mut inside, (size.0, size.1 as usize), c);
            }
            for (n, cell) in pic.content.iter().enumerate() {
                if "◌○●".contains(cell.symbol()) {
                    drawn += 1;
                    assert!(!inside[n], "a character at cell {n} sits inside a big circle");
                }
            }
        }
        assert!(drawn > 200, "only {drawn} characters drawn: they should still show outside");
    }

    #[test]
    fn a_small_circle_is_one_character_in_one_cell() {
        for class in DOTTED..CLASSES {
            for at in [[10.3f32, 8.4f32], [10.0, 8.0], [10.99, 9.99], [10.5, 7.99]] {
                let mut acc = vec![Acc::default(); 40 * 16];
                let c = Circle { class: class as u8, at, radius: 3.0, vis: 1.0, feel: 0.8, weight: 0.9, ..Circle::default() };
                stamp(&mut acc, (40, 16), &c, 0.8, 1.0, &[]);
                assert_eq!(acc.iter().filter(|a| a.weight > 0.0).count(), 1, "size {class} at {at:?}");
            }
        }
    }

    #[test]
    fn a_character_beside_another_gives_way() {
        let mut b = Bokeh::new();
        (b.columns, b.rows) = (20, 6);
        let at = |x: f32, y: f32, rank: f32| Circle { class: 7, at: [x, y], rank, feel: 0.7, ..Circle::default() };
        b.circles = vec![at(5.5, 7.0, 0.1), at(6.5, 7.0, 0.2), at(6.5, 9.0, 0.3), at(10.5, 7.0, 0.4), at(7.5, 7.0, 0.5), at(5.5, 11.0, 0.6), at(7.5, 9.0, 0.7)];
        b.order = (0..b.circles.len()).collect();
        b.give_way(f32::INFINITY);
        let vis: Vec<f32> = b.circles.iter().map(|c| c.vis).collect();
        assert_eq!(vis, vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0]);
        b.circles[1].at = [14.5, 7.0];
        b.give_way(0.05);
        assert!(b.circles[1].vis > 0.1 && b.circles[1].vis < 0.9, "{}", b.circles[1].vis);
        b.circles[3].at = [5.5, 9.0];
        b.give_way(0.05);
        assert!(b.circles[3].vis < 0.5, "{}", b.circles[3].vis);
        b.give_way(f32::INFINITY);
        assert!((b.circles[1].vis - 1.0).abs() < 1.0e-6 && b.circles[3].vis.abs() < 1.0e-6, "{} {}", b.circles[1].vis, b.circles[3].vis);
        assert!(b.circles[1].weight > T_MID && b.circles[3].weight < 1.0e-6);
    }

    #[test]
    fn small_circles_never_travel_in_groups() {
        for (size, seed) in [((85usize, 12u16), 7u64), ((40, 6), 3), ((120, 30), 11)] {
            let (tap, mut b) = (Tap::new(), Bokeh::new());
            let mut clock = Instant::now() + Duration::from_secs(5);
            let signal = |t: f32| (music(t) * 2.2).clamp(-1.0, 1.0);
            run(&mut b, &tap, size, seed, 0.0, 2.0, signal, &mut clock);
            let (mut shown, mut touching) = (0usize, 0usize);
            for f in 0..120 {
                run(&mut b, &tap, size, seed, 2.0 + f as f32 / 30.0, 0.034, signal, &mut clock);
                let pic = painted(&mut b, dracula(), size);
                let on = |x: i32, y: i32| (0..size.0 as i32).contains(&x) && (0..size.1 as i32).contains(&y) && ['◌', '○', '●'].iter().any(|g| pic[(x as u16, y as u16)].symbol().starts_with(*g));
                for y in 0..size.1 as i32 {
                    for x in 0..size.0 as i32 {
                        if on(x, y) {
                            shown += 1;
                            let beside = (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && on(x + dx, y + dy)));
                            touching += (beside || on(x - 2, y) || on(x + 2, y)) as usize;
                        }
                    }
                }
            }
            let share = touching as f32 / shown.max(1) as f32;
            println!("{size:?}: {shown} characters shown over 120 frames, {:.2}% beside another", share * 100.0);
            assert!(shown > 120 * 8, "{size:?}: hardly any characters ({shown})");
            assert!(share < 0.06, "{size:?}: {:.1}% of the characters have another beside them", share * 100.0);
        }
    }

    #[test]
    fn the_characters_of_a_size_pulse_together_but_not_as_clones() {
        let kinds = |hz: f32, size: usize| {
            let (tap, mut b) = (Tap::new(), Bokeh::new());
            tap.set_rate(48_000);
            let mut clock = Instant::now() + Duration::from_secs(5);
            run(&mut b, &tap, (85, 12), 3, 0.0, 2.0, |t| 0.3 * sine(hz, 48_000, t), &mut clock);
            let mut seen = [0usize; 4];
            for (c, w) in b.weights() {
                if c == size {
                    seen[glyph_for(w, 0) as usize] += 1;
                }
            }
            seen
        };
        for size in DOTTED..CLASSES {
            let resting = kinds(center(1), size);
            let hitting = kinds(center(size), size);
            println!("size {size}: resting {resting:?}, hitting {hitting:?}  (none, speck, hollow, filled)");
            assert!(resting.iter().filter(|&&n| n > 0).count() >= 2, "size {size} at rest: {resting:?}");
            assert!(hitting.iter().filter(|&&n| n > 0).count() >= 2, "size {size} hitting: {hitting:?}");
            assert!(hitting[3] > resting[3] && hitting[3] + hitting[2] > resting[3] + resting[2], "size {size}: resting {resting:?}, hitting {hitting:?}");
        }
    }

    #[test]
    fn characters_swell_with_each_burst_and_fall_back() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now() + Duration::from_secs(5);
        let size = 8;
        let gated = |t: f32| (if t % 1.0 < 0.5 { 0.3 } else { 0.01 }) * sine(center(size), 48_000, t);
        run(&mut b, &tap, (85, 12), 3, 0.0, 2.0, gated, &mut clock);
        let mean_weight = |b: &Bokeh| {
            let (sum, n) = b.circles.iter().filter(|c| c.class as usize == size && c.vis > GONE).fold((0.0, 0), |(s, n), c| (s + c.weight / c.vis, n + 1));
            sum / n.max(1) as f32
        };
        for burst in 2..6 {
            let start = burst as f32;
            run(&mut b, &tap, (85, 12), 3, start, 0.4, gated, &mut clock);
            let loud = mean_weight(&b);
            run(&mut b, &tap, (85, 12), 3, start + 0.4, 0.6, gated, &mut clock);
            let quiet = mean_weight(&b);
            println!("burst {burst}: weight {quiet:.2} -> {loud:.2}");
            assert!(loud > quiet + 0.25, "burst {burst}: {quiet} -> {loud}");
        }
    }

    fn lit_dots(c: &Circle, pulse: f32) -> Vec<(usize, usize)> {
        let (cols, rows) = (40usize, 16usize);
        let mut acc = vec![Acc::default(); cols * rows];
        stamp(&mut acc, (cols, rows), c, pulse, 1.0, &[]);
        let mut out = Vec::new();
        for (k, a) in acc.iter().enumerate() {
            for &(across, down, bit) in &DOTS {
                if a.dots & bit != 0 {
                    out.push((2 * (k % cols) + across, 4 * (k / cols) + down));
                }
            }
        }
        out
    }

    #[test]
    fn a_disc_of_dots_is_round_a_ring_is_lit_from_inside_and_a_glow_thins_out() {
        for r in [1.6f32, 2.5, 3.9, 6.0, 8.5] {
            for at in [[20.0f32, 16.0f32], [20.3, 16.4], [20.55, 16.75]] {
                let make = |style| Circle { style, shape: [0.5, 0.5], radius: r, at, ..Circle::default() };
                let dist = |&(x, y): &(usize, usize)| (((x as f32 + 0.5) * DOT - at[0]).powi(2) + ((y as f32 + 0.5) * DOT - at[1]).powi(2)).sqrt();

                let solid = lit_dots(&make(Style::Solid), 0.0);
                let area = std::f32::consts::PI * (r / DOT).powi(2);
                assert!((solid.len() as f32 - area).abs() <= 0.1 * area + 2.0, "r {r}: {} dots for an area of {area:.0}", solid.len());
                let span = |f: fn(&(usize, usize)) -> usize| {
                    let (lo, hi) = (solid.iter().map(f).min().unwrap(), solid.iter().map(f).max().unwrap());
                    hi - lo + 1
                };
                let (wide, tall) = (span(|d| d.0), span(|d| d.1));
                assert!(wide.abs_diff(tall) <= 1, "r {r}: {wide} dots wide and {tall} tall");

                let ring = lit_dots(&make(Style::Ring), 0.0);
                let rim = (DOT + RING_MORE * 0.5).min(0.3 * r).max(DOT);
                let (on_rim, on_body) = (ring.iter().filter(|d| dist(d) > r - rim).count(), ring.iter().filter(|d| dist(d) <= r - rim).count());
                let (rim_of, body_of) = (solid.iter().filter(|d| dist(d) > r - rim).count(), solid.iter().filter(|d| dist(d) <= r - rim).count());
                assert_eq!(on_rim, rim_of, "r {r}: a gap in the ring");
                assert!((on_body as f32) < 0.65 * body_of as f32, "r {r}: a ring with {on_body} of {body_of} dots inside it");

                if r >= 3.0 {
                    let glow = lit_dots(&make(Style::Soft), 0.0);
                    let (near, far) = (glow.iter().filter(|d| dist(d) < 0.45 * r).count() as f32, glow.iter().filter(|d| dist(d) >= 0.45 * r).count() as f32);
                    let (near_of, far_of) = (solid.iter().filter(|d| dist(d) < 0.45 * r).count() as f32, solid.iter().filter(|d| dist(d) >= 0.45 * r).count() as f32);
                    assert!(near / near_of > 2.0 * far / far_of, "r {r}: {near}/{near_of} near, {far}/{far_of} far");
                }
            }
        }
    }

    #[test]
    fn the_big_circles_come_in_a_mix_of_styles_and_brightness() {
        for seed in [3u64, 11, 40] {
            let looks: Vec<_> = make_circles(85, 12, seed).iter().map(|c| (c.class as usize, c.style, c.alpha)).collect();
            let big: Vec<_> = looks.iter().filter(|l| l.0 < DOTTED).collect();
            for style in [Style::Ring, Style::Solid, Style::Soft] {
                assert!(big.iter().any(|l| l.1 == style), "seed {seed}: no {style:?}");
            }
            for c in 0..DOTTED {
                let of_size: Vec<_> = big.iter().filter(|l| l.0 == c).collect();
                assert!(of_size.windows(2).all(|w| w[0].1 != w[1].1), "seed {seed}: two alike in a row among size {c}");
            }
            let (dim, bright) = (big.iter().map(|l| l.2).fold(f32::MAX, f32::min), big.iter().map(|l| l.2).fold(0.0, f32::max));
            assert!(bright - dim > 0.2, "seed {seed}: brightness only from {dim} to {bright}");
        }
    }

    #[test]
    fn the_source_has_no_ugly_marks_no_colors_and_no_theme_names() {
        let source = include_str!("bokeh.rs");
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        for mark in ['#', '=', '+', '-', '*', '@'] {
            for banned in [format!("b'{mark}'"), format!("'{mark}'"), format!("\"{mark}\"")] {
                assert!(!code.contains(&banned), "the visualizer draws {banned}");
            }
        }
        for t in crate::theme::THEMES {
            assert!(!code.contains(&format!("\"{}\"", t.name)), "names the theme {}", t.name);
        }
        assert!(!code.contains("Color::Rgb") && !code.contains("Color::Indexed"), "a color made here");
        let hex_literal = |s: &str| s.match_indices("0x").any(|(i, _)| s[i + 2..].chars().take_while(|c| c.is_ascii_hexdigit() || *c == '_').filter(|c| *c != '_').count() == 6);
        assert!(!hex_literal(code), "a six-digit color literal");
    }

    #[test]
    fn it_fades_into_the_flat_color_when_the_sound_stops_and_returns_with_it() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let size = (85, 12);
        run(&mut b, &tap, size, 3, 0.0, 2.0, music, &mut clock);
        assert!(b.is_active() && b.presence() > 0.99);
        run(&mut b, &tap, size, 3, 2.0, 4.0, |_| 0.0, &mut clock);
        assert!(b.presence() == 0.0 && !b.is_active(), "presence {}", b.presence());
        let theme = dracula();
        let ground = crate::theme::THEMES[theme].bokeh_colors().at(0, 0.0);
        let pic = painted(&mut b, theme, size);
        assert!(pic.content.iter().all(|c| c.symbol() == " " && c.fg == ground && c.bg == ground), "not one flat color");
        run(&mut b, &tap, size, 3, 6.0, 0.6, music, &mut clock);
        assert!(b.presence() > 0.99 && b.is_active());
    }

    #[test]
    fn one_seed_gives_one_picture() {
        let shot = |seed: u64| {
            let (tap, mut b) = (Tap::new(), Bokeh::new());
            let mut clock = Instant::now() + Duration::from_secs(5);
            run(&mut b, &tap, (85, 12), seed, 0.0, 2.0, music, &mut clock);
            glyphs(&painted(&mut b, dracula(), (85, 12)))
        };
        assert_eq!(shot(5), shot(5));
        assert_ne!(shot(5), shot(6));
    }

    #[test]
    fn any_window_size_is_painted_edge_to_edge() {
        for size in [(1usize, 1u16), (3, 1), (20, 2), (85, 12), (200, 40), (60, 90)] {
            let (tap, mut b) = (Tap::new(), Bokeh::new());
            let mut clock = Instant::now() + Duration::from_secs(5);
            run(&mut b, &tap, size, 1, 0.0, 1.0, music, &mut clock);
            let pic = painted(&mut b, dracula(), size);
            assert!(pic.content.iter().all(|c| matches!(c.bg, Color::Rgb(..))), "{size:?}: a cell was left unpainted");
        }
    }

    #[test]
    fn a_frame_changes_only_a_small_part_of_the_picture() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let size = (85usize, 12u16);
        let signal = |t: f32| (music(t) * 2.2).clamp(-1.0, 1.0);
        run(&mut b, &tap, size, 7, 0.0, 2.0, signal, &mut clock);
        let mut before = painted(&mut b, dracula(), size);
        let (mut changed, mut total) = (0usize, 0usize);
        for f in 0..90 {
            run(&mut b, &tap, size, 7, 2.0 + f as f32 / 30.0, 0.034, signal, &mut clock);
            let after = painted(&mut b, dracula(), size);
            for y in 0..size.1 {
                for x in 0..size.0 as u16 {
                    changed += (before[(x, y)] != after[(x, y)]) as usize;
                    total += 1;
                }
            }
            before = after;
        }
        let share = changed as f32 / total as f32;
        println!("cells changed per frame: {:.1}%", share * 100.0);
        assert!(share < 0.25, "{:.1}% of the cells changed per frame", share * 100.0);
    }

    fn center(c: usize) -> f32 {
        (BAND_EDGES[c] * BAND_EDGES[c + 1].min(top_of_range(48_000))).sqrt()
    }

    fn tones_lead(tones: &[(f32, f32)]) -> ([bool; CLASSES], [f32; CLASSES]) {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut b = Bokeh::new();
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut b, &tap, (85, 12), 3, 0.0, 1.5, |t| tones.iter().map(|&(hz, level)| level * sine(hz, 48_000, t)).sum(), &mut clock);
        println!("tones {tones:?}: targets {:?}", b.bands.target.map(|v| (v * 100.0).round() / 100.0));
        (b.leading(), b.leads())
    }

    #[test]
    fn the_three_hardest_hitting_ranges_wear_the_brighter_shade() {
        for loud in [[0usize, 1, 2], [1, 4, 7], [7, 8, 9], [2, 5, 8], [3, 6, 9]] {
            let tones: Vec<(f32, f32)> = (0..CLASSES).map(|c| (center(c), if loud.contains(&c) { 0.12 } else { 0.004 })).collect();
            let (leading, lead) = tones_lead(&tones);
            for c in 0..CLASSES {
                assert_eq!(leading[c], loud.contains(&c), "loud {loud:?}: {leading:?}");
                assert!(if leading[c] { lead[c] > 0.9 } else { lead[c] < 0.1 }, "loud {loud:?}: {lead:?}");
            }
        }
    }

    #[test]
    fn a_range_with_nothing_in_it_never_leads() {
        for c in 1..CLASSES {
            let (leading, _) = tones_lead(&[(center(c), 0.3)]);
            assert!(leading[c] && (0..CLASSES).all(|d| d.abs_diff(c) < 2 || !leading[d]), "a tone in range {c}: {leading:?}");
        }
        let (leading, lead) = tones_lead(&[]);
        assert!(leading.iter().all(|&l| !l) && lead.iter().all(|&l| l < 0.01), "silence: {leading:?} {lead:?}");
    }

    #[test]
    fn the_shade_follows_the_music() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now() + Duration::from_secs(5);
        let tone = |hz: f32| move |t: f32| 0.3 * sine(hz, 48_000, t);
        run(&mut b, &tap, (85, 12), 3, 0.0, 1.5, tone(center(1)), &mut clock);
        assert!(b.leading()[1] && b.leads()[1] > 0.9, "{:?}", b.leads());
        run(&mut b, &tap, (85, 12), 3, 1.5, 0.6, tone(center(5)), &mut clock);
        assert!(b.leading()[5] && !b.leading()[1], "{:?}", b.leading());
        assert!(b.leads()[5] > 0.9 && b.leads()[1] < 0.2, "{:?}", b.leads());
    }

    #[test]
    fn two_sizes_neck_and_neck_do_not_trade_the_shade() {
        let mut b = Bokeh::new();
        b.bands.target = [0.9, 0.8, 0.5, 0.5, 0.4, 0.3, 0.2, 0.05, 0.05, 0.05];
        let (mut changes, mut last) = (0, None);
        for f in 0..120 {
            let hair = if f % 2 == 0 { 0.02 } else { -0.02 };
            b.pulse = [0.9, 0.8, 0.5 + hair, 0.5 - hair, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1];
            b.choose_leaders();
            changes += last.is_some_and(|l| l != b.leading) as usize;
            last = Some(b.leading);
        }
        assert_eq!(changes, 0, "the shade changed hands {changes} times");
        b.pulse = [0.9, 0.8, 0.3, 0.7, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1];
        b.choose_leaders();
        assert!(b.leading[3] && !b.leading[2], "{:?}", b.leading);
        b.bands.target[3] = 0.0;
        b.choose_leaders();
        assert!(!b.leading[3] && b.leading[2] && b.leading.iter().filter(|&&l| l).count() == LEADERS, "{:?}", b.leading);
    }

    #[test]
    fn a_leading_size_wears_a_brighter_shade_of_its_own_color_in_any_theme() {
        let size = (85usize, 12u16);
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut b, &tap, size, 7, 0.0, 2.0, |t| (music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
        let lead = b.leads();
        assert!(lead.iter().any(|&l| l > 0.9) && (1..=LEADERS).contains(&b.leading().iter().filter(|&&l| l).count()), "{lead:?} {:?}", b.leading());
        let mut tops = std::collections::BTreeSet::new();
        for theme in [dracula(), 0, 90, 180, 270, 360, 450, 540, 630, 720, 810, 900] {
            b.lead = lead;
            let with = painted(&mut b, theme, size);
            b.lead = [0.0; CLASSES];
            let without = painted(&mut b, theme, size);
            let colors = crate::theme::THEMES[theme].bokeh_colors();
            let ramps: Vec<Vec<Color>> = (0..CLASSES).map(|c| (0..=FG_LEVELS).map(|i| colors.at(c, i as f32 / FG_LEVELS as f32)).collect()).collect();
            tops.insert(format!("{:?}", colors.at(3, 1.0)));
            let (mut brighter, mut wrong) = (0, 0);
            for y in 0..size.1 {
                for x in 0..size.0 as u16 {
                    let (a, o) = (&with[(x, y)], &without[(x, y)]);
                    assert_eq!(a.symbol(), o.symbol(), "the shade changed what is drawn");
                    if a.fg == o.fg {
                        continue;
                    }
                    match (0..CLASSES).find(|&c| ramps[c].contains(&a.fg) && ramps[c].contains(&o.fg)) {
                        Some(c) if ramps[c].iter().position(|&k| k == a.fg) > ramps[c].iter().position(|&k| k == o.fg) => brighter += 1,
                        _ => wrong += 1,
                    }
                }
            }
            println!("{}: {brighter} cells brighter, {wrong} not", crate::theme::THEMES[theme].name);
            assert!(wrong == 0 && brighter > 0, "{}: {brighter} cells brighter, {wrong} not", crate::theme::THEMES[theme].name);
        }
        assert!(tops.len() >= 9, "the themes share their colors: {tops:?}");
    }

    #[test]
    fn each_size_goes_clearly_darker_and_brighter_with_its_own_range() {
        let steps = FG_LEVELS as f32;
        for theme in [dracula(), 90, 450, 810] {
            for (c, [rest, hit, led]) in swings(theme).into_iter().enumerate() {
                let what = format!("{} size {c}: resting {rest:?}, hitting {hit:?}, leading {led:?}", crate::theme::THEMES[theme].name);
                assert!(rest.2 >= 3 && hit.2 >= 3 && led.2 >= 3, "{what}: hardly any cells to tell by");
                assert!(rest.0 <= 0.30 * steps, "{what}: not dark enough while its range is empty");
                assert!(hit.0 - rest.0 >= 0.30 * steps, "{what}: not bright enough while its range hits");
                assert!(led.0 - hit.0 >= 0.15 * steps, "{what}: the brighter shade of a leader does not show");
            }
        }
    }

    #[test]
    fn the_choice_is_saved_by_name() {
        use crate::settings::Visual;
        assert_eq!(Visual::from_name(Visual::Bokeh.name()), Visual::Bokeh);
        assert_ne!(Visual::from_name("bokeh"), Visual::from_name("trace"));
    }

    fn steps_of(theme: usize) -> std::collections::HashMap<Color, (usize, usize)> {
        let colors = crate::theme::THEMES[theme].bokeh_colors();
        let mut seen: std::collections::HashMap<Color, Vec<(usize, usize)>> = Default::default();
        for c in 0..CLASSES {
            for i in 1..=FG_LEVELS {
                seen.entry(colors.at(c, i as f32 / FG_LEVELS as f32)).or_default().push((c, i));
            }
        }
        seen.into_iter().filter(|(_, v)| v.len() == 1).map(|(k, v)| (k, v[0])).collect()
    }

    fn steps_by_size(buf: &Buffer, lookup: &std::collections::HashMap<Color, (usize, usize)>) -> [(usize, usize, usize); CLASSES] {
        let mut out = [(0, 0, 0); CLASSES];
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(&(c, step)) = lookup.get(&buf[(x, y)].fg) {
                    out[c] = (out[c].0 + step, out[c].1 + 1, out[c].2.max(step));
                }
            }
        }
        out
    }

    fn steps_with_tone(theme: usize, hz: f32, shade: bool) -> [(f32, usize, usize); CLASSES] {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut b, &tap, (85, 12), 3, 0.0, 2.0, |t| 0.3 * sine(hz, 48_000, t), &mut clock);
        if !shade {
            b.lead = [0.0; CLASSES];
        }
        let steps = steps_by_size(&painted(&mut b, theme, (85, 12)), &steps_of(theme));
        std::array::from_fn(|c| (steps[c].0 as f32 / steps[c].1.max(1) as f32, steps[c].2, steps[c].1))
    }

    fn swings(theme: usize) -> Vec<[(f32, usize, usize); 3]> {
        let low = steps_with_tone(theme, center(1), false);
        let high = steps_with_tone(theme, center(8), false);
        (0..CLASSES)
            .map(|c| [if c >= 5 { low[c] } else { high[c] }, steps_with_tone(theme, center(c), false)[c], steps_with_tone(theme, center(c), true)[c]])
            .collect()
    }

    #[test]
    #[ignore]
    fn bokeh_swing_table() {
        let names = std::env::var("BOKEH_THEMES").unwrap_or_else(|_| "Dracula".to_string());
        for name in names.split(',').map(str::trim) {
            let Some(theme) = crate::theme::index_of(name) else { println!("no theme {name}"); continue };
            println!("== {name}: color step of {FG_LEVELS} of the cells of each size (mean, highest, cells)");
            for (c, [rest, hit, led]) in swings(theme).into_iter().enumerate() {
                println!(
                    "size {c} ({:>6.0} Hz): resting {:>4.1} ({:>2}, {:>3}) -> hitting {:>4.1} ({:>2}, {:>3})  swing {:+.1}  | as a leader {:>4.1} ({:>2}, {:>3})",
                    center(c), rest.0, rest.1, rest.2, hit.0, hit.1, hit.2, hit.0 - rest.0, led.0, led.1, led.2
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn bokeh_song_stats() {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let bytes = fs::read(env("BOKEH_AUDIO", "")).unwrap();
        let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let theme = crate::theme::index_of(env("BOKEH_THEMES", "Dracula").split(',').next().unwrap().trim()).unwrap();
        let lookup = steps_of(theme);
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now() + Duration::from_secs(5);
        let per = 48_000 / 30;
        let (mut pulse, mut step): (Vec<Vec<f32>>, Vec<Vec<f32>>) = (vec![vec![]; CLASSES], vec![vec![]; CLASSES]);
        for f in 0..audio.len() / per {
            let mut w = tap.writer();
            for &s in &audio[f * per..(f + 1) * per] {
                w.push(s);
            }
            drop(w);
            clock += Duration::from_secs_f64(1.0 / 30.0);
            b.update(&tap, 85, 12, 7, clock);
            let steps = steps_by_size(&painted(&mut b, theme, (85, 12)), &lookup);
            for c in 0..CLASSES {
                pulse[c].push(b.pulses()[c] * 100.0);
                if steps[c].1 > 0 {
                    step[c].push(steps[c].0 as f32 / steps[c].1 as f32);
                }
            }
        }
        let at = |v: &mut Vec<f32>, q: f32| {
            v.sort_by(f32::total_cmp);
            if v.is_empty() { 0.0 } else { v[((v.len() - 1) as f32 * q) as usize] }
        };
        println!("size |  pulse %: p5 p25 p50 p75 p95 |  color step of {FG_LEVELS}: p5 p25 p50 p75 p95");
        for c in 0..CLASSES {
            let p: Vec<String> = [0.05, 0.25, 0.5, 0.75, 0.95].iter().map(|&q| format!("{:>3.0}", at(&mut pulse[c], q))).collect();
            let s: Vec<String> = [0.05, 0.25, 0.5, 0.75, 0.95].iter().map(|&q| format!("{:>4.1}", at(&mut step[c], q))).collect();
            println!("{c:>4} |  {} |  {}", p.join(" "), s.join(" "));
        }
    }

    #[test]
    #[ignore]
    fn bokeh_tone_table() {
        let rate: u32 = std::env::var("BOKEH_RATE").ok().and_then(|v| v.parse().ok()).unwrap_or(48_000);
        let top = top_of_range(rate);
        println!("sample rate {rate} Hz: each size listens to");
        for c in 0..CLASSES {
            let (lo, hi) = (BAND_EDGES[c], BAND_EDGES[c + 1].min(top));
            if hi > lo {
                println!("  size {c}: {lo:.0} to {hi:.0} Hz");
            } else {
                println!("  size {c}: beyond what this rate carries");
            }
        }
        println!("{:>7} Hz | pulse of each size, biggest first, in percent | leads", "tone");
        for hz in [30.0f32, 45.0, 90.0, 180.0, 350.0, 700.0, 1400.0, 2800.0, 5600.0, 11000.0, 17000.0] {
            if hz > top {
                continue;
            }
            let p = tone_pulses(rate, hz);
            let lead = (0..CLASSES).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap();
            let row: Vec<String> = p.iter().map(|v| format!("{:>3.0}", v * 100.0)).collect();
            println!("{hz:>7.0}    | {} | size {lead}", row.join(" "));
        }
    }

    #[test]
    #[ignore]
    fn bokeh_ramps_dump() {
        let names = std::env::var("BOKEH_THEMES").unwrap_or_else(|_| "Dracula".to_string());
        let hsv = |c: u32| {
            let (r, g, b) = (((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0);
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            let d = hi - lo;
            let h = if d <= 0.0 { 0.0 } else if hi == r { 60.0 * ((g - b) / d).rem_euclid(6.0) } else if hi == g { 60.0 * ((b - r) / d + 2.0) } else { 60.0 * ((r - g) / d + 4.0) };
            format!("h{:>3.0} s{:>3.0} v{:>3.0}", h, if hi > 0.0 { 100.0 * d / hi } else { 0.0 }, 100.0 * hi)
        };
        for name in names.split(',').map(str::trim) {
            let Some(theme) = crate::theme::index_of(name) else { println!("no theme {name}"); continue };
            let colors = crate::theme::THEMES[theme].bokeh_colors();
            println!("== {name}");
            for (c, ramp) in colors.ramps.iter().enumerate() {
                let hex: Vec<String> = ramp.iter().map(|v| format!("{v:06x}")).collect();
                println!("size {c}: {}   color {}  ->  hottest {}", hex.join(" "), hsv(ramp[2]), hsv(ramp[3]));
            }
        }
    }

    #[test]
    #[ignore]
    fn bokeh_text_dump() {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let size = (env("BOKEH_COLS", "85").parse::<usize>().unwrap(), env("BOKEH_ROWS", "12").parse::<u16>().unwrap());
        let seed: u64 = env("BOKEH_SEED", "7").parse().unwrap();
        let moments: Vec<f32> = env("BOKEH_AT", "4.0").split(',').map(|s| s.trim().parse().unwrap()).collect();
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let mut now = 0.0f32;
        for m in moments {
            run(&mut b, &tap, size, seed, now, m - now, |t| (music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            now = m;
            println!("--- {m:.2} s   pulses {:?}", b.pulses().map(|p| (p * 100.0).round() / 100.0));
            for line in glyphs(&painted(&mut b, dracula(), size)) {
                println!("|{line}|");
            }
        }
    }

    #[test]
    #[ignore]
    fn bokeh_class_chart() {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let (columns, rows) = (env("BOKEH_COLS", "88").parse::<usize>().unwrap(), env("BOKEH_ROWS", "12").parse::<u16>().unwrap());
        let out = env("BOKEH_OUT", "/tmp/bokeh_chart");
        fs::create_dir_all(&out).unwrap();
        let pulses: Vec<f32> = env("BOKEH_PULSES", "0,0.3,0.6,1").split(',').map(|s| s.trim().parse().unwrap()).collect();
        let names: Vec<String> = env("BOKEH_THEMES", "Dracula").split(',').map(|s| s.trim().to_string()).collect();
        let styles: Vec<String> = env("BOKEH_STYLES", "ring,solid,soft").split(',').map(|s| s.trim().to_string()).collect();
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            other => panic!("not a plain color: {other:?}"),
        };
        let mut centers = [0.0f32; CLASSES];
        let mut left = 1.0f32;
        for (c, center) in centers.iter_mut().enumerate() {
            let half = (RADIUS[c] * scale_for(rows) * (1.0 + GROW)).max(0.8);
            *center = left + half;
            left += 2.0 * half + 1.5;
        }
        for name in &names {
            let theme = crate::theme::index_of(name).unwrap();
            for style in &styles {
                let lit = match style.as_str() {
                    "ring" => Style::Ring,
                    "solid" => Style::Solid,
                    "soft" => Style::Soft,
                    other => panic!("no style called {other}"),
                };
                for &p in &pulses {
                    let mut b = Bokeh::new();
                    b.columns = columns;
                    b.rows = rows;
                    b.made_for = Some((columns, rows, 0));
                    b.presence = 1.0;
                    b.silent = false;
                    b.pulse = [p; CLASSES];
                    let s = scale_for(rows);
                    b.circles = (0..CLASSES)
                        .map(|c| Circle {
                            class: c as u8,
                            size: 1.0,
                            alpha: 1.0,
                            style: lit,
                            shape: [0.5, 0.5],
                            at: [centers[c], rows as f32],
                            radius: (RADIUS[c] * s * (1.0 + GROW * p)).max(0.8),
                            vis: 1.0,
                            feel: p,
                            weight: p + WEIGHT_LIFT + WEIGHT_BIAS[c],
                            ..Circle::default()
                        })
                        .collect();
                    let pic = painted(&mut b, theme, (columns, rows));
                    let mut text = format!("{columns} {rows}\n");
                    for y in 0..rows {
                        for x in 0..columns as u16 {
                            let c = &pic[(x, y)];
                            text.push_str(if c.symbol().is_empty() { " " } else { c.symbol() });
                            text.push_str(&hex(c.fg));
                            text.push_str(&hex(c.bg));
                        }
                        text.push('\n');
                    }
                    fs::write(format!("{out}/{}_{style}_pulse_{:.2}.cells", name.replace(' ', "_"), p), text).unwrap();
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn bokeh_cells_dump() {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let (columns, rows) = (env("BOKEH_COLS", "85").parse::<usize>().unwrap(), env("BOKEH_ROWS", "12").parse::<u16>().unwrap());
        let seed: u64 = env("BOKEH_SEED", "7").parse().unwrap();
        let rate: u32 = env("BOKEH_RATE", "48000").parse().unwrap();
        let out = env("BOKEH_OUT", "/tmp/bokeh_cells");
        fs::create_dir_all(&out).unwrap();
        let bytes = fs::read(env("BOKEH_AUDIO", "")).unwrap();
        let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let names: Vec<String> = env("BOKEH_THEMES", "Dracula").split(',').map(|s| s.trim().to_string()).collect();
        let moments: Vec<f32> = env("BOKEH_AT", "5.0").split(',').map(|s| s.trim().parse().unwrap()).collect();
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            other => panic!("not a plain color: {other:?}"),
        };
        let text = |pic: &Buffer| {
            let mut text = format!("{columns} {rows}\n");
            for y in 0..rows {
                for x in 0..columns as u16 {
                    let c = &pic[(x, y)];
                    text.push_str(if c.symbol().is_empty() { " " } else { c.symbol() });
                    text.push_str(&hex(c.fg));
                    text.push_str(&hex(c.bg));
                }
                text.push('\n');
            }
            text
        };

        let tap = Tap::new();
        tap.set_rate(rate);
        let mut b = Bokeh::new();
        let mut clock = Instant::now() + Duration::from_secs(5);
        let per = (rate / 30) as usize;
        for f in 0..audio.len() / per {
            let mut w = tap.writer();
            for &s in &audio[f * per..(f + 1) * per] {
                w.push(s);
            }
            drop(w);
            clock += Duration::from_secs_f64(1.0 / 30.0);
            b.update(&tap, columns, rows, seed, clock);
            if std::env::var("BOKEH_NO_LEAD").is_ok() {
                b.lead = [0.0; CLASSES];
            }
            let first = crate::theme::index_of(&names[0]).unwrap();
            let _ = painted(&mut b, first, (columns, rows));
            let now = (f + 1) as f32 / 30.0;
            if let Ok(each) = std::env::var("BOKEH_CYCLE") {
                let index = (f as f32 / 30.0 / each.parse::<f32>().unwrap()) as usize % names.len();
                let pic = painted(&mut b, crate::theme::index_of(&names[index]).unwrap(), (columns, rows));
                fs::write(format!("{out}/frame_{f:05}.cells"), text(&pic)).unwrap();
                let mut caption = fs::OpenOptions::new().create(true).append(true).open(format!("{out}/captions.txt")).unwrap();
                const RANGE: [&str; CLASSES] = ["20 Hz", "60 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"];
                let hitting: Vec<&str> = (0..CLASSES).filter(|&c| b.leading[c]).map(|c| RANGE[c]).collect();
                std::io::Write::write_all(&mut caption, format!("{}  {:.1}s  brighter: {}\n", names[index], now, if hitting.is_empty() { "none".to_string() } else { hitting.join(", ") }).as_bytes()).unwrap();
                continue;
            }
            for &m in &moments {
                if (now - m).abs() < 0.5 / 30.0 {
                    const RANGE: [&str; CLASSES] = ["20 Hz", "60 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"];
                    println!("at {m:.2}s the sizes hitting hardest: {:?}", (0..CLASSES).filter(|&c| b.leading[c]).map(|c| RANGE[c]).collect::<Vec<_>>());
                    for name in &names {
                        let Some(index) = crate::theme::index_of(name) else { panic!("no theme called {name}") };
                        let pic = painted(&mut b, index, (columns, rows));
                        fs::write(format!("{out}/{}_{:.2}s.cells", name.replace(' ', "_"), m), text(&pic)).unwrap();
                    }
                }
            }
        }
    }

    fn at_tempo(bpm: f32) -> impl Fn(f32) -> f32 {
        move |t| {
            let beat = t % (60.0 / bpm);
            let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 70.0 / 18.0 * (1.0 - (-18.0 * beat).exp()))).sin();
            let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
            (kick + bass).clamp(-1.0, 1.0)
        }
    }

    fn tempo_after(bpm: f32, secs: f32) -> (f32, f32) {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut b = Bokeh::new();
        let mut clock = Instant::now();
        run(&mut b, &tap, (85, 12), 7, 0.0, secs, at_tempo(bpm), &mut clock);
        b.tempo()
    }

    #[test]
    fn the_tempo_of_the_song_is_found() {
        for bpm in [90.0, 120.0, 150.0] {
            let (found, _) = tempo_after(bpm, 14.0);
            assert!((found - bpm).abs() <= 0.06 * bpm, "{bpm} bpm read as {found}");
        }
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut b = Bokeh::new();
        let mut clock = Instant::now();
        run(&mut b, &tap, (85, 12), 7, 0.0, 14.0, music, &mut clock);
        let (found, _) = b.tempo();
        assert!((found - 120.0).abs() <= 7.0, "the demo song read as {found}");
    }

    #[test]
    fn each_size_holds_its_place_and_sways_as_one() {
        let (tap, mut b) = (Tap::new(), Bokeh::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now() + Duration::from_secs(5);
        let signal = |t: f32| (music(t) * 2.2).clamp(-1.0, 1.0);
        run(&mut b, &tap, (85, 12), 7, 0.0, 1.0, signal, &mut clock);
        let start: Vec<[f32; 2]> = b.circles.iter().map(|c| c.at).collect();
        let mut furthest = [0.0f32; 2];
        for f in 0..300 {
            run(&mut b, &tap, (85, 12), 7, 1.0 + f as f32 / 30.0, 0.034, signal, &mut clock);
            for class in 0..CLASSES {
                let moved: Vec<[f32; 2]> = b
                    .circles
                    .iter()
                    .zip(&start)
                    .filter(|(c, _)| c.class as usize == class)
                    .filter(|(c, _)| c.at[0] > 0.0 && c.at[0] < 85.0 && c.at[1] > 0.0 && c.at[1] < 24.0)
                    .map(|(c, s)| [c.at[0] - s[0], c.at[1] - s[1]])
                    .collect();
                for m in &moved {
                    if class >= DOTTED {
                        assert_eq!(*m, [0.0, 0.0], "a glyph moved");
                    } else {
                        assert!((m[0] - moved[0][0]).abs() < 1.0e-3 && (m[1] - moved[0][1]).abs() < 1.0e-3, "size {class} isn't moving as one");
                        furthest = [furthest[0].max(m[0].abs()), furthest[1].max(m[1].abs())];
                    }
                }
            }
        }
        println!("furthest a big circle went from its start over 10 s: {furthest:?} (columns, half-rows)");
        assert!(furthest[0] > 0.3 && furthest[0] <= 2.0 * SWAY[0] + 1.0e-3, "{furthest:?}");
        assert!(furthest[1] > 0.2 && furthest[1] <= 2.0 * SWAY[1] + 1.0e-3, "{furthest:?}");
    }

    #[test]
    fn a_faster_song_moves_the_circles_faster() {
        let (_, slow) = tempo_after(90.0, 20.0);
        let (_, mid) = tempo_after(120.0, 20.0);
        let (_, fast) = tempo_after(150.0, 20.0);
        assert!(slow < mid && mid < fast, "{slow} {mid} {fast}");
        assert!((mid - 1.0).abs() < 0.1, "120 bpm should keep the usual speed ({mid})");
        assert!(fast / slow > 1.4, "{fast} against {slow}");
    }

    #[test]
    fn without_a_beat_the_speed_stays_usual() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut b = Bokeh::new();
        let mut clock = Instant::now();
        run(&mut b, &tap, (85, 12), 7, 0.0, 10.0, |t| 0.3 * sine(220.0, 48_000, t), &mut clock);
        assert!((b.tempo().1 - 1.0).abs() < 0.05, "a steady tone changed the speed ({})", b.tempo().1);
    }
}
