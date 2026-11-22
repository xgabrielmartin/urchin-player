use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::spectrum::{top_frequency, Tap, FFT_SIZE};
use crate::theme::TraceColors;

const F_LO: f32 = 30.0;
const TILT_HI: f32 = 3.5;
const TILT_LO: f32 = 2.0;
const SPAN_DB: f32 = 54.0;
const ABS_CEIL: f32 = -16.0;
const ADAPT: f32 = 0.7;
const HEADROOM: f32 = 6.0;
const ENV_GAMMA: f32 = 1.5;
const LEVEL_ATTACK: f32 = 2.0;
const LEVEL_RELEASE: f32 = 10.0;
const FLUX_HISTORY: usize = 30;
const REFRACTORY: f32 = 0.09;
const STALE: Duration = Duration::from_millis(200);
const SILENCE_PEAK: f32 = 1.0e-4;
const SILENCE_TAIL: usize = 1024;
const SILENCE_HOLD: f32 = 0.12;
const FADE_OUT: f32 = 2.5;
const FADE_IN: f32 = 0.35;
const MIN_STEP: f32 = 0.028;

const REGIONS: [(f32, f32); 3] = [(0.0, 0.30), (0.25, 0.72), (0.62, 1.0)];

struct Features {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    columns: usize,
    rate: u32,
    k0: Vec<usize>,
    k1: Vec<usize>,
    center_bin: Vec<f32>,
    tilt: Vec<f32>,
    regions: [(usize, usize); 3],
    db: Vec<f32>,
    prev_db: Vec<f32>,
    have_prev: bool,
    rise: Vec<f32>,
    norm: Vec<f32>,
    sorted: Vec<f32>,
    target: Vec<f32>,
    onset: [f32; 3],
    level: f32,
    history: [[f32; FLUX_HISTORY]; 3],
    history_len: [usize; 3],
    history_pos: [usize; 3],
    last_onset: [f32; 3],
    time: f32,
}

