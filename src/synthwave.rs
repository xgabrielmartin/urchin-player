use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::night_sky::{Cover, Mark, NightSky, Sky, SHADES};
use crate::spectrum::{Tap, FFT_SIZE};
use crate::theme::{rgb_color, rgb_mix, SynthColors};

const VOICE_HZ: [(f32, f32); VOICES] =
    [(20.0, 80.0), (80.0, 130.0), (130.0, 200.0), (200.0, 300.0), (300.0, 450.0), (800.0, 3000.0), (4000.0, 9000.0)];
const VOICES: usize = 7;
const SUN: usize = 0;
const GLOW: usize = 1;
const GLOW_RINGS: usize = 4;
const RIDGE: usize = 5;
const ROAD: usize = 6;
const VOICE_SPAN: f32 = 24.0;
const VOICE_FLOOR: f32 = -62.0;
const VOICE_FALL: f32 = 3.0;
const VOICE_GATE: f32 = 30.0;
const VOICE_ATTACK: f32 = 0.02;
const VOICE_RELEASE: f32 = 0.18;
const VOICE_AVERAGE: f32 = 1.5;
const FLUX_HISTORY: usize = 30;
const REFRACTORY: f32 = 0.13;
const VOICE_HIT_LEVEL: f32 = 0.4;
const STALE: Duration = Duration::from_millis(200);
const SILENCE_PEAK: f32 = 1.0e-4;
const SILENCE_TAIL: usize = 1024;
const SILENCE_HOLD: f32 = 0.12;
const FADE_OUT: f32 = 2.5;
const FADE_IN: f32 = 0.35;
const MIN_STEP: f32 = 0.028;

#[derive(Clone, Copy)]
struct Voice {
    bins: (usize, usize),
    loudest: f32,
    prev_db: f32,
    have_prev: bool,
    fast: f32,
    slow: f32,
    hit: f32,
    history: [f32; FLUX_HISTORY],
    history_len: usize,
    history_pos: usize,
    last_hit: f32,
}

impl Voice {
    const NEW: Voice = Voice {
        bins: (0, 1),
        loudest: VOICE_FLOOR,
        prev_db: 0.0,
        have_prev: false,
        fast: 0.0,
        slow: 0.0,
        hit: 0.0,
        history: [0.0; FLUX_HISTORY],
        history_len: 0,
        history_pos: 0,
        last_hit: -1.0,
    };

    fn hear(&mut self, db: f32, gate: f32, dt: f32, time: f32) {
        let floor = (self.loudest - VOICE_SPAN).max(gate);
        let level = ((db - floor) / VOICE_SPAN).clamp(0.0, 1.0);
        let heard = ((level - 0.25) / 0.75).max(0.0);
        let tau = if heard > self.fast { VOICE_ATTACK } else { VOICE_RELEASE };
        self.fast += (heard - self.fast) * (1.0 - (-dt / tau).exp());
        self.slow += (heard - self.slow) * (1.0 - (-dt / VOICE_AVERAGE).exp());

        let now = db.max(floor);
        let rise = if self.have_prev { (now - self.prev_db).max(0.0) } else { 0.0 };
        self.prev_db = now;
        self.have_prev = true;
        let h = &self.history[..self.history_len];
        let mean = if h.is_empty() { 0.0 } else { h.iter().sum::<f32>() / h.len() as f32 };
        let var = if h.is_empty() { 0.0 } else { h.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / h.len() as f32 };
        let threshold = mean + 1.4 * var.sqrt() + 1.0;
        self.history[self.history_pos] = rise;
        self.history_pos = (self.history_pos + 1) % FLUX_HISTORY;
        self.history_len = (self.history_len + 1).min(FLUX_HISTORY);
        self.hit = 0.0;
        if rise > threshold && level >= VOICE_HIT_LEVEL && time - self.last_hit > REFRACTORY {
            self.hit = 0.55 + 0.45 * ((rise - threshold) / (3.0 + threshold) * 2.0).min(1.0);
            self.last_hit = time;
        }
    }

    fn stir(&self) -> f32 {
        (0.45 * self.fast + 2.0 * (self.fast - self.slow).max(0.0)).clamp(0.0, 1.0)
    }

    fn hush(&mut self) {
        let bins = self.bins;
        *self = Voice { bins, ..Voice::NEW };
    }
}

struct Bands {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    rate: u32,
    voices: [Voice; VOICES],
    time: f32,
}

impl Bands {
    fn new() -> Bands {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos()).collect();
        Bands {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; FFT_SIZE / 2 + 1],
            fft,
            window,
            rate: 0,
            voices: [Voice::NEW; VOICES],
            time: 0.0,
        }
    }

    fn layout(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let df = rate as f32 / FFT_SIZE as f32;
        let bins = FFT_SIZE / 2 + 1;
        for (voice, (lo, up)) in self.voices.iter_mut().zip(VOICE_HZ) {
            let k0 = ((lo / df).ceil() as usize).clamp(1, bins - 1);
            let k1 = ((up / df).ceil() as usize).clamp(k0 + 1, bins);
            *voice = Voice { bins: (k0, k1), ..Voice::NEW };
        }
    }

    fn update(&mut self, tap: &Tap, dt: f32, stale: bool) -> bool {
        self.time += dt;
        for voice in &mut self.voices {
            voice.hit = 0.0;
        }
        if !stale {
            tap.latest(&mut self.input);
        }
        let quiet = stale || self.input[FFT_SIZE - SILENCE_TAIL..].iter().all(|x| x.abs() < SILENCE_PEAK);
        if quiet {
            for voice in &mut self.voices {
                voice.have_prev = false;
            }
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
        let mut db = [0.0f32; VOICES];
        for (db, voice) in db.iter_mut().zip(self.voices.iter_mut()) {
            let energy: f32 = self.power[voice.bins.0..voice.bins.1].iter().sum();
            *db = 10.0 * (energy * scale * scale + 1e-14).log10();
            voice.loudest = db.max(voice.loudest - VOICE_FALL * dt).max(VOICE_FLOOR);
        }
        let gate = self.voices.iter().map(|v| v.loudest).fold(VOICE_FLOOR, f32::max) - VOICE_GATE;
        for (db, voice) in db.iter().zip(self.voices.iter_mut()) {
            voice.hear(*db, gate, dt, self.time);
        }
        false
    }
}

pub const OWN_COLORS: usize = usize::MAX;
pub const MOON_COLORS: usize = usize::MAX - 1;

