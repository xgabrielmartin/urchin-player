use std::f32::consts::{PI, TAU};
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::bokeh::{
    fade_curve, hash01, pace_for, sticky, Bands, Tempo, AT_REST, BPM_REF, CLASSES, FADE_IN, FADE_OUT, MIN_STEP, PACE_TAU, PULSE_ATTACK, PULSE_FALL,
    SILENCE_HOLD, SONG_SHARE, STALE,
};
use crate::spectrum::Tap;
use crate::theme::{club_lights_colors, ClubLightsColors};

const BEATS_PER_SWING: f32 = 4.0;
const SWAY_DEG: f32 = 26.0;
const FAN_DEG: f32 = 7.0;
const PHASE_STEP: f32 = 0.28;

const ORDER_10: [usize; 10] = [0, 5, 2, 7, 4, 9, 1, 6, 3, 8];
const ORDER_6: [usize; 6] = [0, 5, 2, 8, 4, 9];
const ORDER_4: [usize; 4] = [0, 6, 3, 9];

const LEVELS: [f32; 8] = [0.0, 12.0, 30.0, 56.0, 92.0, 138.0, 196.0, 255.0];

const BEAM_BASE: f32 = 0.50;
const BEAM_PULSE: f32 = 0.90;
const LEADERS: usize = 3;
const LEAD_MIN: f32 = 0.15;
const LEAD_LIFT: f32 = 0.25;
const LEAD_RISE: f32 = 20.0;
const LEAD_FALL: f32 = 8.0;
const HAZE_SHARE: f32 = 0.42;
const SEQUIN_BASE: f32 = 0.42;
const SEQUIN_GAIN: f32 = 0.58;
const GLOW_LOW: f32 = 0.16;
const GLOW_HIGH: f32 = 0.36;
const HOT: f32 = 90.0;
const HOT_FROM: f32 = 1.5;

fn level_of(v: f32) -> usize {
    (1..LEVELS.len()).fold(0, |best, i| if (v - LEVELS[i]).abs() < (v - LEVELS[best]).abs() { i } else { best })
}

fn beams_for(cols: usize) -> &'static [usize] {
    if cols >= 60 {
        &ORDER_10
    } else if cols >= 34 {
        &ORDER_6
    } else {
        &ORDER_4
    }
}

#[derive(Clone, Copy)]
struct Look {
    ground: [f32; 3],
    hues: [[f32; 3]; CLASSES],
}

impl Look {
    fn from(c: ClubLightsColors) -> Look {
        let ch = |v: u32| [((v >> 16) & 0xff) as f32, ((v >> 8) & 0xff) as f32, (v & 0xff) as f32];
        let mut hues = [[0.0; 3]; CLASSES];
        for (h, &v) in hues.iter_mut().zip(c.beams.iter()) {
            *h = ch(v);
        }
        Look { ground: ch(c.ground), hues }
    }
}

#[derive(Clone, Copy)]
struct Sequin {
    glyph: char,
    shine: f32,
    period: f32,
    phase: f32,
    hue: usize,
    haze: f32,
}

fn make_cells(cols: usize, rows: usize, seed: u64) -> Vec<Sequin> {
    let mut cells = Vec::with_capacity(cols * rows);
    for y in 0..rows {
        for x in 0..cols {
            let (xs, ys) = (x as u64, y as u64);
            let size = hash01(seed, xs, ys * 8);
            let glyph = if x % 2 != 0 {
                ' '
            } else if size < 0.22 {
                '·'
            } else if size < 0.80 {
                '∙'
            } else {
                '•'
            };
            let patch = hash01(seed ^ 0x51ED, xs / 4, (ys / 2) * 8 + 7);
            cells.push(Sequin {
                glyph,
                shine: 0.35 + 0.65 * hash01(seed, xs, ys * 8 + 1),
                period: 5.0 + 8.0 * hash01(seed, xs, ys * 8 + 2),
                phase: TAU * hash01(seed, xs, ys * 8 + 3),
                hue: ((hash01(seed, xs, ys * 8 + 4) * CLASSES as f32) as usize).min(CLASSES - 1),
                haze: (patch - 0.5) * 2.0,
            });
        }
    }
    cells
}