impl Features {
    fn new() -> Features {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Features {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; FFT_SIZE / 2 + 1],
            fft,
            window,
            columns: 0,
            rate: 0,
            k0: Vec::new(),
            k1: Vec::new(),
            center_bin: Vec::new(),
            tilt: Vec::new(),
            regions: [(0, 0); 3],
            db: Vec::new(),
            prev_db: Vec::new(),
            have_prev: false,
            rise: Vec::new(),
            norm: Vec::new(),
            sorted: Vec::new(),
            target: Vec::new(),
            onset: [0.0; 3],
            level: -30.0,
            history: [[0.0; FLUX_HISTORY]; 3],
            history_len: [0; 3],
            history_pos: [0; 3],
            last_onset: [-1.0; 3],
            time: 0.0,
        }
    }

    fn layout(&mut self, columns: usize, rate: u32) {
        if columns == self.columns && rate == self.rate {
            return;
        }
        let old = std::mem::take(&mut self.target);
        self.columns = columns;
        self.rate = rate;
        let df = rate as f32 / FFT_SIZE as f32;
        let hi = top_frequency(rate);
        let ratio = (hi / F_LO).powf(1.0 / columns as f32);
        self.k0.clear();
        self.k1.clear();
        self.center_bin.clear();
        self.tilt.clear();
        let bins = FFT_SIZE / 2 + 1;
        for b in 0..columns {
            let lo = F_LO * ratio.powi(b as i32);
            let up = lo * ratio;
            let center = (lo * up).sqrt();
            self.k0.push((lo / df).ceil() as usize);
            self.k1.push(((up / df).ceil() as usize).min(bins));
            self.center_bin.push(center / df);
            let octaves = (center / 1000.0).log2();
            self.tilt.push(if octaves > 0.0 { TILT_HI * octaves } else { TILT_LO * octaves });
        }
        self.regions = REGIONS.map(|(a, b)| ((a * columns as f32) as usize, ((b * columns as f32) as usize).min(columns)));
        for v in [&mut self.db, &mut self.prev_db, &mut self.rise, &mut self.norm, &mut self.sorted] {
            *v = vec![0.0; columns];
        }
        self.target = (0..columns).map(|i| if old.is_empty() { 0.0 } else { old[i * old.len() / columns] }).collect();
        self.have_prev = false;
    }

    fn update(&mut self, tap: &Tap, dt: f32, stale: bool) -> bool {
        self.time += dt;
        let n = self.columns;
        if n == 0 {
            return true;
        }
        if !stale {
            tap.latest(&mut self.input);
        }
        let quiet = stale || self.input[FFT_SIZE - SILENCE_TAIL..].iter().all(|x| x.abs() < SILENCE_PEAK);
        if quiet {
            self.onset = [0.0; 3];
            self.have_prev = false;
            return true;
        }

        for (x, w) in self.input.iter_mut().zip(&self.window) {
            *x *= w;
        }
        if self.fft.process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch).is_err() {
            return false;
        }
        for (p, c) in self.power.iter_mut().zip(&self.spectrum) {
            *p = c.norm_sqr();
        }

        let scale = 1.0 / (FFT_SIZE as f32 * 0.25);
        let last = self.power.len() - 2;
        for b in 0..n {
            let amplitude = if self.k1[b] > self.k0[b] {
                self.power[self.k0[b]..self.k1[b]].iter().fold(0.0f32, |m, &p| m.max(p)).sqrt()
            } else {
                let x = self.center_bin[b];
                let k = (x.floor() as usize).min(last);
                let f = x - k as f32;
                self.power[k].sqrt() * (1.0 - f) + self.power[k + 1].sqrt() * f
            };
            self.db[b] = 20.0 * (amplitude * scale + 1e-7).log10() + self.tilt[b];
        }

        let top_n = (n / 12).max(4).min(n);
        self.sorted.copy_from_slice(&self.db);
        self.sorted.sort_unstable_by(|a, b| a.total_cmp(b));
        let top = self.sorted[n - top_n..].iter().sum::<f32>() / top_n as f32;
        let tau = if top > self.level { LEVEL_ATTACK } else { LEVEL_RELEASE };
        self.level += (top - self.level) * (1.0 - (-dt / tau).exp());
        let ceil = ADAPT * self.level.max(-45.0) + (1.0 - ADAPT) * ABS_CEIL + HEADROOM;
        let floor = ceil - SPAN_DB;

        for b in 0..n {
            self.norm[b] = ((self.db[b] - floor) / SPAN_DB).clamp(0.0, 1.0);
        }
        let at = |i: isize| self.norm[i.clamp(0, n as isize - 1) as usize];
        for b in 0..n {
            let i = b as isize;
            let env = (at(i - 2) + at(i - 1) + at(i) + at(i + 1) + at(i + 2)) / 5.0;
            self.target[b] = (0.65 * env + 0.35 * self.norm[b]).powf(ENV_GAMMA);
        }

        if !self.have_prev {
            for b in 0..n {
                self.prev_db[b] = self.db[b].max(floor);
            }
            self.have_prev = true;
        }
        for b in 0..n {
            let now = self.db[b].max(floor);
            self.rise[b] = (now - self.prev_db[b]).max(0.0);
            self.prev_db[b] = now;
        }
        self.onset = [0.0; 3];
        for k in 0..3 {
            let (a, b) = self.regions[k];
            let flux = if b > a { self.rise[a..b].iter().sum::<f32>() / (b - a) as f32 } else { 0.0 };
            let pos = self.history_pos[k];
            self.history[k][pos] = flux;
            self.history_pos[k] = (pos + 1) % FLUX_HISTORY;
            self.history_len[k] = (self.history_len[k] + 1).min(FLUX_HISTORY);
            let h = &self.history[k][..self.history_len[k]];
            let mean = h.iter().sum::<f32>() / h.len() as f32;
            let var = h.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / h.len() as f32;
            let threshold = mean + 1.4 * var.sqrt() + 0.9;
            if flux > threshold && self.time - self.last_onset[k] > REFRACTORY {
                self.onset[k] = 0.55 + 0.45 * ((flux - threshold) / (3.0 + threshold) * 2.0).min(1.0);
                self.last_onset[k] = self.time;
            }
        }
        false
    }
}

const FALL: f32 = 1.0;
const AMP_ATTACK: f32 = 0.04;
const GLOW_ATTACK: f32 = 0.08;
const GLOW_RELEASE: f32 = 0.45;
const FLARE_DECAY: f32 = 0.14;
const STREAK_HOLD: f32 = 0.12;
const STREAK_FALL: f32 = 0.5;

pub(crate) const BG_MIN: f32 = 0.08;
pub(crate) const BG_MAX: f32 = 0.20;
const SPREAD: f32 = 0.45;
const NEEDLE_DIM: f32 = 0.55;
const NEEDLE_HOT: f32 = 0.85;
const FG_LEVELS: usize = 12;
const BG_LEVELS: usize = 6;
const FLASH_STEPS: f32 = 4.0;