const BG_LV: usize = 12;
const FG_LV: usize = 10;
const STEP: f32 = 0.34;
const STEP_EASE: f32 = 0.045;
const CRAWL_AFTER: f32 = 1.2;
const CRAWL: f32 = 0.12;
const SUN_DECAY: f32 = 0.20;
const SUN_FILL: f32 = 0.54;
const FLASH_DECAY: f32 = 0.16;
const GLOW_EASE: f32 = 0.14;
const GLOW_EDGE: [f32; GLOW_RINGS] = [1.4, 1.75, 2.1, 2.45];
const GLOW_BASE: [f32; GLOW_RINGS] = [0.50, 0.34, 0.18, 0.06];
const GLOW_STIR: f32 = 0.22;
const FILL_BG: f32 = 0.30;
const FILL_FG: f32 = 0.34;
const DASH_LEN: f32 = 0.7;
const DASH_NEAR: f32 = 1.5;
const DASH_ON: f32 = 0.4;
const ROAD_GLOW: f32 = 0.38;
const CROSS_AT: f32 = 0.55;
fn fade_curve(presence: f32) -> f32 {
    let p = presence.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

const SUN_ROWS: f32 = 0.75;

const SUN_STRETCH: f32 = 1.22;

fn sun_rows(cols: usize, hr: usize) -> usize {
    let tall = (SUN_ROWS * hr as f32).round() as usize;
    tall.min(cols / 10).clamp(1, hr.max(1))
}

fn sun_full_radius(cols: usize, hr: usize) -> f32 {
    2.0 * sun_rows(cols, hr) as f32
}

fn sun_half_width(cols: usize, hr: usize) -> f32 {
    SUN_STRETCH * sun_full_radius(cols, hr)
}

fn in_sun(dx: f32, up: f32, radius: f32, half: f32) -> bool {
    if up < 0.0 || radius <= 0.0 {
        return false;
    }
    let t = (up / radius).min(1.0);
    let ellipse = (half / radius) * (1.0 - t * t).sqrt();
    let circle = (1.0 - t * t).sqrt();
    let k = t.powi(3);
    let width = radius * (circle * k + ellipse * (1.0 - k));
    dx <= width && up <= radius
}

fn sun_base_radius(cols: usize, hr: usize) -> f32 {
    let full = sun_full_radius(cols, hr);
    (full / 1.15).max(full - 1.6)
}

#[cfg(test)]
const REF_COLS: usize = 170;
const REF_HR: usize = 16;
const REF_RANGE: [u8; 85] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 3, 4, 4, 5, 5, 4, 3, 3, 4, 4, 5,
    5, 4, 3, 5, 6, 6, 7, 8, 8, 8, 9, 9, 10, 9, 9, 8, 7, 7, 6, 7, 8, 8, 9, 8, 8, 7, 7, 7, 8, 8, 9, 10, 10, 11, 12, 12, 13, 13, 12,
    13, 13, 14, 15,
];
const REF_REACH: f32 = 29.78;
const REF_TURN: usize = 60;
const RANGE_GIVE: f32 = 1.25;
const BEYOND: usize = 6;
const VALLEY_FLOOR: i32 = 2;

fn sun_reach(cols: usize, hr: usize) -> f32 {
    SUN_STRETCH * sun_full_radius(cols, hr) + 0.5
}

fn range_and_beyond(cols: usize, hr: usize) -> (Vec<i32>, i32) {
    range_outside(cols, hr, sun_reach(cols, hr))
}

fn range_outside(cols: usize, hr: usize, reach: f32) -> (Vec<i32>, i32) {
    let mut out = vec![0i32; cols];
    let cx = cols as f32 / 2.0;
    let ref_reach = REF_REACH;
    let span = REF_RANGE.len() as f32 - ref_reach;
    let room = cx - reach;
    if room < 1.0 || hr == 0 {
        return (out, 0);
    }
    let sy = (hr as f32 / REF_HR as f32).min(RANGE_GIVE * room / span);
    let sx = (room / span).min(RANGE_GIVE * sy);
    let need = (ref_reach + (room + 1.0 + BEYOND as f32) / sx).ceil() as usize + 2;
    let mut range: Vec<f32> = REF_RANGE.iter().map(|v| *v as f32).collect();
    let (lo, hi) = (REF_TURN, REF_RANGE.len() - 1);
    let (mut at, mut back) = (hi, true);
    while range.len() < need {
        if back {
            at -= 1;
            back = at > lo;
        } else {
            at += 1;
            back = at == hi;
        }
        range.push(REF_RANGE[at] as f32);
    }
    let n = range.len();
    let foot = range.iter().position(|v| *v > 0.0).unwrap_or(n);
    let mut turn = vec![0i8; n];
    let mut i = foot;
    while i < n {
        let mut j = i;
        while j + 1 < n && range[j + 1] == range[i] {
            j += 1;
        }
        if i > foot && j + 1 < n {
            let (l, r, v) = (range[i - 1], range[j + 1], range[i]);
            turn[(i + j) / 2] = if l < v && r < v { 1 } else if l > v && r > v { -1 } else { 0 };
        }
        i = j + 1;
    }
    let level = |p: f32| {
        let t = (p - 0.5).clamp(0.0, n as f32 - 1.0);
        let i = (t.floor() as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        range[i] + (range[j] - range[i]) * (t - i as f32)
    };
    let first = cols / 2;
    let mut rows: Vec<i32> = Vec::with_capacity(cols - first + BEYOND);
    let mut held: Vec<(usize, i8, f32)> = Vec::new();
    for x in first..cols + BEYOND {
        let d = x as f32 + 0.5 - cx;
        if d < reach {
            rows.push(0);
            continue;
        }
        let p = |d: f32| ref_reach + (d - reach) / sx;
        let a = ((p(d - 0.5) - 0.5).ceil().max(0.0) as usize).min(n);
        let b = ((p(d + 0.5) - 0.5).ceil().max(0.0) as usize).min(n);
        let tops = (a..b).filter(|i| turn[*i] > 0).count();
        let bottoms = (a..b).filter(|i| turn[*i] < 0).count();
        let v = if tops > 0 && bottoms == 0 {
            let v = (a..b).map(|i| range[i]).fold(0.0f32, f32::max);
            held.push((x - first, 1, v));
            v
        } else if bottoms > 0 && tops == 0 {
            let v = (a..b).map(|i| range[i]).fold(f32::MAX, f32::min);
            held.push((x - first, -1, v));
            v
        } else {
            level(p(d))
        };
        rows.push(((v * sy + 0.5).floor() as i32).clamp(0, hr as i32));
    }
    if let Some(start) = rows.iter().position(|k| *k >= VALLEY_FLOOR) {
        for k in &mut rows[start..] {
            *k = (*k).max(VALLEY_FLOOR.min(hr as i32));
        }
    }
    for (i, &(t, kind, v)) in held.iter().enumerate() {
        if kind < 0 {
            continue;
        }
        let before = held[..i].iter().rev().find(|h| h.1 < 0);
        let after = held[i + 1..].iter().find(|h| h.1 < 0);
        let valleys: Vec<&(usize, i8, f32)> = before.into_iter().chain(after).collect();
        if valleys.is_empty() || valleys.iter().any(|h| v - h.2 < 2.0) {
            continue;
        }
        let above = valleys.iter().map(|h| rows[h.0]).max().unwrap_or(0) + 1;
        rows[t] = rows[t].max(above.min(hr as i32));
    }
    let m = rows.len();
    let top = hr as i32;
    for _ in 0..top + 2 {
        let mut changed = false;
        let mut t = 1;
        while t + 1 < m {
            let k = rows[t];
            let mut end = t;
            while end + 1 < m && rows[end + 1] == k {
                end += 1;
            }
            if end + 1 >= m || !(rows[t - 1] < k && rows[end + 1] < k) {
                t = end + 1;
                continue;
            }
            let wide = end - t + 1;
            match wide {
                2 => {}
                1 => {
                    let low = |side: &mut dyn Iterator<Item = &i32>| side.take_while(|v| **v < k).min().copied().unwrap_or(0);
                    let stands = k - low(&mut rows[..t].iter().rev()).max(low(&mut rows[t + 1..].iter()));
                    let shoulder = |a: usize, b: Option<usize>| rows[a] == k - 1 && b.is_some_and(|b| rows[b] == k - 1);
                    let (outer, inner) = (shoulder(t + 1, (t + 2 < m).then_some(t + 2)), shoulder(t - 1, t.checked_sub(2)));
                    if outer && (stands <= 1 || !inner) {
                        rows[t + 1] = k;
                    } else if inner && stands <= 1 {
                        rows[t - 1] = k;
                    } else {
                        rows.insert(t, k);
                        rows.pop();
                    }
                    end = t + 1;
                    changed = true;
                }
                3 if k > VALLEY_FLOOR => {
                    let at = if rows[end + 1] <= rows[t - 1] { end } else { t };
                    rows[at] = k - 1;
                    changed = true;
                }
                3 if k < top => {
                    rows[t] = k + 1;
                    rows[t + 1] = k + 1;
                    changed = true;
                }
                _ if k < top && wide >= 4 => {
                    let ring = if wide >= 6 { 2 } else { 1 };
                    for v in &mut rows[t + ring..=end - ring] {
                        *v = k + 1;
                    }
                    changed = true;
                }
                _ if wide >= 4 && k > VALLEY_FLOOR => {
                    let keep = t + (wide - 2) / 2;
                    for (i, v) in rows.iter_mut().enumerate().take(end + 1).skip(t) {
                        if i != keep && i != keep + 1 {
                            *v = k - 1;
                        }
                    }
                    changed = true;
                }
                _ => {}
            }
            t = end + 1;
        }
        if !changed {
            break;
        }
    }
    let mut t = 1;
    while t + 1 < m {
        let k = rows[t];
        let mut end = t;
        while end + 1 < m && rows[end + 1] == k {
            end += 1;
        }
        if end + 1 < m && k > VALLEY_FLOOR && (rows[t - 1] - k) * (rows[end + 1] - k) < 0 {
            let down_is_out = rows[end + 1] < k;
            let mut left = end - t + 1;
            let mut at = if down_is_out { end } else { t };
            while left >= 4 {
                let notch = if down_is_out { at - 2 } else { at + 2 };
                rows[notch] = k - 1;
                left -= 3;
                at = if down_is_out { notch - 1 } else { notch + 1 };
            }
        }
        t = end + 1;
    }
    let clear = |t: usize| (first + t) as f32 + 0.5 - cx >= reach;
    if let Some(start) = rows.iter().position(|k| *k >= VALLEY_FLOOR) {
        for k in &mut rows[start..] {
            *k = (*k).max(VALLEY_FLOOR.min(hr as i32));
        }
        for t in (1..=start).rev() {
            if clear(t - 1) && rows[t - 1] < rows[t] - 1 {
                rows[t - 1] = rows[t] - 1;
            }
        }
    }
    let beyond = rows[cols - first];
    rows.truncate(cols - first);
    for (t, k) in rows.iter().enumerate() {
        out[first + t] = *k;
        out[cols - 1 - (first + t)] = *k;
    }
    (out, beyond)
}

#[cfg(test)]
fn range_heights(cols: usize, hr: usize) -> Vec<i32> {
    range_and_beyond(cols, hr).0
}

const BG_SKY: u8 = 0;
const BG_FLOOR: u8 = 1;
const BG_MOUNT: u8 = 2;
const FG_SUN: u8 = 0;
const FG_LINE: u8 = 1;
const FG_STAR: u8 = 2;
const NO_FG: u8 = 255;
const STAR_CLEARANCE: f32 = 1.25;

struct Palette {
    bg: [[Color; BG_LV + 1]; 3],
    fg: [[Color; FG_LV + 1]; 2 + SHADES],
    ground: Color,
}

fn along(stops: &[(f32, u32)], t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        if t <= w[1].0 {
            return rgb_mix(w[0].1, w[1].1, (t - w[0].0) / (w[1].0 - w[0].0));
        }
    }
    stops[stops.len() - 1].1
}