struct Beam {
    sx: f32,
    ux: f32,
    uy: f32,
    spread: f32,
    gain: f32,
    color: [f32; 3],
}

pub struct ClubLights {
    look: Look,
    bands: Bands,
    tempo: Tempo,
    pace: f32,
    turns: f64,
    time: f32,
    columns: usize,
    rows: u16,
    seed: u64,
    pulse: [f32; CLASSES],
    lead: [f32; CLASSES],
    cells: Vec<Sequin>,
    cells_for: (usize, usize, u64),
    shown: Vec<[u8; 6]>,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
}

impl Default for ClubLights {
    fn default() -> Self {
        ClubLights::new()
    }
}

impl ClubLights {
    pub fn new() -> ClubLights {
        ClubLights {
            look: Look::from(club_lights_colors()),
            bands: Bands::new(),
            tempo: Tempo::new(),
            pace: 1.0,
            turns: 0.0,
            time: 0.0,
            columns: 0,
            rows: 0,
            seed: 0,
            pulse: [0.0; CLASSES],
            lead: [0.0; CLASSES],
            cells: Vec::new(),
            cells_for: (0, 0, 0),
            shown: Vec::new(),
            last: None,
            last_written: 0,
            quiet_since: None,
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
        }
    }

    pub fn is_active(&self) -> bool {
        let fading = if self.silent { self.presence > 0.0 } else { self.presence < 1.0 };
        fading || self.pulse.iter().any(|&p| p > AT_REST)
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        (self.columns, self.rows, self.seed) = (columns.max(1), rows.max(1), seed);
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
        if self.silent {
            self.presence = (self.presence - dt / FADE_OUT).max(0.0);
        } else {
            self.presence = (self.presence + dt / FADE_IN).min(1.0);
        }

        self.time += dt;
        self.pace += (pace_for(self.tempo.bpm) - self.pace) * (1.0 - (-dt / PACE_TAU).exp());
        self.turns += (dt * self.pace * BPM_REF / 60.0 / BEATS_PER_SWING) as f64;

        let up = 1.0 - (-dt / PULSE_ATTACK).exp();
        for c in 0..CLASSES {
            let want = ((1.0 - SONG_SHARE) * self.bands.target[c] + SONG_SHARE * self.bands.loudness).clamp(0.0, 1.0);
            let p = self.pulse[c];
            self.pulse[c] = if want > p { p + (want - p) * up } else { want.max(p - PULSE_FALL * dt) };
        }
        let mut order: Vec<usize> = (0..CLASSES).collect();
        order.sort_by(|&a, &b| self.pulse[b].total_cmp(&self.pulse[a]));
        for (rank, &c) in order.iter().enumerate() {
            let (to, rate) = if rank < LEADERS && self.pulse[c] >= LEAD_MIN { (1.0, LEAD_RISE) } else { (0.0, LEAD_FALL) };
            self.lead[c] += (to - self.lead[c]) * (1.0 - (-rate * dt).exp());
        }
    }