fn texture(columns: usize, seed: u64) -> Vec<f32> {
    (0..columns)
        .map(|i| {
            let mut z = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add((i as u64 + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9));
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 11) as f64 / (1u64 << 53) as f64).powf(1.4) as f32
        })
        .collect()
}

pub fn song_seed(size: u64, length_ms: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in size.to_le_bytes().into_iter().chain(length_ms.to_le_bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn pow06(x: f32) -> f32 {
    const N: usize = 32;
    static TABLE: std::sync::OnceLock<[f32; N + 1]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0.0; N + 1];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f32 / N as f32).powf(0.6);
        }
        t
    });
    let p = x.clamp(0.0, 1.0) * N as f32;
    let i = (p as usize).min(N - 1);
    t[i] + (t[i + 1] - t[i]) * (p - i as f32)
}

fn sticky(shown: &mut u8, want: usize, top: usize) -> usize {
    let have = *shown as usize;
    let now = if want == 0 || want == top || want.abs_diff(have) >= 2 { want } else { have };
    *shown = now as u8;
    now
}

struct Ramp {
    fg: [Color; FG_LEVELS + 1],
    bg: [Color; BG_LEVELS + 1],
}

impl Ramp {
    fn new(colors: &TraceColors) -> Ramp {
        Ramp {
            fg: std::array::from_fn(|i| colors.at(BG_MIN + (1.0 - BG_MIN) * i as f32 / FG_LEVELS as f32)),
            bg: std::array::from_fn(|i| colors.at(BG_MIN + (BG_MAX - BG_MIN) * i as f32 / BG_LEVELS as f32)),
        }
    }
}