impl Palette {
    fn new(c: &SynthColors) -> Palette {
        let sky = [(0.0, c.ground), (0.28, c.sky_deep), (0.62, c.sky_low), (1.0, c.halo)];
        let floor = [(0.0, c.ground), (0.30, c.floor_deep), (0.68, rgb_mix(c.floor_deep, c.floor_glow, 0.45)), (1.0, c.floor_glow)];
        let mount = [(0.0, c.ground), (0.5, c.mount), (1.0, rgb_mix(c.mount, c.grid, 0.35))];
        let sun = [(0.0, c.ground), (0.45, rgb_mix(c.sun, c.ground, 0.55)), (0.75, c.sun), (1.0, c.sun_hot)];
        let line = [(0.0, c.ground), (0.40, rgb_mix(c.grid, c.ground, 0.55)), (0.72, c.grid), (1.0, c.grid_hot)];
        let bg = |stops: &[(f32, u32)]| std::array::from_fn(|i| rgb_color(along(stops, i as f32 / BG_LV as f32)));
        let fg = |stops: &[(f32, u32)]| std::array::from_fn(|i| rgb_color(along(stops, i as f32 / FG_LV as f32)));
        let star = |k: usize| fg(&[(0.0, c.ground), (1.0, c.stars[k])]);
        Palette { bg: [bg(&sky), bg(&floor), bg(&mount)], fg: [fg(&sun), fg(&line), star(0), star(1), star(2), star(3)], ground: rgb_color(c.ground) }
    }
}

fn sticky(shown: &mut (u8, u8), ramp: u8, want: usize, top: usize) -> usize {
    let have = if shown.0 == ramp { shown.1 as usize } else { usize::MAX };
    let now = if want == 0 || want == top || have == usize::MAX || want.abs_diff(have) >= 2 { want } else { have };
    *shown = (ramp, now as u8);
    now
}

#[derive(Clone, Copy)]
struct Spec {
    glyph: char,
    fg: u8,
    fg_at: f32,
    bg: u8,
    bg_at: f32,
}

const BLANK: Spec = Spec { glyph: ' ', fg: NO_FG, fg_at: 0.0, bg: BG_SKY, bg_at: 0.0 };

const DOT_BIT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

struct Scene {
    cols: usize,
    rows: usize,
    hr: usize,
    cx: f32,
}

impl Scene {
    fn new(cols: usize, rows: usize) -> Scene {
        let floor = ((rows as f32 * 0.42).round() as usize).clamp(2, rows.saturating_sub(2).max(2));
        Scene { cols, rows, hr: rows - floor, cx: cols as f32 / 2.0 }
    }
    fn floor_rows(&self) -> usize {
        self.rows - self.hr
    }
}

fn sky_cover(sc: &Scene) -> Vec<Cover> {
    let (cols, hr) = (sc.cols, sc.hr);
    let (hi, _) = range_and_beyond(cols, hr);
    let (radius, half) = (sun_base_radius(cols, hr), sun_half_width(cols, hr));
    let cy = 2.0 * hr as f32;
    (0..hr * cols)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            if r + (hi[c].max(0) as usize) >= hr {
                return Cover::Mountain;
            }
            let moon = DOT_BIT.iter().enumerate().any(|(i, col)| {
                (0..col.len()).any(|t| {
                    let (px, py) = ((2 * c + i) as f32 * 0.5 + 0.25, (4 * r + t) as f32 * 0.5 + 0.25);
                    in_sun((px - sc.cx).abs(), cy - py, radius, half)
                })
            });
            if moon {
                Cover::Moon
            } else {
                Cover::Clear
            }
        })
        .collect()
}

fn open_sky(sc: &Scene) -> Vec<bool> {
    let (cols, hr) = (sc.cols, sc.hr);
    let (hi, _) = range_and_beyond(cols, hr);
    let r_base = sun_base_radius(cols, hr);
    let half = sun_half_width(cols, hr);
    let clear = STAR_CLEARANCE * sun_full_radius(cols, hr);
    let cy = 2.0 * hr as f32;
    (0..hr * cols)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let d = (((c as f32 + 0.5 - sc.cx) * r_base / half).powi(2) + (2.0 * r as f32 + 1.0 - cy).powi(2)).sqrt();
            r + (hi[c].max(0) as usize) < hr && d >= clear
        })
        .collect()
}

pub struct Synthwave {
    bands: Bands,
    cols: usize,
    rows: u16,
    sun: f32,
    bass: f32,
    glow: [f32; GLOW_RINGS],
    ridge: f32,
    road: f32,
    phase: f32,
    pending: f32,
    since_beat: f32,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
    palette: Option<(usize, Palette)>,
    shown: Vec<(u8, u8)>,
    spec: Vec<Spec>,
    night: Option<NightSky>,
    open: (usize, usize, Vec<bool>, Vec<Cover>),
    marks: Vec<Mark>,
}

impl Default for Synthwave {
    fn default() -> Self {
        Synthwave::new()
    }
}

impl Synthwave {
    pub fn new() -> Synthwave {
        Synthwave {
            bands: Bands::new(),
            cols: 0,
            rows: 0,
            sun: 0.0,
            bass: 0.0,
            glow: [0.0; GLOW_RINGS],
            ridge: 0.0,
            road: 0.0,
            phase: 0.0,
            pending: 0.0,
            since_beat: 0.0,
            last: None,
            last_written: 0,
            quiet_since: None,
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
            palette: None,
            shown: Vec::new(),
            spec: Vec::new(),
            night: None,
            open: (0, 0, Vec::new(), Vec::new()),
            marks: Vec::new(),
        }
    }