    fn beams(&self, cols: usize) -> Vec<Beam> {
        let order = beams_for(cols);
        let n = order.len();
        let swing = self.turns.fract() as f32;
        let mid = (n as f32 - 1.0) / 2.0;
        order
            .iter()
            .enumerate()
            .map(|(slot, &band)| {
                let pulse = (self.pulse[band] + LEAD_LIFT * self.lead[band]).min(1.0);
                let fan = if mid > 0.0 { -(slot as f32 - mid) / mid * FAN_DEG } else { 0.0 };
                let phase = PI * (slot % 2) as f32 + PHASE_STEP * slot as f32;
                let angle = (fan + SWAY_DEG * (TAU * swing + phase).sin()).to_radians();
                Beam {
                    sx: (slot as f32 + 0.5) / n as f32 * cols as f32,
                    ux: angle.sin(),
                    uy: angle.cos(),
                    spread: (8.0 + 4.0 * pulse).to_radians().tan(),
                    gain: BEAM_BASE + BEAM_PULSE * pulse,
                    color: [self.look.hues[band][0] / 255.0, self.look.hues[band][1] / 255.0, self.look.hues[band][2] / 255.0],
                }
            })
            .collect()
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        if area.is_empty() || area.width as usize != self.columns || area.height != self.rows {
            return;
        }
        let (cols, rows) = (self.columns, area.height as usize);
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
        let rgb = |c: [f32; 3]| Color::Rgb(c[0].round().clamp(0.0, 255.0) as u8, c[1].round().clamp(0.0, 255.0) as u8, c[2].round().clamp(0.0, 255.0) as u8);
        let (ground_rgb, hues) = (self.look.ground, self.look.hues);
        let ground = rgb(ground_rgb);

        let shows = fade_curve(self.presence);
        if shows <= 0.0 {
            self.shown.clear();
            for y in 0..rows {
                for c in 0..cols {
                    put(area.x as usize + c, area.y as usize + y, ' ', ground, ground);
                }
            }
            return;
        }

        if self.cells_for != (cols, rows, self.seed) {
            self.cells = make_cells(cols, rows, self.seed);
            self.cells_for = (cols, rows, self.seed);
            self.shown.clear();
        }
        if self.shown.len() != cols * rows {
            let g = [level_of(ground_rgb[0]) as u8, level_of(ground_rgb[1]) as u8, level_of(ground_rgb[2]) as u8];
            self.shown = vec![[g[0], g[1], g[2], g[0], g[1], g[2]]; cols * rows];
        }

        let beams = self.beams(cols);
        let h2 = 2.0 * rows as f32;
        let w0 = 1.5 + 0.012 * h2;
        let reach = 1.35 * h2;
        let lamp_x: Vec<usize> = beams.iter().map(|b| (b.sx.floor() as usize).min(cols - 1)).collect();
        let lamp_of = |x: usize| lamp_x.iter().position(|&l| l == x);

        for y in 0..rows {
            let py = 2.0 * y as f32 + 1.0;
            for x in 0..cols {
                let px = x as f32 + 0.5;
                let cell = self.cells[y * cols + x];
                let mut light = [0.0f32; 3];
                for b in &beams {
                    let (dx, dy) = (px - b.sx, py);
                    let s = dx * b.ux + dy * b.uy;
                    if s <= 0.0 {
                        continue;
                    }
                    let w = w0 + s * b.spread;
                    let p = (dx * b.uy - dy * b.ux).abs();
                    if p >= w {
                        continue;
                    }
                    let t = 1.0 - (p / w) * (p / w);
                    let strength = t * t.sqrt() * (-s / reach).exp() * b.gain;
                    for (l, c) in light.iter_mut().zip(b.color) {
                        *l += strength * c;
                    }
                }
                let glitter = 0.78 + 0.22 * (TAU * self.time / cell.period + cell.phase).sin();
                let dots = cell.shine * glitter;
                let sparkle = (dots * 2.0).clamp(0.0, 1.4);
                let hot = ((light[0] + light[1] + light[2]) - HOT_FROM).clamp(0.0, 1.0) * HOT;
                let wall = 0.85 + 0.15 * cell.haze.tanh();
                let own = hues[cell.hue];
                let (mut fg, mut bg) = ([0.0f32; 3], [0.0f32; 3]);
                for k in 0..3 {
                    bg[k] = ground_rgb[k] * wall + 255.0 * light[k] * HAZE_SHARE + hot;
                    fg[k] = ground_rgb[k] * wall + 255.0 * (light[k] * (SEQUIN_BASE + SEQUIN_GAIN * sparkle) + own[k] / 255.0 * (GLOW_LOW + GLOW_HIGH * dots)) + hot;
                    bg[k] = ground_rgb[k] + (bg[k] - ground_rgb[k]) * shows;
                    fg[k] = ground_rgb[k] + (fg[k] - ground_rgb[k]) * shows;
                }
                let mut glyph = cell.glyph;
                if y == 0 {
                    if let Some(i) = lamp_of(x) {
                        let b = &beams[i];
                        let hit = ((b.gain - BEAM_BASE) / BEAM_PULSE).clamp(0.0, 1.0);
                        for k in 0..3 {
                            let lamp = b.color[k] * 255.0 * (0.55 + 0.45 * hit) + 70.0 * hit;
                            fg[k] = ground_rgb[k] + (lamp - ground_rgb[k]) * shows;
                        }
                        glyph = '▀';
                    }
                }
                let state = &mut self.shown[y * cols + x];
                let mut out = [[0.0f32; 3]; 2];
                for k in 0..3 {
                    let lb = sticky(&mut state[3 + k], level_of(bg[k]), LEVELS.len() - 1);
                    out[1][k] = LEVELS[lb];
                    if glyph == ' ' {
                        state[k] = lb as u8;
                        out[0][k] = LEVELS[lb];
                    } else {
                        let lf = sticky(&mut state[k], level_of(fg[k]), LEVELS.len() - 1);
                        out[0][k] = LEVELS[lf];
                    }
                }
                put(area.x as usize + x, area.y as usize + y, glyph, rgb(out[0]), rgb(out[1]));
            }
        }
    }