fn fade_curve(presence: f32) -> f32 {
    let p = presence.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

#[derive(Clone, Copy, Default)]
struct Column {
    len_up: f32,
    len_dn: f32,
    heat_up: f32,
    heat_dn: f32,
    streak: f32,
    glow: f32,
    kick: f32,
}

const AT_REST: f32 = 0.004;

pub struct Trace {
    feats: Features,
    columns: usize,
    rows: u16,
    seed: u64,
    tex_up: Vec<f32>,
    tex_dn: Vec<f32>,
    weights: [Vec<f32>; 3],
    amp: Vec<f32>,
    glow: Vec<f32>,
    streak: Vec<f32>,
    hold: Vec<f32>,
    flare: [f32; 3],
    flash: f32,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    above: u16,
    cols: Vec<Column>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
    ramp: Option<(usize, Ramp)>,
    shown: Vec<u8>,
    lit: Vec<f32>,
}

impl Default for Trace {
    fn default() -> Self {
        Trace::new()
    }
}

impl Trace {
    pub fn new() -> Trace {
        Trace {
            feats: Features::new(),
            columns: 0,
            rows: 0,
            seed: 0,
            tex_up: Vec::new(),
            tex_dn: Vec::new(),
            weights: [Vec::new(), Vec::new(), Vec::new()],
            amp: Vec::new(),
            glow: Vec::new(),
            streak: Vec::new(),
            hold: Vec::new(),
            flare: [0.0; 3],
            flash: 0.0,
            last: None,
            last_written: 0,
            quiet_since: None,
            above: 0,
            cols: Vec::new(),
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
            ramp: None,
            shown: Vec::new(),
            lit: Vec::new(),
        }
    }

    pub fn use_theme(&mut self, key: usize, colors: impl FnOnce() -> TraceColors) {
        if self.ramp.as_ref().map(|r| r.0) != Some(key) {
            self.ramp = Some((key, Ramp::new(&colors())));
        }
    }

    pub fn is_active(&self) -> bool {
        let fading = if self.silent { self.presence > 0.0 } else { self.presence < 1.0 };
        fading || self.flash > AT_REST || self.amp.iter().chain(&self.glow).chain(&self.streak).any(|&v| v > AT_REST)
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        self.resize(columns.max(1), rows, tap.rate().max(1));
        if seed != self.seed || self.tex_up.len() != self.columns {
            self.set_seed(seed);
        }
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

        let quiet = self.feats.update(tap, dt, stale);
        self.quiet_for = if quiet { (self.quiet_for + dt).min(10.0) } else { 0.0 };
        self.silent = self.quiet_for >= SILENCE_HOLD;
        self.fade(dt);
        self.step(dt);
    }

    fn fade(&mut self, dt: f32) {
        if self.silent {
            self.presence = (self.presence - dt / FADE_OUT).max(0.0);
            if self.presence <= 0.0 {
                for v in [&mut self.amp, &mut self.glow, &mut self.streak, &mut self.hold] {
                    v.iter_mut().for_each(|x| *x = 0.0);
                }
                self.feats.target.iter_mut().for_each(|x| *x = 0.0);
                self.flare = [0.0; 3];
                self.flash = 0.0;
            }
        } else {
            self.presence = (self.presence + dt / FADE_IN).min(1.0);
        }
    }

    fn resize(&mut self, columns: usize, rows: u16, rate: u32) {
        self.feats.layout(columns, rate);
        if columns != self.columns {
            let old = self.columns.max(1);
            let stretch = |v: &mut Vec<f32>| {
                let was = std::mem::take(v);
                *v = (0..columns).map(|i| if was.is_empty() { 0.0 } else { was[i * old / columns] }).collect();
            };
            for v in [&mut self.amp, &mut self.glow, &mut self.streak, &mut self.hold] {
                stretch(v);
            }
            self.columns = columns;
            self.cols = vec![Column::default(); columns];
            let x = |i: usize| if columns > 1 { i as f32 / (columns - 1) as f32 } else { 0.0 };
            let hill = |center: f32, half: f32| -> Vec<f32> { (0..columns).map(|i| (1.0 - (x(i) - center).abs() / half).clamp(0.0, 1.0).powf(0.7)).collect() };
            self.weights = [hill(0.12, 0.28), hill(0.48, 0.34), hill(0.85, 0.34)];
            self.set_seed(self.seed);
        }
        if rows != self.rows {
            self.rows = rows;
            self.above = rows.saturating_sub(1) / 2;
        }
    }

    fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
        self.tex_up = texture(self.columns, seed);
        let other = texture(self.columns, seed.wrapping_add(1));
        self.tex_dn = self.tex_up.iter().zip(&other).map(|(a, b)| 0.6 * a + 0.4 * b).collect();
    }

    fn step(&mut self, dt: f32) {
        for k in 0..3 {
            self.flare[k] = (self.flare[k] * (-dt / FLARE_DECAY).exp()).max(self.feats.onset[k]);
        }
        let up = 1.0 - (-dt / AMP_ATTACK).exp();
        let glow_up = 1.0 - (-dt / GLOW_ATTACK).exp();
        let glow_down = 1.0 - (-dt / GLOW_RELEASE).exp();
        for c in 0..self.columns {
            let kick = (0..3).map(|k| self.flare[k] * self.weights[k][c]).fold(0.0f32, f32::max);
            let boosted = (self.feats.target[c] + 0.25 * kick).clamp(0.0, 1.0);
            let a = self.amp[c];
            self.amp[c] = if boosted > a { a + (boosted - a) * up } else { boosted.max(a - FALL * dt) };
            let glow_to = (self.amp[c] + 0.30 * kick).clamp(0.0, 1.0);
            let g = self.glow[c];
            self.glow[c] = g + (glow_to - g) * if glow_to > g { glow_up } else { glow_down };
            let needle = self.amp[c] * (0.2 + 0.8 * self.tex_up[c]);
            let peak = needle.max(if kick > 0.25 { (0.30 + 0.45 * kick) * (0.4 + 0.6 * self.tex_up[c]) } else { 0.0 });
            if peak > self.streak[c] {
                self.streak[c] = peak;
                self.hold[c] = STREAK_HOLD;
            } else {
                self.hold[c] -= dt;
                if self.hold[c] <= 0.0 {
                    self.streak[c] = (self.streak[c] - STREAK_FALL * dt).max(0.0);
                }
            }
        }
        self.flash = self.flare.iter().fold(0.0f32, |m, &f| m.max(f));
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        let Some((_, ramp)) = &self.ramp else { return };
        if area.is_empty() || area.width as usize != self.columns || area.height != self.rows {
            return;
        }
        let (cols, rows) = (self.columns, area.height as usize);
        let above = self.above as usize;
        let below = rows - 1 - above;
        let reach = above.max(1) as f32;

        let mut shown = std::mem::take(&mut self.shown);
        if shown.len() != cols * rows {
            shown = vec![0; cols * rows];
        }
        let width = buf.area.width as usize;
        let mut put = |x: usize, y: usize, glyph: u8, fg: Color, bg: Color| {
            let cell = &mut buf.content[(y - buf.area.y as usize) * width + (x - buf.area.x as usize)];
            cell.reset();
            if glyph != b' ' {
                cell.set_char(glyph as char);
            }
            cell.fg = fg;
            cell.bg = bg;
        };

        let shows = fade_curve(self.presence);
        if shows <= 0.0 {
            let flat = ramp.bg[0];
            for y in 0..rows {
                for c in 0..cols {
                    put(area.x as usize + c, area.y as usize + y, b' ', flat, flat);
                }
            }
            shown.fill(0);
            self.shown = shown;
            return;
        }

        for (c, col) in self.cols.iter_mut().enumerate() {
            let length = |tex: f32| {
                let len = self.amp[c] * (0.2 + 0.8 * tex) * reach * 1.2;
                if len >= 1.2 { len } else { 0.0 }
            };
            let streak = self.streak[c] * reach * 1.2;
            let kick = (0..3).map(|k| self.flare[k] * self.weights[k][c]).fold(0.0f32, f32::max);
            *col = Column {
                len_up: length(self.tex_up[c]),
                len_dn: length(self.tex_dn[c]),
                heat_up: NEEDLE_DIM + (NEEDLE_HOT - NEEDLE_DIM) * self.tex_up[c],
                heat_dn: NEEDLE_DIM + (NEEDLE_HOT - NEEDLE_DIM) * self.tex_dn[c],
                streak: if streak >= 1.4 { streak } else { 0.0 },
                glow: self.glow[c].powf(1.3),
                kick: (kick * FLASH_STEPS).round() / FLASH_STEPS,
            };
        }

        let level = |v: f32, steps: usize| ((v * steps as f32 + 0.5) as usize).min(steps);
        let mut lit = std::mem::take(&mut self.lit);
        lit.resize(cols, 0.0);
        for s in -(above as isize)..=(below as isize) {
            let d = s.unsigned_abs();
            let row = (above as isize + s) as usize;
            let y = area.y as usize + row;

            for (c, l) in lit.iter_mut().enumerate() {
                let col = &self.cols[c];
                let len = match s.signum() {
                    -1 => col.len_up,
                    1 if d <= above => col.len_dn,
                    1 => 0.0,
                    _ => col.len_up.max(col.len_dn),
                };
                *l = if len > 0.0 && (d as f32) <= len.floor() + 1.0 { (col.glow + 0.6 * col.kick).min(1.0) } else { 0.0 };
            }

            for c in 0..cols {
                let col = self.cols[c];
                let beside = |i: isize| if i < 0 || i >= cols as isize { 0.0 } else { lit[i as usize] };
                let glow = lit[c].max(SPREAD * beside(c as isize - 1)).max(SPREAD * beside(c as isize + 1));
                let want = level(glow * shows, BG_LEVELS);
                let bg = ramp.bg[sticky(&mut shown[row * cols + c], want, BG_LEVELS)];

                let (glyph, hot) = if d > above {
                    (b' ', 0.0)
                } else if s < 0 {
                    self.needle_cell(d, col.len_up, col.heat_up, col.streak, true)
                } else if s > 0 {
                    self.needle_cell(d, col.len_dn, col.heat_dn, col.streak, false)
                } else {
                    let (up, a) = self.needle_cell(1, col.len_up, col.heat_up, col.streak, true);
                    let (down, b) = self.needle_cell(1, col.len_dn, col.heat_dn, col.streak, false);
                    (if up != b' ' || down != b' ' { b'|' } else { b' ' }, a.max(b))
                };
                let hot = hot * shows;
                let x = area.x as usize + c;
                if glyph != b' ' && hot >= 0.10 {
                    put(x, y, glyph, ramp.fg[level(hot, FG_LEVELS)], bg);
                } else {
                    put(x, y, b' ', bg, bg);
                }
            }
        }
        self.lit = lit;
        self.shown = shown;
    }

    fn needle_cell(&self, d: usize, len: f32, heat: f32, streak: f32, upper: bool) -> (u8, f32) {
        let df = d as f32;
        let spike = if len > 0.0 && df <= len { heat } else { 0.0 };
        let trail = if streak > 0.0 && df <= streak { 0.22 * pow06(1.0 - df / streak) } else { 0.0 };
        let fg = 1.0 - (1.0 - spike) * (1.0 - trail);
        let mut glyph = b' ';
        if fg >= 0.10 {
            glyph = b'|';
        }
        let frac = len - len.floor();
        if df == len.floor() + 1.0 && frac > 0.15 && fg < 0.5 && len > 0.9 && fg >= 0.10 {
            if frac < 0.45 {
                glyph = if upper { b'.' } else { b'\'' };
            } else if frac < 0.75 {
                glyph = b':';
            }
        }
        (glyph, fg)
    }

    #[cfg(test)]
    pub fn amps(&self) -> &[f32] {
        &self.amp
    }

    #[cfg(test)]
    pub fn flash(&self) -> f32 {
        self.flash
    }

    #[cfg(test)]
    pub fn presence(&self) -> f32 {
        self.presence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_seed_gives_its_own_fixed_pattern() {
        let a = texture(200, 7);
        assert_eq!(a, texture(200, 7), "the same seed must always give the same pattern");
        assert_ne!(a, texture(200, 8));
        assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!((0.25..0.6).contains(&mean), "mean {mean}");
        assert!(a.iter().any(|&v| v < 0.1) && a.iter().any(|&v| v > 0.8));
    }

    #[test]
    fn a_song_has_one_seed_whatever_else_is_true_of_it() {
        assert_eq!(song_seed(4_000_000, 215_000), song_seed(4_000_000, 215_000));
        assert_ne!(song_seed(4_000_000, 215_000), song_seed(4_000_001, 215_000));
        assert_ne!(song_seed(4_000_000, 215_000), song_seed(4_000_000, 215_001));
    }

    #[test]
    fn a_background_level_holds_until_the_wanted_one_is_two_steps_away() {
        let mut shown = 5u8;
        assert_eq!((sticky(&mut shown, 6, 16), shown), (5, 5));
        assert_eq!((sticky(&mut shown, 4, 16), shown), (5, 5));
        assert_eq!((sticky(&mut shown, 7, 16), shown), (7, 7));
        assert_eq!((sticky(&mut shown, 3, 16), shown), (3, 3));
        assert_eq!((sticky(&mut shown, 0, 16), shown), (0, 0));
        assert_eq!((sticky(&mut shown, 1, 16), shown), (0, 0));
        shown = 15;
        assert_eq!((sticky(&mut shown, 16, 16), shown), (16, 16));
        let mut shown = 8u8;
        for want in [8, 9, 8, 9, 9, 8, 7, 8, 9, 7, 9] {
            assert_eq!(sticky(&mut shown, want, 16), 8);
        }
    }

    #[test]
    fn the_streak_curve_matches_the_power_it_stands_for() {
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            let error = (pow06(x) - x.powf(0.6)).abs();
            assert!(error < 0.025, "{x}: {error}");
            if x >= 0.25 {
                assert!(error < 0.001, "{x}: {error}");
            }
        }
        assert_eq!((pow06(-1.0), pow06(2.0)), (0.0, 1.0));
    }

    #[test]
    fn a_fade_is_eased_at_both_ends_and_only_ever_rises() {
        assert_eq!((fade_curve(0.0), fade_curve(1.0)), (0.0, 1.0));
        assert_eq!((fade_curve(-1.0), fade_curve(2.0)), (0.0, 1.0));
        assert!((fade_curve(0.5) - 0.5).abs() < 1e-6);
        assert!(fade_curve(0.05) < 0.01 && fade_curve(0.95) > 0.99);
        let mut last = 0.0;
        for i in 0..=1000 {
            let v = fade_curve(i as f32 / 1000.0);
            assert!(v >= last, "{i}: {v} after {last}");
            last = v;
        }
    }

    #[test]
    fn the_visualizer_never_draws_the_marks_that_look_ugly() {
        let source = include_str!("viz.rs");
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        for mark in ['#', '=', '+', '-', '*', '@'] {
            for banned in [format!("b'{mark}'"), format!("'{mark}'"), format!("\"{mark}\"")] {
                assert!(!code.contains(&banned), "the visualizer draws {banned}");
            }
        }
    }

    #[test]
    fn the_visualizer_has_no_colors_or_theme_names_of_its_own() {
        let source = include_str!("viz.rs");
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        for t in crate::theme::THEMES {
            assert!(!code.contains(&format!("\"{}\"", t.name)), "the visualizer names the theme {}", t.name);
        }
        assert!(!code.contains("Color::Rgb"), "a color made here");
        assert!(!code.contains("Color::Indexed"), "a color made here");
        let hex_literal = |s: &str| s.match_indices("0x").any(|(i, _)| s[i + 2..].chars().take_while(|c| c.is_ascii_hexdigit() || *c == '_').filter(|c| *c != '_').count() == 6);
        assert!(!hex_literal(code), "a six-digit color literal");
    }
}