    pub fn moonlit() -> Synthwave {
        Synthwave { night: Some(NightSky::new()), ..Synthwave::new() }
    }

    pub fn use_colors(&mut self, key: usize, colors: impl FnOnce() -> SynthColors) {
        if self.palette.as_ref().map(|p| p.0) != Some(key) {
            self.palette = Some((key, Palette::new(&colors())));
        }
    }

    pub fn is_active(&self) -> bool {
        if self.silent {
            return self.presence > 0.0;
        }
        true
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, cols: usize, rows: u16, seed: u64, now: Instant) {
        let cols = cols.max(1);
        self.bands.layout(tap.rate().max(1));
        self.cols = cols;
        self.rows = rows;
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
        if self.silent {
            self.presence = (self.presence - dt / FADE_OUT).max(0.0);
            if self.presence <= 0.0 {
                self.sun = 0.0;
                self.bass = 0.0;
                self.glow = [0.0; GLOW_RINGS];
                self.ridge = 0.0;
                self.road = 0.0;
                self.pending = 0.0;
                for voice in &mut self.bands.voices {
                    voice.hush();
                }
            }
        } else {
            self.presence = (self.presence + dt / FADE_IN).min(1.0);
        }
        if let Some(night) = &mut self.night {
            let size = (cols, rows as usize);
            if (self.open.0, self.open.1) != size {
                let (open, cover) = if size.1 >= 4 && size.0 >= 8 {
                    let sc = Scene::new(size.0, size.1);
                    (open_sky(&sc), sky_cover(&sc))
                } else {
                    (Vec::new(), Vec::new())
                };
                self.open = (size.0, size.1, open, cover);
            }
            if !self.open.2.is_empty() {
                let sky = Sky { cols, rows: self.open.2.len() / cols, open: &self.open.2, cover: &self.open.3 };
                night.update(&sky, seed, dt, !self.silent);
            }
            if self.silent && self.presence <= 0.0 {
                night.calm();
            }
        }
        self.step(dt);
    }

    fn step(&mut self, dt: f32) {
        let voices = &self.bands.voices;
        let fall = |tau: f32| (-dt / tau).exp();
        if voices[SUN].hit > 0.0 {
            self.sun = self.sun.max(0.55 + 0.45 * voices[SUN].hit);
        }
        self.sun *= fall(SUN_DECAY);
        self.bass += (voices[SUN].fast - self.bass) * (1.0 - fall(0.4));
        for (ring, voice) in self.glow.iter_mut().zip(&voices[GLOW..=GLOW_RINGS]) {
            *ring += (voice.stir() - *ring) * (1.0 - fall(GLOW_EASE));
        }
        self.ridge = (self.ridge * fall(FLASH_DECAY)).max(voices[RIDGE].stir()).max(voices[RIDGE].hit);
        let hit = voices[ROAD].hit;
        self.road = (self.road * fall(FLASH_DECAY)).max(hit);
        if hit > 0.0 {
            self.pending += STEP * (0.6 + 0.4 * hit);
            self.since_beat = 0.0;
        } else {
            self.since_beat += dt;
        }

        let moved = self.pending * (1.0 - (-dt / STEP_EASE).exp());
        self.pending -= moved;
        self.phase += moved;
        if self.since_beat > CRAWL_AFTER && !self.silent {
            self.phase += CRAWL * dt;
        }
        self.phase = self.phase.rem_euclid(1.0);
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        let Some((_, palette)) = &self.palette else { return };
        if area.is_empty() || area.width as usize != self.cols || area.height != self.rows {
            return;
        }
        let (cols, rows) = (area.width as usize, area.height as usize);
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
        if shows <= 0.0 || rows < 4 || cols < 8 {
            for y in 0..rows {
                for c in 0..cols {
                    put(area.x as usize + c, area.y as usize + y, ' ', palette.ground, palette.ground);
                }
            }
            self.shown.clear();
            return;
        }

        let scene = Scene::new(cols, rows);
        if let Some(night) = &self.night {
            night.marks(&mut self.marks);
        }
        let mut spec = std::mem::take(&mut self.spec);
        spec.clear();
        spec.resize(cols * rows, BLANK);
        self.compose(&scene, &mut spec);

        if self.shown.len() != cols * rows {
            self.shown = vec![(255, 0); cols * rows];
        }
        let level = |v: f32, n: usize| ((v * n as f32 + 0.5) as usize).min(n);
        for y in 0..rows {
            for c in 0..cols {
                let i = y * cols + c;
                let s = spec[i];
                let want = level(s.bg_at * shows, BG_LV);
                let bg = palette.bg[s.bg as usize][sticky(&mut self.shown[i], s.bg, want, BG_LV)];
                let strength = s.fg_at * shows;
                if s.glyph != ' ' && s.fg != NO_FG && strength >= 0.10 {
                    put(area.x as usize + c, area.y as usize + y, s.glyph, palette.fg[s.fg as usize][level(strength, FG_LV)], bg);
                } else {
                    put(area.x as usize + c, area.y as usize + y, ' ', bg, bg);
                }
            }
        }
        self.spec = spec;
    }

    fn compose(&self, sc: &Scene, spec: &mut [Spec]) {
        let (cols, hr) = (sc.cols, sc.hr);
        let f = sc.floor_rows();
        let swell = self.sun.max(0.45 * self.bass).clamp(0.0, 1.0);

        let y_h = 2.0 * hr as f32;
        let r_base = sun_base_radius(cols, hr);
        let cy = y_h;
        let radius = r_base + (sun_full_radius(cols, hr) - r_base) * swell;

        for r in 0..sc.rows {
            for c in 0..cols {
                let xc = c as f32 + 0.5;
                if r < hr {
                    let v = r as f32 / hr as f32;
                    let d = (((xc - sc.cx) * r_base / sun_half_width(cols, hr)).powi(2) + (2.0 * r as f32 + 1.0 - cy).powi(2)).sqrt();
                    let halo = (0..GLOW_RINGS).find(|i| d < GLOW_EDGE[*i] * r_base).map_or(0.0, |i| self.glow_at(i));
                    spec[r * cols + c] = Spec { bg: BG_SKY, bg_at: 0.10 + 0.30 * v.powf(1.3) + 0.50 * halo, ..BLANK };
                } else {
                    spec[r * cols + c] = Spec { bg: BG_FLOOR, bg_at: ROAD_GLOW, ..BLANK };
                }
            }
        }

        if self.night.is_some() && self.open.0 == cols && self.open.1 == sc.rows {
            for m in &self.marks {
                let cell = &mut spec[m.y * cols + m.x];
                cell.glyph = m.glyph;
                cell.fg = FG_STAR + m.shade;
                cell.fg_at = m.level;
            }
        }

        self.sun_cells(sc, spec, cy, radius, swell);
        self.mountain_cells(sc, spec);
        self.grid_cells(sc, spec, f);
    }

    fn glow_at(&self, i: usize) -> f32 {
        GLOW_BASE[i] + GLOW_STIR * self.glow[i]
    }