    #[cfg(test)]
    pub fn pulses(&self) -> [f32; CLASSES] {
        self.pulse
    }

    #[cfg(test)]
    pub fn presence(&self) -> f32 {
        self.presence
    }

    #[cfg(test)]
    pub fn swing(&self) -> (f32, f32, f64) {
        (self.tempo.bpm, self.pace, self.turns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn rate_of(tap: &Tap) -> f32 {
        tap.rate() as f32
    }

    fn run(s: &mut ClubLights, tap: &Tap, (columns, rows): (usize, u16), start: f32, secs: f32, signal: impl Fn(f32) -> f32, clock: &mut Instant) {
        let rate = rate_of(tap);
        let per = (rate / 30.0) as usize;
        for f in 0..(secs * 30.0) as usize {
            let mut w = tap.writer();
            for i in 0..per {
                w.push(signal(start + (f * per + i) as f32 / rate));
            }
            drop(w);
            *clock += Duration::from_millis(33);
            s.update(tap, columns, rows, 7, *clock);
        }
    }

    fn music(t: f32) -> f32 {
        let beat = t % 0.5;
        let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 3.9 * (1.0 - (-18.0 * beat).exp()))).sin();
        let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
        let chord: f32 = [220.0f32, 277.2, 329.6].iter().flat_map(|&f| (1..4).map(move |k| 0.05 / k as f32 * (TAU * f * k as f32 * t).sin())).sum();
        let lead = 0.06 * (TAU * 880.0 * t).sin() + 0.03 * (TAU * 3520.0 * t).sin() + 0.02 * (TAU * 9000.0 * t).sin();
        (kick + bass + chord + lead).clamp(-1.0, 1.0)
    }