    fn sun_cells(&self, sc: &Scene, spec: &mut [Spec], cy: f32, radius: f32, swell: f32) {
        let (cols, hr) = (sc.cols, sc.hr);
        let top = (cy - radius) / 2.0;
        let half = sun_half_width(cols, hr);
        let mid = ((top.max(0.0) + hr as f32) / 2.0) / hr as f32;
        let behind = 0.10 + 0.30 * mid.powf(1.3) + 0.50 * self.glow_at(0);
        let c0 = ((sc.cx - half).floor().max(0.0)) as usize;
        let c1 = ((sc.cx + half).ceil() as usize).min(cols - 1);
        for r in (top.floor().max(0.0) as usize)..hr {
            for c in c0..=c1 {
                let (mut bits, mut ring) = (0u8, false);
                for (i, col_bits) in DOT_BIT.iter().enumerate() {
                    for (t, bit) in col_bits.iter().enumerate() {
                        let (gx, gy) = (2 * c + i, 4 * r + t);
                        let (px, py) = (gx as f32 * 0.5 + 0.25, gy as f32 * 0.5 + 0.25);
                        let (dx, up) = ((px - sc.cx).abs(), cy - py);
                        if !in_sun(dx, up, radius, half) {
                            continue;
                        }
                        bits |= bit;
                        ring |= !in_sun(dx, up, radius - 0.75, half - 0.9);
                    }
                }
                if bits != 0 {
                    let cell = &mut spec[r * cols + c];
                    cell.bg_at = behind;
                    cell.glyph = braille(bits);
                    cell.fg = FG_SUN;
                    cell.fg_at = SUN_FILL + 0.22 * swell + if ring { 0.14 } else { 0.0 };
                }
            }
        }
    }

    fn mountain_cells(&self, sc: &Scene, spec: &mut [Spec]) {
        let (cols, hr) = (sc.cols, sc.hr);
        let (hi, beyond) = range_and_beyond(cols, hr);
        for x in 0..cols {
            let here = hi[x];
            if here <= 0 {
                continue;
            }
            let left = if x > 0 { hi[x - 1] } else { beyond };
            let right = if x + 1 < cols { hi[x + 1] } else { beyond };
            let (dl, dr) = (here - left, right - here);
            let (glyph, apex) = match (dl.signum(), dr.signum()) {
                (1, -1) => (if here >= 3 { '▲' } else { '∧' }, true),
                (1, _) => ('╱', false),
                (-1, 1) => ('∨', false),
                (-1, _) => ('╲', false),
                (_, -1) => ('╲', false),
                (_, 1) => ('╱', false),
                _ => ('─', false),
            };
            let t = hr - here as usize;
            let edge = 0.60 + 0.30 * self.ridge * if apex { 1.0 } else { 0.5 };
            for r in t..hr {
                let cell = &mut spec[r * cols + x];
                let keep_sky = r == t;
                let behind = if keep_sky { (cell.bg, cell.bg_at) } else { (BG_MOUNT, FILL_BG) };
                *cell = Spec { glyph: ' ', fg: NO_FG, fg_at: 0.0, bg: behind.0, bg_at: behind.1 };
                if r == t {
                    cell.glyph = glyph;
                    cell.fg = FG_LINE;
                    cell.fg_at = edge;
                } else if (dl > 1 && r - t < dl as usize) || (dr < -1 && r - t < (-dr) as usize) {
                    cell.glyph = '│';
                    cell.fg = FG_LINE;
                    cell.fg_at = edge;
                } else {
                    cell.glyph = '▲';
                    cell.fg = FG_LINE;
                    cell.fg_at = FILL_FG;
                }
            }
        }
    }

    fn center_dashes(&self, sc: &Scene, spec: &mut [Spec], f: usize) {
        let (cols, hr) = (sc.cols, sc.hr);
        let col = sc.cx.floor() as usize;
        if col >= cols {
            return;
        }
        let along = |y: f32| (y + DASH_NEAR).ln();
        for j in 0..f {
            let (lo, hi) = (along(j as f32), along(j as f32 + 1.0));
            let lit = if hi - lo >= DASH_LEN {
                true
            } else {
                let n = 12;
                let on = (0..n)
                    .filter(|i| {
                        let u = lo + (*i as f32 + 0.5) / n as f32 * (hi - lo);
                        (u / DASH_LEN - self.phase).rem_euclid(1.0) < DASH_ON
                    })
                    .count();
                on * 2 >= n
            };
            if lit {
                let cell = &mut spec[(hr + j) * cols + col];
                *cell = Spec { glyph: '│', fg: FG_LINE, fg_at: 0.88 + 0.12 * self.road, ..*cell };
            }
        }
    }

    fn grid_cells(&self, sc: &Scene, spec: &mut [Spec], f: usize) {
        let (cols, hr) = (sc.cols, sc.hr);
        const E: f32 = 0.35;
        let zc = f as f32 + E;
        let lift = 0.8 * f as f32;
        let zv = zc + lift;
        let spacing = (cols as f32 / 22.0).max(3.5);
        let lines = (sc.cx / spacing).ceil() as i32 + 1;
        let mut dots = vec![0u8; cols * f];
        let flash = self.road;

        for k in -lines..=lines {
            if k.abs() == 1 {
                continue;
            }
            if k == 0 {
                self.center_dashes(sc, spec, f);
                continue;
            }
            let slope = (k.abs() as f32) * spacing / zv;
            if slope > 3.2 {
                continue;
            }
            for j in 0..f {
                let row = hr + j;
                let x_of = |u: f32| sc.cx + k as f32 * spacing * (u + lift + E) / zv;
                if slope < 1.7 {
                    let col = x_of(j as f32 + 0.5).floor();
                    if col < 0.0 || col >= cols as f32 {
                        continue;
                    }
                    let glyph = if slope < 0.5 {
                        '│'
                    } else if k < 0 {
                        '╱'
                    } else {
                        '╲'
                    };
                    let cell = &mut spec[row * cols + col as usize];
                    *cell = Spec { glyph, fg: FG_LINE, fg_at: 0.50 + 0.35 * flash, ..*cell };
                } else {
                    #[allow(clippy::needless_range_loop, reason = "t is the dot row: an index into each DOT_BIT row and a position along the line")]
                    for t in 0..4 {
                        let (a, b) = (x_of(j as f32 + t as f32 / 4.0) * 2.0, x_of(j as f32 + (t as f32 + 1.0) / 4.0) * 2.0);
                        let (lo, hi) = (a.min(b).floor() as i32, (a.max(b).ceil() as i32 - 1).max(a.min(b).floor() as i32));
                        for dot in lo.max(0)..=hi.min(2 * cols as i32 - 1) {
                            dots[j * cols + dot as usize / 2] |= DOT_BIT[dot as usize % 2][t];
                        }
                    }
                }
            }
        }
        for j in 0..f {
            for c in 0..cols {
                let bits = dots[j * cols + c];
                let cell = &mut spec[(hr + j) * cols + c];
                if bits != 0 && cell.glyph == ' ' {
                    cell.glyph = braille(bits);
                    cell.fg = FG_LINE;
                    cell.fg_at = 0.40 + 0.30 * flash;
                }
            }
        }

        let zl = f as f32 + E;
        for j in 1..f {
            let zlo = zl / ((j + 1) as f32 + E);
            let zhi = zl / (j as f32 + E);
            let count = ((zhi - CROSS_AT).floor() - (zlo - CROSS_AT).ceil() + 1.0).max(0.0);
            if count < 1.0 {
                continue;
            }
            for c in 0..cols {
                let cell = &mut spec[(hr + j) * cols + c];
                let glyph = if cell.glyph == '│' { '┼' } else { '─' };
                *cell = Spec { glyph, fg: FG_LINE, fg_at: 0.52 + 0.42 * flash + if count > 1.0 { 0.10 } else { 0.0 }, ..*cell };
            }
        }
    }

    #[cfg(test)]
    pub fn presence(&self) -> f32 {
        self.presence
    }

    #[cfg(test)]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    #[cfg(test)]
    pub fn sun_swell(&self) -> f32 {
        self.sun
    }

    #[cfg(test)]
    fn parts(&self) -> [f32; VOICES] {
        [self.sun, self.glow[0], self.glow[1], self.glow[2], self.glow[3], self.ridge, self.road]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectrum::Tap;

    #[test]
    fn a_fade_is_eased_at_both_ends() {
        assert_eq!((fade_curve(0.0), fade_curve(1.0)), (0.0, 1.0));
        assert!(fade_curve(0.05) < 0.01 && fade_curve(0.95) > 0.99);
    }

    #[test]
    fn the_visualizer_never_draws_the_marks_that_look_ugly() {
        let source = include_str!("synthwave.rs");
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        for mark in ['#', '=', '+', '-', '*', '@'] {
            for banned in [format!("b'{mark}'"), format!("'{mark}'"), format!("\"{mark}\"")] {
                assert!(!code.contains(&banned), "the visualizer draws {banned}");
            }
        }
    }

    #[test]
    fn the_visualizer_has_no_colors_or_theme_names_of_its_own() {
        let source = include_str!("synthwave.rs");
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        for t in crate::theme::THEMES {
            assert!(!code.contains(&format!("\"{}\"", t.name)), "the visualizer names the theme {}", t.name);
        }
        assert!(!code.contains("Color::Rgb"), "a color made here");
        assert!(!code.contains("Color::Indexed"), "a color made here");
        let hex_literal = |s: &str| s.match_indices("0x").any(|(i, _)| s[i + 2..].chars().take_while(|c| c.is_ascii_hexdigit() || *c == '_').filter(|c| *c != '_').count() == 6);
        assert!(!hex_literal(code), "a six-digit color literal");
    }

    fn drums(t: f32) -> f32 {
        use std::f32::consts::TAU;
        let ph = t % 0.5;
        let kick = 0.7 * (-ph * 11.0).exp() * (TAU * (48.0 * ph + 4.0 * (1.0 - (-20.0f32 * ph).exp()))).sin();
        let bass = 0.15 * (TAU * 55.0 * t).sin();
        let hat = 0.12 * (-((t + 0.25) % 0.5) * 40.0).exp() * (TAU * 7300.0 * t).sin();
        (kick + bass + hat).clamp(-1.0, 1.0)
    }

    struct Run {
        tap: Tap,
        sw: Synthwave,
        clock: Instant,
        t: f32,
        size: (usize, u16),
        seed: u64,
    }

    impl Run {
        fn new(size: (usize, u16), seed: u64) -> Run {
            Run { tap: Tap::new(), sw: Synthwave::new(), clock: Instant::now() + Duration::from_secs(5), t: 0.0, size, seed }
        }

        fn frame(&mut self, signal: Option<&dyn Fn(f32) -> f32>) {
            if let Some(signal) = signal {
                let mut w = self.tap.writer();
                for i in 0..1600 {
                    w.push(signal(self.t + i as f32 / 48_000.0));
                }
            }
            self.t += 1600.0 / 48_000.0;
            self.clock += Duration::from_millis(33);
            self.sw.update(&self.tap, self.size.0, self.size.1, self.seed, self.clock);
        }

        fn play(&mut self, secs: f32) {
            for _ in 0..(secs * 30.0) as usize {
                self.frame(Some(&drums));
            }
        }

        fn silence(&mut self, secs: f32) {
            for _ in 0..(secs * 30.0) as usize {
                self.frame(None);
            }
        }

        fn picture(&mut self) -> Buffer {
            self.sw.use_colors(OWN_COLORS, crate::theme::synthwave_colors);
            let area = Rect::new(0, 0, self.size.0 as u16, self.size.1);
            let mut buf = Buffer::empty(area);
            self.sw.paint(&mut buf, area);
            buf
        }
    }

    fn cells(buf: &Buffer) -> Vec<(String, Color, Color)> {
        buf.content.iter().map(|c| (c.symbol().to_string(), c.fg, c.bg)).collect()
    }

    #[test]
    fn only_lines_dots_and_peak_marks_are_ever_drawn() {
        let allowed = ['╱', '╲', '─', '│', '┼', '▲', '∧', '∨'];
        for size in [(30, 5), (60, 8), (85, 12), (120, 14), (200, 21)] {
            let mut run = Run::new(size, 7);
            let mut seen = std::collections::HashSet::new();
            for _ in 0..90 {
                run.frame(Some(&drums));
                for c in &run.picture().content {
                    for ch in c.symbol().chars() {
                        let ok = ch == ' ' || allowed.contains(&ch) || ('\u{2800}'..='\u{28FF}').contains(&ch);
                        assert!(ok, "{size:?}: {ch:?} ({:04x}) on screen", ch as u32);
                        seen.insert(ch);
                    }
                }
            }
            let must: &[char] = if size.1 >= 8 { &['▲', '╱', '╲', '─'] } else { &['╱', '╲', '─'] };
            for must in must {
                assert!(seen.contains(must), "{size:?}: never drew {must}");
            }
            assert!(seen.iter().any(|c| ('\u{2800}'..='\u{28FF}').contains(c)), "{size:?}: no braille");
        }
    }

    #[test]
    fn every_cell_is_painted_at_any_size() {
        for size in [(8, 4), (9, 5), (31, 5), (85, 13), (97, 34), (300, 60)] {
            let mut run = Run::new(size, 3);
            run.play(1.0);
            let buf = run.picture();
            assert_eq!(buf.content.len(), size.0 * size.1 as usize);
            assert!(buf.content.iter().all(|c| matches!(c.bg, Color::Rgb(..))), "{size:?}: a cell without its own background");
        }
    }

    #[test]
    fn the_same_music_gives_the_same_picture_and_the_song_does_not_change_how_it_looks() {
        let pic = |seed: u64| {
            let mut run = Run::new((85, 12), seed);
            run.play(3.0);
            cells(&run.picture())
        };
        assert_eq!(pic(7), pic(7));
        assert_eq!(pic(7), pic(8));
        assert_eq!(pic(7), pic(12345));
    }

    #[test]
    fn the_range_is_the_picture_in_a_window_of_its_size() {
        let h = range_outside(REF_COLS, REF_HR, REF_REACH).0;
        let mut right: Vec<i32> = Vec::new();
        for (i, v) in REF_RANGE.iter().enumerate() {
            right.push(*v as i32);
            if i > 0 && i + 1 < REF_RANGE.len() && REF_RANGE[i - 1] < *v && REF_RANGE[i + 1] < *v {
                right.push(*v as i32);
            }
        }
        assert_eq!(right.len(), REF_RANGE.len() + 2);
        right.truncate(REF_RANGE.len());
        let from = REF_REACH.ceil() as usize + 2;
        assert!(h[REF_COLS / 2..REF_COLS / 2 + from - 2].iter().all(|v| *v <= 2));
        assert_eq!(&h[REF_COLS / 2 + from..], &right[from..]);
        let left: Vec<i32> = right.iter().rev().copied().collect();
        let n = REF_COLS / 2;
        assert_eq!(&h[..n - from], &left[..n - from]);
    }

    #[test]
    fn the_range_keeps_its_look_in_every_window() {
        for cols in (40..=260).step_by(1) {
            for rows in 8..=40usize {
                let sc = Scene::new(cols, rows);
                let (hr, k) = (sc.hr as i32, range_heights(cols, sc.hr));
                let size = (cols, rows);
                let reach = sun_reach(cols, sc.hr);
                let off = |x: usize| (x as f32 + 0.5 - sc.cx).abs();
                for x in 0..cols {
                    assert_eq!(k[x], k[cols - 1 - x], "{size:?}: column {x} is not mirrored");
                    assert!((0..=hr).contains(&k[x]), "{size:?}: column {x} stands {} rows in a sky of {hr}", k[x]);
                    assert!(off(x) >= reach || k[x] == 0, "{size:?}: column {x} stands in front of the sun");
                }
                let half = &k[cols / 2..];
                if let Some(start) = half.iter().position(|v| *v >= VALLEY_FLOOR) {
                    assert!(half[start..].iter().all(|v| *v >= VALLEY_FLOOR), "{size:?}: a valley opens to the ground: {half:?}");
                }
                let mut tops = 0;
                let mut t = 1;
                while t < half.len() {
                    let mut end = t;
                    while end + 1 < half.len() && half[end + 1] == half[t] {
                        end += 1;
                    }
                    if end + 1 < half.len() && half[t - 1] < half[t] && half[end + 1] < half[t] {
                        tops += 1;
                        assert_eq!(end - t + 1, 2, "{size:?}: the top at {t} is not a point: {half:?}");
                    }
                    if end + 1 < half.len() && half[t] > VALLEY_FLOOR && (half[t - 1] - half[t]) * (half[end + 1] - half[t]) < 0 {
                        assert!(end - t < 3, "{size:?}: a shelf on the slope at {t}: {half:?}");
                    }
                    t = end + 1;
                }
                if sc.hr >= 6 && cols >= 60 {
                    let least = if cols >= 100 { 3 } else { 2 };
                    assert!(tops >= least, "{size:?}: only {tops} mountains: {half:?}");
                }
            }
        }
    }

    #[test]
    fn the_right_hand_range_is_mirrored_on_the_left_and_the_sun_is_left_clear() {
        let mirror = |s: &str| match s {
            "╱" => "╲".to_string(),
            "╲" => "╱".to_string(),
            other => other.to_string(),
        };
        for size in [(60, 10), (85, 12), (120, 21), (170, 28)] {
            let mut run = Run::new(size, 3);
            run.play(2.0);
            let buf = run.picture();
            let (cols, rows) = (size.0, size.1 as usize);
            let sc = Scene::new(cols, rows);
            let reach = sun_reach(cols, sc.hr);
            let mut mountains = 0;
            for y in 0..sc.hr {
                for x in 0..cols {
                    let from_center = (x as f32 + 0.5 - sc.cx).abs();
                    let sym = buf[(x as u16, y as u16)].symbol().to_string();
                    let is_mountain = matches!(sym.as_str(), "╱" | "╲" | "─" | "│" | "▲" | "∧" | "∨");
                    if from_center < reach {
                        assert!(!is_mountain, "{size:?}: a mountain mark '{sym}' stands in front of the sun at {x},{y}");
                        continue;
                    }
                    mountains += is_mountain as usize;
                    let other = buf[((cols - 1 - x) as u16, y as u16)].symbol().to_string();
                    assert_eq!(mirror(&sym), other, "{size:?}: {x},{y} is not mirrored");
                }
            }
            assert!(mountains > 20, "{size:?}: only {mountains} mountain marks");
        }
    }

    #[test]
    fn the_grid_steps_on_the_beat_and_does_not_glide() {
        let mut run = Run::new((85, 12), 7);
        run.play(2.0);
        let (mut still, mut moved, mut traveled) = (0, 0, 0.0f32);
        let mut last = run.sw.phase();
        for _ in 0..150 {
            run.frame(Some(&drums));
            let now = run.sw.phase();
            let d = (now - last).rem_euclid(1.0);
            let d = if d > 0.5 { 0.0 } else { d };
            if d < 0.004 { still += 1 } else { moved += 1 }
            traveled += d;
            last = now;
        }
        assert!(traveled > 1.5 && traveled < 6.0, "traveled {traveled}");
        assert!(still > moved, "gliding: still {still}, moving {moved}");
    }

    #[test]
    fn the_mountains_stand_still() {
        let mut run = Run::new((120, 21), 7);
        run.play(2.0);
        let rows = run.size.1 as usize * 58 / 100;
        let part = |buf: &Buffer| -> Vec<String> {
            let mut v = Vec::new();
            for y in 0..rows {
                for x in (0..120).filter(|x| *x < 18 || *x >= 102) {
                    v.push(buf[(x as u16, y as u16)].symbol().to_string());
                }
            }
            v
        };
        let first = part(&run.picture());
        for _ in 0..120 {
            run.frame(Some(&drums));
            assert_eq!(part(&run.picture()), first, "a mountain moved");
        }
    }

    #[test]
    fn the_inside_of_the_mountains_is_filled_with_even_triangles() {
        let mut run = Run::new((120, 21), 7);
        run.play(3.0);
        let buf = run.picture();
        let tri: Vec<_> = buf.content.iter().filter(|c| c.symbol() == "▲").map(|c| (c.fg, c.bg)).collect();
        assert!(tri.len() > 200, "only {} triangles", tri.len());
        let most = tri.iter().map(|k| tri.iter().filter(|o| *o == k).count()).max().unwrap();
        assert!(most * 10 >= tri.len() * 8, "the triangles are not even: {most} of {}", tri.len());
    }

    #[test]
    fn the_lines_across_stand_still_and_the_middle_line_is_slotted() {
        let mut run = Run::new((120, 21), 7);
        run.play(2.0);
        let across = |buf: &Buffer| -> Vec<u16> { (12..21).filter(|y| buf[(10, *y)].symbol() == "─").collect() };
        let middle = |buf: &Buffer| -> String { (12..21).map(|y| if matches!(buf[(60, y)].symbol(), "│" | "┼") { '│' } else { ' ' }).collect() };
        let first = across(&run.picture());
        assert!(first.len() >= 3, "no lines across: {first:?}");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..150 {
            run.frame(Some(&drums));
            let buf = run.picture();
            assert_eq!(across(&buf), first, "a line across moved");
            seen.insert(middle(&buf));
        }
        assert!(seen.len() > 3, "the dashes never moved: {seen:?}");
        for m in &seen {
            let dashes = m.chars().filter(|c| *c == '│').count();
            assert!((2..=7).contains(&dashes), "not slotted: {m:?}");
            assert!(m.contains(' '), "no gaps: {m:?}");
        }
    }

    #[test]
    fn the_sun_swells_on_a_beat_and_settles_between_beats() {
        let mut run = Run::new((85, 12), 7);
        run.play(2.0);
        let (mut high, mut low) = (0.0f32, 1.0f32);
        for _ in 0..60 {
            run.frame(Some(&drums));
            high = high.max(run.sw.sun_swell());
            low = low.min(run.sw.sun_swell());
        }
        assert!(high > 0.55, "no swell on the beat ({high})");
        assert!(low < 0.3, "never settles ({low})");
    }

    #[test]
    fn each_part_moves_with_its_own_range_and_no_other() {
        for (which, (lo, up)) in VOICE_HZ.into_iter().enumerate() {
            let tone = (lo * up).sqrt();
            let notes = move |t: f32| {
                let ph = t % 0.5;
                0.5 * (ph / 0.02).min(1.0) * (-ph * 6.0).exp() * (std::f32::consts::TAU * tone * t).sin()
            };
            let mut run = Run::new((85, 12), 7);
            let mut most = [0.0f32; VOICES];
            for f in 0..150 {
                run.frame(Some(&notes));
                if f >= 60 {
                    for (m, v) in most.iter_mut().zip(run.sw.parts()) {
                        *m = m.max(v);
                    }
                }
            }
            for (part, moved) in most.into_iter().enumerate() {
                if part == which {
                    assert!(moved > 0.4, "{tone:.0} Hz: its part ({part}) only moved {moved:.2}: {most:?}");
                } else {
                    assert!(moved < 0.15, "{tone:.0} Hz moved part {part} too ({moved:.2}): {most:?}");
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn synthwave_parts_report() {
        let bytes = std::fs::read(std::env::var("SYNTH_AUDIO").unwrap()).unwrap();
        let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let (tap, mut sw) = (Tap::new(), Synthwave::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        let (mut sum, mut most, mut hits, mut frames) = ([0.0f32; VOICES], [0.0f32; VOICES], [0usize; VOICES], 0);
        for f in 0..audio.len() / 1600 {
            let mut w = tap.writer();
            for &s in &audio[f * 1600..(f + 1) * 1600] {
                w.push(s);
            }
            drop(w);
            clock += Duration::from_secs_f64(1.0 / 30.0);
            sw.update(&tap, 101, 13, 7, clock);
            frames += 1;
            for (i, v) in sw.parts().into_iter().enumerate() {
                sum[i] += v;
                most[i] = most[i].max(v);
                hits[i] += (sw.bands.voices[i].hit > 0.0) as usize;
            }
        }
        let secs = frames as f32 / 30.0;
        for (i, name) in ["sun", "glow 1", "glow 2", "glow 3", "glow 4", "ridge", "road"].into_iter().enumerate() {
            println!("{name:7} {:?} Hz: mean {:.2}  most {:.2}  hits {:.1}/s", VOICE_HZ[i], sum[i] / frames as f32, most[i], hits[i] as f32 / secs);
        }
    }

    #[test]
    fn the_inside_of_the_sun_is_one_color_on_every_row() {
        for size in [(90, 11), (120, 14)] {
            let mut run = Run::new(size, 3);
            run.play(1.0);
            for _ in 0..120 {
                run.frame(Some(&drums));
                let buf = run.picture();
                let x = size.0 as u16 / 2;
                let sun: Vec<_> = (0..size.1)
                    .map(|y| &buf[(x, y)])
                    .filter(|c| c.symbol().chars().all(|ch| ('\u{2801}'..='\u{28FF}').contains(&ch)))
                    .map(|c| (c.fg, c.bg))
                    .collect();
                assert!(sun.len() >= 4, "{size:?}: the sun is {} rows", sun.len());
                assert!(sun[1..].iter().all(|c| *c == sun[1]), "{size:?}: rows differ: {sun:?}");
            }
        }
    }

    #[test]
    fn the_moonlit_sky_gets_its_shooting_star_at_2_seconds() {
        for size in [(90, 11), (120, 14), (60, 8)] {
            for seed in [1u64, 7, 42, 12_630_764_548_812_270_369] {
                let mut run = Run::new(size, seed);
                run.sw = Synthwave::moonlit();
                run.play(1.8);
                let before: Vec<String> = run.picture().content.iter().map(|c| c.symbol().to_string()).collect();
                let mut seen = false;
                for _ in 0..15 {
                    run.frame(Some(&drums));
                    run.sw.use_colors(MOON_COLORS, crate::theme::synthmoonwave_colors);
                    let area = Rect::new(0, 0, size.0 as u16, size.1);
                    let mut buf = Buffer::empty(area);
                    run.sw.paint(&mut buf, area);
                    seen |= buf.content.iter().zip(&before).any(|(c, b)| c.symbol() == "✦" && b != "✦");
                }
                assert!(seen, "{size:?} seed {seed}: no shooting star at 2 s");
            }
        }
    }

    #[test]
    fn the_sun_is_six_rows_tall_swollen_or_not() {
        let mut run = Run::new((101, 13), 7);
        let sc = Scene::new(101, 13);
        assert_eq!((sc.hr, sun_rows(101, sc.hr)), (8, 6));
        run.play(1.0);
        let (mut least, mut most) = (1.0f32, 0.0f32);
        for _ in 0..90 {
            run.frame(Some(&drums));
            let swell = run.sw.sun_swell();
            least = least.min(swell);
            most = most.max(swell);
            let buf = run.picture();
            let braille = |y: usize| (0..101).any(|x| buf[(x as u16, y as u16)].symbol().chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)));
            let rows: Vec<usize> = (0..sc.hr).filter(|y| braille(*y)).collect();
            assert_eq!(rows, (2..8).collect::<Vec<_>>(), "swell {swell}");
        }
        assert!(most > 0.6 && least < 0.3, "the sun did not both swell and settle ({least}..{most})");
    }

    #[test]
    fn without_a_beat_the_grid_crawls() {
        let tone = |t: f32| 0.3 * (std::f32::consts::TAU * 440.0 * t).sin();
        let mut run = Run::new((85, 12), 7);
        for _ in 0..30 {
            run.frame(Some(&tone));
        }
        let before = run.sw.phase();
        for _ in 0..150 {
            run.frame(Some(&tone));
        }
        let moved = (run.sw.phase() - before).rem_euclid(1.0);
        assert!(moved > 0.15 && moved < 0.9, "moved {moved}");
    }

    #[test]
    fn the_picture_fades_into_one_flat_color_when_the_music_stops_and_comes_back() {
        let mut run = Run::new((85, 12), 7);
        run.play(3.0);
        assert!(run.sw.is_active() && run.sw.presence() > 0.99);
        let playing = cells(&run.picture());
        run.silence(0.5);
        let half = cells(&run.picture());
        let same = playing.iter().zip(&half).filter(|(a, b)| a == b).count();
        assert!(same as f32 > 0.5 * playing.len() as f32, "gone too fast: {same} of {}", playing.len());
        run.silence(4.0);
        let flat = cells(&run.picture());
        assert!(flat.iter().all(|c| c == &flat[0] && c.0 == " "), "not flat");
        assert!(!run.sw.is_active(), "still asking for frames");
        run.play(0.6);
        assert!(run.sw.presence() > 0.99);
        assert!(cells(&run.picture()).iter().any(|c| c.0 != " "));
    }

    #[test]
    fn few_cells_change_from_one_frame_to_the_next() {
        let mut run = Run::new((85, 12), 7);
        run.play(2.0);
        let mut last = cells(&run.picture());
        let (mut changed, mut total) = (0usize, 0usize);
        for _ in 0..240 {
            run.frame(Some(&drums));
            let now = cells(&run.picture());
            changed += now.iter().zip(&last).filter(|(a, b)| a != b).count();
            total += now.len();
            last = now;
        }
        let share = changed as f32 / total as f32;
        assert!(share < 0.25, "{:.1}% of the cells change per frame", share * 100.0);
    }

    #[test]
    #[ignore]
    fn synthwave_cells_dump() {
        use std::fs;
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let (cols, rows) = (env("SYNTH_COLS", "85").parse::<usize>().unwrap(), env("SYNTH_ROWS", "12").parse::<u16>().unwrap());
        let seed: u64 = env("SYNTH_SEED", "7").parse().unwrap();
        let out = env("SYNTH_OUT", "/tmp/synth_cells");
        fs::create_dir_all(&out).unwrap();
        let bytes = fs::read(env("SYNTH_AUDIO", "")).unwrap();
        let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let moments: Vec<f32> = env("SYNTH_AT", "5.0").split(',').map(|s| s.trim().parse().unwrap()).collect();
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            other => panic!("not a plain color: {other:?}"),
        };
        let painted = |sw: &mut Synthwave| {
            sw.use_colors(OWN_COLORS, crate::theme::synthwave_colors);
            let area = Rect::new(0, 0, cols as u16, rows);
            let mut buf = Buffer::empty(area);
            sw.paint(&mut buf, area);
            let mut text = format!("{cols} {rows}\n");
            for y in 0..rows {
                for x in 0..cols as u16 {
                    let c = &buf[(x, y)];
                    let sym = if c.symbol().is_empty() { " " } else { c.symbol() };
                    text.push_str(sym);
                    text.push_str(&hex(c.fg));
                    text.push_str(&hex(c.bg));
                }
                text.push('\n');
            }
            text
        };
        let (tap, mut sw) = (Tap::new(), Synthwave::new());
        let mut clock = Instant::now() + Duration::from_secs(5);
        for f in 0..audio.len() / 1600 {
            let mut w = tap.writer();
            for &s in &audio[f * 1600..(f + 1) * 1600] {
                w.push(s);
            }
            drop(w);
            clock += Duration::from_secs_f64(1.0 / 30.0);
            sw.update(&tap, cols, rows, seed, clock);
            let text = painted(&mut sw);
            let now = (f + 1) as f32 / 30.0;
            if std::env::var("SYNTH_CYCLE").is_ok() {
                fs::write(format!("{out}/frame_{f:05}.cells"), text).unwrap();
                let mut cap = fs::OpenOptions::new().create(true).append(true).open(format!("{out}/captions.txt")).unwrap();
                std::io::Write::write_all(&mut cap, format!("Synthwave  {:.1}s  swell {:.2}  grid {:.2}\n", now, sw.sun_swell(), sw.phase()).as_bytes()).unwrap();
                continue;
            }
            for &m in &moments {
                if (now - m).abs() < 0.5 / 30.0 {
                    fs::write(format!("{out}/Synthwave_{m:.2}s.cells"), &text).unwrap();
                }
            }
        }
    }
}