    fn at_tempo(bpm: f32) -> impl Fn(f32) -> f32 {
        move |t| {
            let beat = t % (60.0 / bpm);
            let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 70.0 / 18.0 * (1.0 - (-18.0 * beat).exp()))).sin();
            let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
            (kick + bass).clamp(-1.0, 1.0)
        }
    }

    fn painted(s: &mut ClubLights, (columns, rows): (usize, u16)) -> Buffer {
        let area = Rect::new(0, 0, columns as u16, rows);
        let mut buf = Buffer::empty(area);
        s.paint(&mut buf, area);
        buf
    }

    fn playing(size: (usize, u16)) -> (ClubLights, Tap) {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        run(&mut s, &tap, size, 0.0, 1.5, music, &mut clock);
        (s, tap)
    }

    fn symbol(buf: &Buffer, x: u16, y: u16) -> String {
        let s = buf[(x, y)].symbol();
        if s.is_empty() { " ".into() } else { s.into() }
    }

    fn rgb_of(c: Color) -> [f32; 3] {
        match c {
            Color::Rgb(r, g, b) => [r as f32, g as f32, b as f32],
            _ => [0.0; 3],
        }
    }

    fn hue_of(c: [f32; 3]) -> (f32, f32) {
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let d = hi - lo;
        if d <= 0.0 || hi <= 0.0 {
            return (0.0, 0.0);
        }
        let h = if hi == c[0] {
            60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
        } else if hi == c[1] {
            60.0 * ((c[2] - c[0]) / d + 2.0)
        } else {
            60.0 * ((c[0] - c[1]) / d + 4.0)
        };
        (h, d / hi)
    }

    #[test]
    fn the_ground_is_on_the_color_steps_and_the_beams_are_ten_different_colors() {
        let look = Look::from(club_lights_colors());
        assert!(look.ground.iter().all(|v| LEVELS.contains(v)), "the ground is not on the color steps");
        for i in 0..CLASSES {
            for j in i + 1..CLASSES {
                let d: f32 = (0..3).map(|k| (look.hues[i][k] - look.hues[j][k]).abs()).sum();
                assert!(d > 60.0, "beam colors {i} and {j} are too alike ({d})");
            }
        }
    }

    #[test]
    fn nothing_played_is_one_flat_color() {
        let (tap, mut s) = (Tap::new(), ClubLights::new());
        s.update(&tap, 85, 12, 7, Instant::now());
        let buf = painted(&mut s, (85, 12));
        let flat = rgb_color_of(Look::from(club_lights_colors()).ground);
        assert!(buf.content.iter().all(|c| c.bg == flat && c.fg == flat && c.symbol() == " "));
        assert!(!s.is_active());
    }

    fn rgb_color_of(c: [f32; 3]) -> Color {
        Color::Rgb(c[0] as u8, c[1] as u8, c[2] as u8)
    }

    #[test]
    fn only_sequins_and_lamps_are_drawn() {
        let (mut s, _tap) = playing((85, 12));
        let buf = painted(&mut s, (85, 12));
        let (mut sequins, mut lamps) = (0, 0);
        for y in 0..12 {
            for x in 0..85 {
                let g = symbol(&buf, x, y);
                assert!(" ·∙•▀".contains(&g), "unexpected {g:?} at {x},{y}");
                if g == "▀" {
                    assert_eq!(y, 0, "a lamp below the top row");
                    lamps += 1;
                } else if g != " " {
                    assert_eq!(x % 2, 0, "a sequin in an odd column");
                    sequins += 1;
                }
            }
        }
        assert_eq!(lamps, 10, "ten beams in a wide window");
        assert!(sequins > 12 * 40, "only {sequins} sequins");
    }

    #[test]
    fn every_sequin_keeps_its_character_for_the_whole_song() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        let mut first: Option<Vec<String>> = None;
        for step in 0..10 {
            run(&mut s, &tap, (85, 12), step as f32 * 0.5, 0.5, music, &mut clock);
            let buf = painted(&mut s, (85, 12));
            let all: Vec<String> = (0..12u16).flat_map(|y| (0..85u16).map(move |x| (x, y))).map(|(x, y)| symbol(&buf, x, y)).collect();
            match &first {
                None => first = Some(all),
                Some(f) => assert_eq!(f, &all, "a sequin changed its character"),
            }
        }
    }

    #[test]
    fn the_beams_are_many_colors() {
        let (mut s, _tap) = playing((120, 24));
        let buf = painted(&mut s, (120, 24));
        let mut families = [0usize; 12];
        for c in &buf.content {
            let (h, sat) = hue_of(rgb_of(c.bg));
            let lum = rgb_of(c.bg).iter().copied().fold(0.0, f32::max);
            if sat > 0.5 && lum > 40.0 {
                families[(h / 30.0) as usize % 12] += 1;
            }
        }
        let many = families.iter().filter(|&&n| n >= 5).count();
        assert!(many >= 6, "only {many} hue families lit: {families:?}");
    }

    #[test]
    fn the_beams_swing() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        run(&mut s, &tap, (120, 24), 0.0, 1.0, music, &mut clock);
        let spots = |s: &mut ClubLights| {
            let buf = painted(s, (120, 24));
            (0..120u16).map(|x| rgb_of(buf[(x, 23)].bg).iter().sum::<f32>()).collect::<Vec<f32>>()
        };
        let before = spots(&mut s);
        run(&mut s, &tap, (120, 24), 1.0, 0.4, music, &mut clock);
        let after = spots(&mut s);
        let moved = before.iter().zip(&after).filter(|(a, b)| (*a - *b).abs() > 25.0).count();
        assert!(moved >= 12, "only {moved} cells of the bottom row changed in 0.4 s");
    }

    fn swings_over(bpm: f32, warm: f32, secs: f32) -> (f32, f64) {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        run(&mut s, &tap, (85, 12), 0.0, warm, at_tempo(bpm), &mut clock);
        let (_, _, before) = s.swing();
        run(&mut s, &tap, (85, 12), warm, secs, at_tempo(bpm), &mut clock);
        let (found, _, after) = s.swing();
        (found, after - before)
    }

    #[test]
    fn one_swing_is_four_beats_so_a_faster_song_swings_faster() {
        let mut rates = Vec::new();
        for bpm in [90.0f32, 120.0, 150.0] {
            let (found, swings) = swings_over(bpm, 20.0, 8.0);
            assert!((found - bpm).abs() <= 0.06 * bpm, "{bpm} bpm read as {found}");
            let beats = bpm as f64 * 8.0 / 60.0;
            let per_swing = beats / swings;
            assert!((per_swing - 4.0).abs() < 0.4, "{bpm} bpm: {per_swing:.2} beats a swing");
            rates.push(swings);
        }
        assert!(rates[0] < rates[1] && rates[1] < rates[2], "{rates:?}");
        assert!(rates[2] / rates[0] > 1.5, "{} against {}", rates[2], rates[0]);
    }

    #[test]
    fn without_a_beat_it_swings_as_at_120() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        let tone = |t: f32| {
            let n = (t as f64 * 48_000.0).round();
            0.3 * (std::f64::consts::TAU * 220.0 * n / 48_000.0).sin() as f32
        };
        run(&mut s, &tap, (85, 12), 0.0, 10.0, tone, &mut clock);
        let (_, pace, _) = s.swing();
        assert!((pace - 1.0).abs() < 0.05, "a steady tone changed the swing's speed ({pace})");
    }

    #[test]
    fn each_range_lights_its_own_beam() {
        let (tap, mut s) = (Tap::new(), ClubLights::new());
        tap.set_rate(48_000);
        let mut clock = Instant::now();
        run(&mut s, &tap, (85, 12), 0.0, 1.5, |t| 0.5 * (TAU * 60.0 * t).sin(), &mut clock);
        let p = s.pulses();
        assert!(p[0] > 0.5 && p[9] < 0.2, "a 60 Hz tone: {p:?}");
        run(&mut s, &tap, (85, 12), 1.5, 1.5, |t| 0.5 * (TAU * 9_000.0 * t).sin(), &mut clock);
        let p = s.pulses();
        assert!(p[8] > 0.5 && p[0] < 0.2, "a 9 kHz tone: {p:?}");
    }

    #[test]
    fn it_fits_any_window() {
        for &(cols, rows) in &[(1usize, 1u16), (2, 1), (8, 2), (20, 4), (40, 6), (85, 12), (120, 24), (200, 50), (300, 12), (30, 60)] {
            let (mut s, _tap) = playing((cols, rows));
            let buf = painted(&mut s, (cols, rows));
            assert_eq!(buf.content.len(), cols * rows as usize);
            let flat = rgb_color_of(Look::from(club_lights_colors()).ground);
            let lit = buf.content.iter().filter(|c| c.bg != flat || c.fg != flat).count();
            assert!(cols < 8 || lit > 0, "{cols}x{rows}: nothing drawn");
        }
    }

    #[test]
    fn every_color_is_one_of_the_steps() {
        let (mut s, _tap) = playing((85, 12));
        let buf = painted(&mut s, (85, 12));
        for c in &buf.content {
            for col in [c.fg, c.bg] {
                for v in rgb_of(col) {
                    assert!(LEVELS.contains(&v), "{v} is not a level");
                }
            }
        }
    }

    #[test]
    fn the_picture_fades_to_the_flat_color_when_the_music_stops() {
        let (mut s, tap) = playing((85, 12));
        assert!(s.is_active() && s.presence() > 0.99);
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut s, &tap, (85, 12), 0.0, 4.0, |_| 0.0, &mut clock);
        assert!(s.presence() < 0.01, "presence {}", s.presence());
        let buf = painted(&mut s, (85, 12));
        let flat = rgb_color_of(Look::from(club_lights_colors()).ground);
        assert!(buf.content.iter().all(|c| c.bg == flat && c.symbol() == " "));
        assert!(!s.is_active());
    }

    #[test]
    fn a_redraw_that_comes_too_soon_changes_nothing() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        run(&mut s, &tap, (85, 12), 0.0, 1.0, music, &mut clock);
        let (before, turns) = (s.pulses(), s.swing().2);
        s.update(&tap, 85, 12, 7, clock + Duration::from_millis(5));
        assert_eq!(before, s.pulses());
        assert_eq!(turns, s.swing().2);
    }

    #[test]
    fn one_song_gives_one_picture_and_another_song_another() {
        let a = |seed: u64| {
            let tap = Tap::new();
            tap.set_rate(48_000);
            let mut s = ClubLights::new();
            let mut clock = Instant::now();
            for f in 0..45 {
                let mut w = tap.writer();
                for i in 0..1600 {
                    w.push(music((f * 1600 + i) as f32 / 48_000.0));
                }
                drop(w);
                clock += Duration::from_millis(33);
                s.update(&tap, 85, 12, seed, clock);
            }
            painted(&mut s, (85, 12))
        };
        assert_eq!(a(7).content, a(7).content);
        assert_ne!(a(7).content, a(8).content);
    }

    #[test]
    fn few_cells_change_from_one_frame_to_the_next() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = ClubLights::new();
        let mut clock = Instant::now();
        run(&mut s, &tap, (85, 12), 0.0, 2.0, music, &mut clock);
        let mut prev = painted(&mut s, (85, 12));
        let (mut sum, mut worst, mut n) = (0.0f32, 0.0f32, 0);
        for f in 0..150 {
            run(&mut s, &tap, (85, 12), 2.0 + f as f32 / 30.0, 1.0 / 30.0, music, &mut clock);
            let buf = painted(&mut s, (85, 12));
            let changed = buf.content.iter().zip(&prev.content).filter(|(a, b)| a != b).count() as f32 / buf.content.len() as f32;
            sum += changed;
            worst = worst.max(changed);
            n += 1;
            prev = buf;
        }
        let mean = sum / n as f32;
        println!("club lights: cells changed a frame: mean {:.1}%, worst {:.1}%", mean * 100.0, worst * 100.0);
        assert!(mean < 0.30, "mean {mean}");
    }

    #[test]
    #[ignore]
    fn dump_a_picture() {
        let Ok(path) = std::env::var("URCHIN_CLUB_DUMP") else { return };
        let size = std::env::var("URCHIN_CLUB_SIZE").unwrap_or_else(|_| "85x12".into());
        let (c, r) = size.split_once('x').expect("size like 85x12");
        let (cols, rows) = (c.parse::<usize>().unwrap(), r.parse::<u16>().unwrap());
        let (mut s, _tap) = playing((cols, rows));
        let buf = painted(&mut s, (cols, rows));
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            _ => "000000".into(),
        };
        let mut out = format!("{cols} {rows}\n");
        for y in 0..rows {
            for x in 0..cols as u16 {
                let cell = &buf[(x, y)];
                out += &format!("{} {} {}\n", symbol(&buf, x, y), hex(cell.fg), hex(cell.bg));
            }
        }
        std::fs::write(path, out).unwrap();
    }
}
