use std::f32::consts::TAU;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::bokeh::{fade_curve, hash01, sticky, Bands, AT_REST, CLASSES, FADE_IN, FADE_OUT, MIN_STEP, PULSE_ATTACK, PULSE_FALL, SILENCE_HOLD, SONG_SHARE, STALE};
use crate::spectrum::Tap;
use crate::theme::{rgb_color, rgb_mix, GalaxyColors, GALAXY_KINDS, GALAXY_STOPS};

const KINDS: usize = GALAXY_KINDS;
const LV: usize = 9;
const LEAD_STEPS: usize = 3;
const LEADERS: usize = 3;
const LEAD_MIN: f32 = 0.15;
const LEAD_RISE: f32 = 20.0;
const LEAD_FALL: f32 = 8.0;

const SIZE_BANDS: [(usize, usize); 6] = [(7, 9), (6, 8), (4, 6), (3, 5), (2, 3), (0, 2)];
const TWINKLE: (f32, f32) = (0.12, 1.07);

pub(crate) const MIN_GAP: f32 = 21.0;
const HIT_WAIT: f32 = 3.0;
const FIRST_AT: f32 = 6.0;
const MAX_LIVE: usize = 2;
const TREBLE_HIT: f32 = 0.72;
const METEOR_SPEED: (f32, f32) = (34.0, 52.0);
const METEOR_LIFE: (f32, f32) = (1.0, 1.5);
const TRAIL: f32 = 12.0;
const FLARE_FALL: f32 = 3.0;

const BREATHE_REST: f32 = 0.62;
const BREATHE_SWING: f32 = 0.40;
const BREATHE_LEAD: f32 = 0.20;
const PHOTO_GLOW: f32 = 0.5;
const BREATHE_UP: f32 = 0.05;
const BREATHE_DOWN: f32 = 0.26;

const CLOUDS: usize = 4;

const BRAILLE: [char; 8] = ['\u{2801}', '\u{2802}', '\u{2804}', '\u{2840}', '\u{2808}', '\u{2810}', '\u{2820}', '\u{2880}'];
const SIZED: [char; 4] = ['.', '·', '∙', '•'];
const GIANT: [char; 5] = ['✧', '☆', '✦', '★', '✴'];
pub(crate) const HEAD: char = '✦';
pub(crate) const TRAIL_GLYPHS: [(f32, char); 5] = [(0.42, '•'), (0.24, '∙'), (0.15, '·'), (0.09, '.'), (0.0, ' ')];

const SIZE_BELOW: [f32; 4] = [0.05, 0.22, 0.50, 0.992];
const BIG_CIRCLE_CHANCE: f32 = 0.30;
const GIANT_SHARE: f32 = 0.09;
const GIANT_MIN_MAG: f32 = 0.20;
const FLOOR_STEP: usize = 1;
const LIFT: f32 = 0.18;

#[derive(Clone, Copy, Debug)]
struct Star {
    x: u16,
    y: u16,
    mag: f32,
    size: u8,
    kind: u8,
    band: u8,
    rate: f32,
    p1: f32,
    p2: f32,
    ready: f32,
    dot: u8,
}

const PHOTO: &[u8] = include_bytes!("galaxy_bg.bin");
const PHOTO_W: usize = 168;
const PHOTO_H: usize = 112;

fn photo_at(cols: usize, rows: usize, x: usize, y: usize) -> [f32; 3] {
    let (win_w, win_h) = (cols as f32, 2.0 * rows as f32);
    let (img_w, img_h) = (PHOTO_W as f32, PHOTO_H as f32);
    let scale = (win_w / img_w).max(win_h / img_h);
    let (u, v) = ((x as f32 + 0.5) / scale + (img_w - win_w / scale) / 2.0, (2.0 * (y as f32 + 0.5)) / scale + (img_h - win_h / scale) / 2.0);
    let (u, v) = ((u - 0.5).clamp(0.0, img_w - 1.001), (v - 0.5).clamp(0.0, img_h - 1.001));
    let (iu, iv, fu, fv) = (u.floor() as usize, v.floor() as usize, u.fract(), v.fract());
    let px = |i: usize, j: usize, c: usize| PHOTO.get((j * PHOTO_W + i) * 3 + c).copied().unwrap_or(0) as f32;
    std::array::from_fn(|c| lerp(lerp(px(iu, iv, c), px(iu + 1, iv, c), fu), lerp(px(iu, iv + 1, c), px(iu + 1, iv + 1, c), fu), fv))
}

fn photo_clouds([r, g, b]: [f32; 3]) -> [f32; CLOUDS] {
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let d = hi - lo;
    let hue = if d <= 0.0 {
        230.0
    } else if hi == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if hi == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    const AT: [(f32, f32); CLOUDS] = [(10.0, 60.0), (295.0, 55.0), (232.0, 38.0), (188.0, 32.0)];
    let mut w = [0.0f32; CLOUDS];
    for (j, &(center, reach)) in AT.iter().enumerate() {
        let gap = (hue - center).abs();
        w[j] = (1.0 - gap.min(360.0 - gap) / reach).max(0.0);
    }
    let total: f32 = w.iter().sum();
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let strength = (lum / 95.0).clamp(0.0, 1.0).powf(0.6);
    if total <= 0.0 {
        return [0.0, 0.0, strength, 0.0];
    }
    w.map(|v| v / total * strength)
}

struct Field {
    cols: usize,
    rows: usize,
    seed: u64,
    stars: Vec<Star>,
    cloud: Vec<[f32; CLOUDS]>,
    photo: Vec<[f32; 3]>,
    ring: Vec<u8>,
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn noise(seed: u64, salt: u64, (cols, rows): (usize, usize), (x, y): (usize, usize)) -> f32 {
    const LATTICE: [usize; 5] = [3, 6, 12, 24, 48];
    let (mut sum, mut total) = (0.0, 0.0);
    for (k, &n) in LATTICE.iter().enumerate() {
        let gh = ((n as f32 * 2.0 * rows as f32 / cols as f32).round() as usize).max(2);
        let u = (x as f32 + 0.5) / cols as f32 * (n - 1) as f32;
        let v = (y as f32 + 0.5) / rows as f32 * (gh - 1) as f32;
        let (iu, iv) = (u.floor() as u64, v.floor() as u64);
        let (fu, fv) = (smooth(u.fract()), smooth(v.fract()));
        let g = |i: u64, j: u64| hash01(seed, salt * 16 + k as u64, (i << 20) | j);
        let top = lerp(g(iu, iv), g(iu + 1, iv), fu);
        let bottom = lerp(g(iu, iv + 1), g(iu + 1, iv + 1), fu);
        let w = 0.55f32.powi(k as i32);
        sum += w * lerp(top, bottom, fv);
        total += w;
    }
    sum / total
}

impl Field {
    fn new(cols: usize, rows: usize, seed: u64) -> Field {
        let mut field = Field { cols, rows, seed, stars: Vec::new(), cloud: Vec::with_capacity(cols * rows), photo: Vec::with_capacity(cols * rows), ring: Vec::with_capacity(cols * rows) };
        let aspect = 2.0 * rows as f32 / cols as f32;
        for y in 0..rows {
            for x in 0..cols {
                let idx = (y * cols + x) as u64;
                let cell = (cols, rows);
                let px = (x as f32 + 0.5) / cols as f32 - 0.5;
                let py = ((y as f32 + 0.5) / rows as f32 - 0.5) * aspect;
                let (cx, cy) = (0.03, -0.01);
                let dc = (((px - cx) / 0.15).powi(2) + ((py - cy) / 0.15f32.min(0.40 * aspect)).powi(2)).sqrt();
                let hollow = (-dc.powi(3)).exp();
                let rgb = photo_at(cols, rows, x, y);
                let cloud = photo_clouds(rgb);
                field.photo.push(rgb);
                let ring = ((((noise(0, 8, cell, (x, y)) - 0.32) / 0.36).clamp(0.0, 0.999)) * CLASSES as f32) as u8;
                field.cloud.push(cloud);
                field.ring.push(ring);

                let p = 0.15 + 0.16 * cloud.iter().sum::<f32>().min(1.0) + 0.14 * hollow;
                let (u1, u2, u3, u4) = (hash01(seed, idx, 1), hash01(seed, idx, 2), hash01(seed, idx, 3), hash01(seed, idx, 4));
                if u1 >= p {
                    continue;
                }
                let mag = (u2.powf(2.3) + 0.30 * hollow * u2).clamp(0.0, 1.0);
                let mut size = SIZE_BELOW.iter().position(|&b| mag < b).unwrap_or(4) as u8;
                if size == 4 && hash01(seed, idx, 18) > BIG_CIRCLE_CHANCE {
                    size = 3;
                }
                let mut dot = (hash01(seed, idx, 16) * 8.0) as u8 % 8;
                if u3 < GIANT_SHARE && mag > GIANT_MIN_MAG {
                    size = 5;
                    let big = (mag - GIANT_MIN_MAG) / (1.0 - GIANT_MIN_MAG);
                    dot = ((big * 3.0 + hash01(seed, idx, 17) * 2.0) as usize).min(GIANT.len() - 1) as u8;
                }
                let w = [
                    0.24 * (1.0 - hollow) + 0.04 * hollow,
                    0.07,
                    0.14 + 0.60 * hollow,
                    0.06 + 0.18 * hollow,
                    0.15,
                    0.07 * (1.0 - hollow) + 0.02 * hollow,
                    0.08 + 0.08 * hollow,
                    0.07 + 0.08 * hollow,
                    0.07 + 0.08 * hollow,
                ];
                let total: f32 = w.iter().sum();
                let mut acc = 0.0;
                let mut kind = 3;
                for (k, wk) in w.iter().enumerate() {
                    acc += wk / total;
                    if u4 <= acc {
                        kind = k as u8;
                        break;
                    }
                }
                let h = |salt: u64| hash01(seed, idx, 10 + salt);
                let (lo, hi) = SIZE_BANDS[size as usize];
                let band = (lo + (h(0) * (hi - lo + 1) as f32) as usize).min(hi) as u8;
                field.stars.push(Star {
                    x: x as u16,
                    y: y as u16,
                    mag,
                    size,
                    kind,
                    band,
                    rate: lerp(TWINKLE.0, TWINKLE.1, (band as f32 / (CLASSES - 1) as f32).powf(1.15)) * lerp(0.7, 1.3, h(1)),
                    p1: h(2) * TAU,
                    p2: h(3) * TAU,
                    ready: h(4),
                    dot,
                });
            }
        }
        field
    }
}

fn star_level(s: &Star, t: f32, pulse: &[f32; CLASSES], lead: &[f32; CLASSES], flare: f32) -> f32 {
    let b = s.band as usize;
    let tw = (0.5 + 0.5 * (TAU * s.rate * t + s.p1).sin() * (TAU * 0.41 * s.rate * t + s.p2).sin()) * 1.25;
    let tw = tw.clamp(0.0, 1.0);
    let amp = 0.20 + 0.45 * pulse[b];
    let hit = pulse[b] * (0.20 + 0.80 * s.ready);
    let flicker = (tw - 0.5) * 2.0 * amp * (0.30 + 0.70 * s.mag.sqrt());
    (s.mag * 0.62 + 0.55 * hit + flicker + 0.12 * lead[b] + 0.55 * flare).clamp(0.0, 1.0)
}

fn glyph_of(s: &Star) -> char {
    match s.size {
        0 => BRAILLE[s.dot as usize % 8],
        5 => GIANT[s.dot as usize % GIANT.len()],
        n => SIZED[n as usize - 1],
    }
}

#[derive(Clone, Copy, Debug)]
struct Meteor {
    born: f32,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    speed: f32,
    life: f32,
}

impl Meteor {
    fn head(&self, t: f32) -> (f32, f32) {
        let a = (t - self.born) * self.speed;
        (self.x + self.dx * a, self.y + self.dy * a)
    }
}

struct Look {
    ground: u32,
    ramp: [[u32; LV]; KINDS],
    trail: (u32, u32),
}

impl Look {
    fn new(c: GalaxyColors) -> Look {
        let mut ramp = [[0u32; LV]; KINDS];
        for (r, shades) in ramp.iter_mut().zip(c.stars.iter()) {
            for (s, v) in r.iter_mut().enumerate() {
                let pos = s as f32 / (LV - 1) as f32 * (GALAXY_STOPS - 1) as f32;
                let i = (pos.floor() as usize).min(GALAXY_STOPS - 2);
                *v = rgb_mix(shades[i], shades[i + 1], pos - i as f32);
            }
        }
        let hot = c.stars[3][GALAXY_STOPS - 1];
        Look {
            ground: c.ground,
            ramp,
            trail: (ramp[6][6], hot),
        }
    }
}

pub struct Galaxy {
    bands: Bands,
    columns: usize,
    rows: u16,
    pulse: [f32; CLASSES],
    lead: [f32; CLASSES],
    breathe: [[f32; CLOUDS]; CLASSES],
    field: Option<Field>,
    steps: Vec<u8>,
    flare: Vec<f32>,
    meteors: Vec<Meteor>,
    trails: Vec<(u16, u16, char, f32)>,
    spawned: u64,
    launched: u64,
    last_meteor: f32,
    treble_before: f32,
    time: f32,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
    look: Look,
}

impl Default for Galaxy {
    fn default() -> Self {
        Galaxy::new()
    }
}

impl Galaxy {
    pub fn new() -> Galaxy {
        Galaxy {
            bands: Bands::new(),
            columns: 0,
            rows: 0,
            pulse: [0.0; CLASSES],
            lead: [0.0; CLASSES],
            breathe: [[BREATHE_REST; CLOUDS]; CLASSES],
            field: None,
            steps: Vec::new(),
            flare: Vec::new(),
            meteors: Vec::new(),
            trails: Vec::new(),
            spawned: 0,
            launched: 0,
            last_meteor: FIRST_AT - MIN_GAP,
            treble_before: 0.0,
            time: 0.0,
            last: None,
            last_written: 0,
            quiet_since: None,
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
            look: Look::new(crate::theme::galaxy_colors()),
        }
    }

    pub fn is_active(&self) -> bool {
        let fading = if self.silent { self.presence > 0.0 } else { true };
        fading || !self.meteors.is_empty() || self.pulse.iter().any(|&p| p > AT_REST)
    }

    pub fn hold(&mut self, now: Instant) {
        self.last = Some(now);
        self.quiet_since = None;
    }

    pub fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        (self.columns, self.rows) = (columns.max(1), rows.max(1));
        let (cols, rows) = (self.columns, self.rows as usize);
        if self.field.as_ref().is_none_or(|f| (f.cols, f.rows, f.seed) != (cols, rows, seed)) {
            let field = Field::new(cols, rows, seed);
            self.steps = vec![0; field.stars.len()];
            self.flare = vec![0.0; field.stars.len()];
            self.meteors.clear();
            self.field = Some(field);
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

        let quiet = self.bands.update(tap, dt, stale);
        self.quiet_for = if quiet { (self.quiet_for + dt).min(10.0) } else { 0.0 };
        self.silent = self.quiet_for >= SILENCE_HOLD;
        if self.silent {
            self.presence = (self.presence - dt / FADE_OUT).max(0.0);
        } else {
            self.presence = (self.presence + dt / FADE_IN).min(1.0);
        }
        let up = 1.0 - (-dt / PULSE_ATTACK).exp();
        for c in 0..CLASSES {
            let want = ((1.0 - SONG_SHARE) * self.bands.target[c] + SONG_SHARE * self.bands.loudness).clamp(0.0, 1.0);
            let p = self.pulse[c];
            self.pulse[c] = if want > p { p + (want - p) * up } else { want.max(p - PULSE_FALL * dt) };
        }
        self.time += dt;

        let shows = fade_curve(self.presence);

        self.move_leaders(dt);
        self.move_clouds(dt);
        self.move_meteors(dt, seed);
        self.move_stars(dt, shows);
    }

    fn move_leaders(&mut self, dt: f32) {
        let mut order: Vec<usize> = (0..CLASSES).collect();
        order.sort_by(|&a, &b| self.pulse[b].total_cmp(&self.pulse[a]));
        let mut target = [0.0f32; CLASSES];
        for &c in order.iter().take(LEADERS) {
            if self.pulse[c] >= LEAD_MIN {
                target[c] = 1.0;
            }
        }
        for (lead, &want) in self.lead.iter_mut().zip(&target) {
            let rate = if want > *lead { LEAD_RISE } else { LEAD_FALL };
            *lead += (want - *lead) * (1.0 - (-dt * rate).exp());
        }
    }

    fn move_clouds(&mut self, dt: f32) {
        let top = |a: usize, b: usize| self.pulse[a..b].iter().copied().fold(0.0f32, f32::max);
        let group = [top(0, 4), top(4, 7), top(6, CLASSES), top(5, 9)];
        for b in 0..CLASSES {
            for (j, g) in group.iter().enumerate() {
                let want = BREATHE_REST + BREATHE_SWING * (0.5 * self.pulse[b] + 0.5 * g) + BREATHE_LEAD * self.lead[b];
                let have = self.breathe[b][j];
                let tau = if want > have { BREATHE_UP } else { BREATHE_DOWN };
                self.breathe[b][j] = have + (want - have) * (1.0 - (-dt / tau).exp());
            }
        }
    }

    fn move_meteors(&mut self, dt: f32, seed: u64) {
        let (cols, rows) = (self.columns as f32, self.rows as f32);
        let t = self.time;
        self.trails.clear();
        let treble = self.pulse[7..].iter().copied().fold(0.0f32, f32::max);
        let rising = treble >= TREBLE_HIT && self.treble_before < TREBLE_HIT;
        self.treble_before = treble;
        let ready = self.presence >= 0.99 && self.meteors.len() < MAX_LIVE && t - self.last_meteor >= MIN_GAP;
        if ready && (rising || t - self.last_meteor >= MIN_GAP + HIT_WAIT) {
            let n = self.spawned;
            self.spawned += 1;
            {
                let angle = hash01(seed, n, 101) * TAU;
                let (dx, dy) = (angle.cos(), angle.sin());
                let speed = lerp(METEOR_SPEED.0, METEOR_SPEED.1, hash01(seed, n, 102));
                let life = lerp(METEOR_LIFE.0, METEOR_LIFE.1, hash01(seed, n, 103));
                let span = speed * life;
                let x = lerp(0.15, 0.85, hash01(seed, n, 104)) * cols - dx * span * 0.5;
                let y = lerp(0.15, 0.85, hash01(seed, n, 105)) * 2.0 * rows - dy * span * 0.5;
                self.meteors.push(Meteor { born: t, x, y, dx, dy, speed, life });
                self.last_meteor = t;
                self.launched += 1;
            }
        }
        self.meteors.retain(|m| t - m.born <= m.life && self.presence >= 0.5);
        let Some(field) = &self.field else { return };
        for f in &mut self.flare {
            *f = (*f - FLARE_FALL * dt).max(0.0);
        }
        for m in &self.meteors {
            let age = (t - m.born) / m.life;
            let strength = (std::f32::consts::PI * age.min(1.0)).sin().powf(0.6);
            let (hx, hy) = m.head(t);
            let mut s = 0.0;
            while s < TRAIL {
                let (x, y) = (hx - m.dx * s, hy - m.dy * s);
                let (col, row) = (x.floor(), (y / 2.0).floor());
                s += 0.4;
                if col < 0.0 || row < 0.0 || col >= cols || row >= rows {
                    continue;
                }
                let v = strength * (-(s - 0.4) / 4.2).exp();
                if v < 0.09 {
                    continue;
                }
                let glyph = if s - 0.4 < 0.9 { HEAD } else { TRAIL_GLYPHS.iter().find(|(from, _)| v > *from).map_or(' ', |g| g.1) };
                let (c, r) = (col as u16, row as u16);
                if let Some(t) = self.trails.iter_mut().find(|t| (t.0, t.1) == (c, r)) {
                    if v > t.3 {
                        *t = (c, r, glyph, v);
                    }
                } else {
                    self.trails.push((c, r, glyph, v));
                }
            }
            for (i, st) in field.stars.iter().enumerate() {
                if (st.x as f32 + 0.5 - hx).abs() < 2.2 && ((st.y as f32 + 0.5) * 2.0 - hy).abs() < 3.2 {
                    self.flare[i] = 1.0;
                }
            }
        }
    }

    fn move_stars(&mut self, _dt: f32, shows: f32) {
        let Some(field) = &self.field else { return };
        for (i, s) in field.stars.iter().enumerate() {
            let level = star_level(s, self.time, &self.pulse, &self.lead, self.flare[i]);
            let mut want = ((LIFT + (1.0 - LIFT) * level) * shows * (LV - 1) as f32).round() as usize;
            if shows > 0.4 {
                want = want.max(FLOOR_STEP);
            }
            sticky(&mut self.steps[i], want.min(LV - 1), LV - 1);
        }
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        let (look, Some(field)) = (&self.look, &self.field) else { return };
        if area.is_empty() || area.width as usize != self.columns || area.height != self.rows {
            return;
        }
        let (cols, rows) = (self.columns, area.height as usize);
        if (field.cols, field.rows) != (cols, rows) {
            return;
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
        let ground = rgb_color(look.ground);
        if shows <= 0.0 {
            for y in 0..rows {
                for x in 0..cols {
                    put(area.x as usize + x, area.y as usize + y, ' ', ground, ground);
                }
            }
            return;
        }

        let (gr, gg, gb) = ((look.ground >> 16) as f32, ((look.ground >> 8) & 0xff) as f32, (look.ground & 0xff) as f32);
        let mut bgs = vec![look.ground; cols * rows];
        for (i, bg) in bgs.iter_mut().enumerate() {
            let ring = field.ring[i] as usize;
            let (mut r, mut g, mut b) = (gr, gg, gb);
            let total: f32 = field.cloud[i].iter().sum::<f32>().max(1e-4);
            let lift: f32 = (0..CLOUDS).map(|j| field.cloud[i][j] / total * (self.breathe[ring][j] / BREATHE_REST - 1.0).max(0.0)).sum();
            let f = ((1.0 + PHOTO_GLOW * lift) * 16.0).round() / 16.0;
            let [pr, pg, pb] = field.photo[i];
            r += (pr * f - gr) * shows;
            g += (pg * f - gg) * shows;
            b += (pb * f - gb) * shows;
            *bg = ((r.round().clamp(0.0, 255.0) as u32) << 16) | ((g.round().clamp(0.0, 255.0) as u32) << 8) | b.round().clamp(0.0, 255.0) as u32;
        }
        let mut fgs: Vec<Option<(char, u32)>> = vec![None; cols * rows];
        for (i, s) in field.stars.iter().enumerate() {
            let step = self.steps[i] as usize;
            if step == 0 {
                continue;
            }
            let glyph = glyph_of(s);
            let shown = if self.lead[s.band as usize] > 0.5 { (step + LEAD_STEPS).min(LV - 1) } else { step };
            fgs[s.y as usize * cols + s.x as usize] = Some((glyph, look.ramp[s.kind as usize][shown]));
        }
        for &(c, r, glyph, v) in &self.trails {
            if glyph == ' ' {
                continue;
            }
            let hot = rgb_mix(look.trail.0, look.trail.1, (v * 1.25).min(1.0));
            let i = r as usize * cols + c as usize;
            fgs[i] = Some((glyph, rgb_mix(bgs[i], hot, (0.25 + v * 1.1).min(1.0))));
        }
        for y in 0..rows {
            for x in 0..cols {
                let i = y * cols + x;
                let (glyph, fg) = fgs[i].unwrap_or((' ', bgs[i]));
                put(area.x as usize + x, area.y as usize + y, glyph, rgb_color(fg), rgb_color(bgs[i]));
            }
        }
    }

    #[cfg(test)]
    pub fn presence(&self) -> f32 {
        self.presence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Duration;

    fn rate_of(tap: &Tap) -> f32 {
        tap.rate() as f32
    }

    #[allow(clippy::too_many_arguments)]
    fn run_with(
        g: &mut Galaxy,
        tap: &Tap,
        (columns, rows): (usize, u16),
        seed: u64,
        start: f32,
        secs: f32,
        signal: impl Fn(f32) -> f32,
        clock: &mut Instant,
        mut each: impl FnMut(&Galaxy),
    ) {
        let rate = rate_of(tap);
        let per = (rate / 30.0) as usize;
        for f in 0..(secs * 30.0) as usize {
            let mut w = tap.writer();
            for i in 0..per {
                w.push(signal(start + (f * per + i) as f32 / rate));
            }
            drop(w);
            *clock += Duration::from_millis(33);
            g.update(tap, columns, rows, seed, *clock);
            each(g);
        }
    }

    #[allow(clippy::too_many_arguments, reason = "a test driver")]
    fn run(g: &mut Galaxy, tap: &Tap, size: (usize, u16), seed: u64, start: f32, secs: f32, signal: impl Fn(f32) -> f32, clock: &mut Instant) {
        run_with(g, tap, size, seed, start, secs, signal, clock, |_| {});
    }

    fn music(t: f32) -> f32 {
        let beat = t % 0.5;
        let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 3.9 * (1.0 - (-18.0 * beat).exp()))).sin();
        let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
        let chord: f32 = [220.0f32, 277.2, 329.6].iter().flat_map(|&f| (1..4).map(move |k| 0.05 / k as f32 * (TAU * f * k as f32 * t).sin())).sum();
        let lead = 0.06 * (TAU * 880.0 * t).sin() + 0.03 * (TAU * 3520.0 * t).sin() + 0.02 * (TAU * 9000.0 * t).sin();
        let hat = 0.18 * (-((t + 0.25) % 0.5) * 40.0).exp() * (TAU * 11000.0 * t).sin();
        (kick + bass + chord + lead + hat).clamp(-1.0, 1.0)
    }

    const SIZE: (usize, u16) = (96, 20);

    fn painted(g: &mut Galaxy, (columns, rows): (usize, u16)) -> Buffer {
        let area = Rect::new(0, 0, columns as u16, rows);
        let mut buf = Buffer::empty(area);
        g.paint(&mut buf, area);
        buf
    }

    fn playing(size: (usize, u16), seed: u64, secs: f32) -> (Galaxy, Tap, Instant) {
        let (tap, mut g) = (Tap::new(), Galaxy::new());
        let mut clock = Instant::now();
        run(&mut g, &tap, size, seed, 0.0, secs, music, &mut clock);
        (g, tap, clock)
    }

    fn symbol(buf: &Buffer, x: u16, y: u16) -> String {
        let s = buf[(x, y)].symbol();
        if s.is_empty() { " ".into() } else { s.into() }
    }

    fn rgb_of(c: Color) -> (f32, f32, f32) {
        let Color::Rgb(r, g, b) = c else { panic!("not a plain color") };
        (r as f32, g as f32, b as f32)
    }

    #[test]
    fn only_the_listed_characters_are_drawn() {
        let allowed: HashSet<char> = BRAILLE.iter().copied().chain(['.', '·', '∙', '•', '✧', '☆', '✦', '★', '✴', ' ']).collect();
        let (mut g, tap, mut clock) = playing(SIZE, 7, 2.0);
        for _ in 0..40 {
            run(&mut g, &tap, SIZE, 7, 2.0, 0.5, music, &mut clock);
            let buf = painted(&mut g, SIZE);
            for y in 0..SIZE.1 {
                for x in 0..SIZE.0 as u16 {
                    let s = symbol(&buf, x, y);
                    assert!(s.chars().count() == 1 && allowed.contains(&s.chars().next().unwrap()), "unexpected {s:?} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn stars_never_move() {
        let (mut g, tap, mut clock) = playing(SIZE, 11, 1.5);
        let place: HashSet<(u16, u16)> = g.field.as_ref().unwrap().stars.iter().map(|s| (s.x, s.y)).collect();
        assert!(place.len() > 200, "only {} stars", place.len());
        for _ in 0..60 {
            run(&mut g, &tap, SIZE, 11, 1.5, 0.5, music, &mut clock);
            let trails: HashSet<(u16, u16)> = g.trails.iter().map(|t| (t.0, t.1)).collect();
            let buf = painted(&mut g, SIZE);
            for y in 0..SIZE.1 {
                for x in 0..SIZE.0 as u16 {
                    if symbol(&buf, x, y) != " " {
                        assert!(place.contains(&(x, y)) || trails.contains(&(x, y)), "a character at {x},{y} where there is no star");
                    }
                }
            }
        }
    }

    #[test]
    fn every_star_listens_to_the_range_of_its_size() {
        let f = Field::new(96, 20, 5);
        for s in &f.stars {
            let (lo, hi) = SIZE_BANDS[s.size as usize];
            assert!((lo..=hi).contains(&(s.band as usize)), "a star of size {} listens to range {}", s.size, s.band);
        }
        let mean = |size: u8| {
            let b: Vec<f32> = f.stars.iter().filter(|s| s.size == size).map(|s| s.band as f32).collect();
            b.iter().sum::<f32>() / b.len().max(1) as f32
        };
        assert!(mean(5) < mean(3) && mean(3) < mean(1) && mean(1) < mean(0), "bigger stars should take lower sound");
        assert!(f.stars.iter().filter(|s| s.size == 3).map(|s| s.band).collect::<HashSet<_>>().len() >= 2);
        let none = [0.0f32; CLASSES];
        let giant = f.stars.iter().find(|s| s.size == 5 && s.mag > 0.4).expect("a giant");
        let (mut bass, mut treble) = (none, none);
        bass[giant.band as usize] = 1.0;
        treble[9] = 1.0;
        assert!(star_level(giant, 1.0, &bass, &none, 0.0) > star_level(giant, 1.0, &treble, &none, 0.0) + 0.1);
        let moved = f.stars.iter().filter(|s| s.mag > 0.3).filter(|s| {
            let mut p = none;
            p[s.band as usize] = 1.0;
            star_level(s, 1.0, &none, &none, 0.0) + 0.05 < star_level(s, 1.0, &p, &none, 0.0)
        });
        assert!(moved.count() > 20, "the music hardly lifts the stars");
    }

    #[test]
    fn nothing_flickers_fast() {
        let f = Field::new(96, 20, 3);
        let loud = [0.3f32; CLASSES];
        let none = [0.0f32; CLASSES];
        let mut worst = 0.0f32;
        for s in &f.stars {
            let mut t = 0.0;
            while t < 40.0 {
                worst = worst.max((star_level(s, t + 0.1, &loud, &none, 0.0) - star_level(s, t, &loud, &none, 0.0)).abs());
                t += 0.1;
            }
        }
        assert!(f.stars.iter().all(|s| s.rate <= TWINKLE.1 * 1.3 + 0.001), "a star twinkles faster than allowed");
        assert!(worst < 0.30, "a star twinkled {worst:.3} in 0.1 s");
    }

    #[test]
    fn big_circles_are_rare() {
        let (mut g, tap, mut clock) = playing(SIZE, 7, 1.5);
        let (mut black, mut stars) = (0usize, 0usize);
        for _ in 0..80 {
            run(&mut g, &tap, SIZE, 7, 1.5, 0.5, music, &mut clock);
            let buf = painted(&mut g, SIZE);
            for y in 0..SIZE.1 {
                for x in 0..SIZE.0 as u16 {
                    match symbol(&buf, x, y).as_str() {
                        " " => {}
                        "•" | "●" => { black += 1; stars += 1 }
                        _ => stars += 1,
                    }
                }
            }
        }
        let share = black as f32 / stars as f32;
        assert!(share < 0.012, "{:.1}% of the characters are big circles", share * 100.0);
    }

    fn glow_of(buf: &Buffer) -> f32 {
        let mut sum = 0.0;
        for c in &buf.content {
            let (r, g, b) = rgb_of(c.bg);
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
        }
        sum / buf.content.len() as f32
    }

    #[test]
    fn the_clouds_follow_the_music_but_only_a_little() {
        let (tap, mut g) = (Tap::new(), Galaxy::new());
        let mut clock = Instant::now();
        let mut glows = Vec::new();
        run_with(&mut g, &tap, SIZE, 7, 0.0, 8.0, music, &mut clock, |g| {
            let mut copy_look = 0;
            let _ = &mut copy_look;
            glows.push(g.breathe.iter().flatten().sum::<f32>());
        });
        let (lo, hi) = glows[60..].iter().fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi > lo * 1.05, "the clouds do not move with the music ({lo:.1} to {hi:.1})");
        assert!(hi < lo * 2.0, "the clouds swing too far ({lo:.1} to {hi:.1})");
        let mut seen = Vec::new();
        for _ in 0..40 {
            run(&mut g, &tap, SIZE, 7, 8.0, 0.1, music, &mut clock);
            seen.push(glow_of(&painted(&mut g, SIZE)));
        }
        let (lo, hi) = seen.iter().fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi > lo * 1.005, "the background does not change with the music ({lo:.2} to {hi:.2})");
        assert!(hi < lo * 1.30, "the background swings too far ({lo:.2} to {hi:.2})");
    }

    #[test]
    fn shooting_stars_are_rare_and_go_every_way() {
        let mut angles = Vec::new();
        for seed in [3u64, 7, 21, 55, 90, 123] {
            let (tap, mut g) = (Tap::new(), Galaxy::new());
            let mut clock = Instant::now();
            let mut births: Vec<f32> = Vec::new();
            let mut most = 0;
            run_with(&mut g, &tap, SIZE, seed, 0.0, 70.0, music, &mut clock, |g| {
                most = most.max(g.meteors.len());
                for m in &g.meteors {
                    if !births.contains(&m.born) {
                        births.push(m.born);
                        angles.push(m.dy.atan2(m.dx).to_degrees());
                    }
                }
            });
            assert!(most <= MAX_LIVE, "seed {seed}: {most} at once");
            assert!((3..=4).contains(&g.launched), "seed {seed}: {} shooting stars in 70 s (one every 21 s)", g.launched);
            assert!(births.windows(2).all(|w| w[1] - w[0] >= MIN_GAP - 0.1), "seed {seed}: two too close: {births:?}");
        }
        assert!(angles.len() >= 3, "hardly any shooting stars: {angles:?}");
        let (min, max) = angles.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(max - min > 90.0, "they all go the same way: {angles:?}");
    }

    #[test]
    fn a_shooting_star_crosses_the_sky_with_a_trail() {
        let (tap, mut g) = (Tap::new(), Galaxy::new());
        let mut clock = Instant::now();
        let mut best = 0;
        let mut head = false;
        let mut inside = true;
        run_with(&mut g, &tap, SIZE, 7, 0.0, 40.0, music, &mut clock, |g| {
            best = best.max(g.trails.len());
            head |= g.trails.iter().any(|t| t.2 == HEAD);
            inside &= g.trails.iter().all(|t| (t.0 as usize) < SIZE.0 && (t.1 as usize) < SIZE.1 as usize);
        });
        assert!(g.launched >= 1, "no shooting star in 40 s");
        assert!(best >= 5, "the trail has only {best} cells");
        assert!(head && inside);
    }

    #[test]
    fn silence_fades_to_the_flat_color_and_back() {
        let (mut g, tap, mut clock) = playing(SIZE, 7, 3.0);
        let own = crate::theme::galaxy_colors().ground;
        let lit = painted(&mut g, SIZE);
        assert!(lit.content.iter().any(|c| c.symbol() != " "));
        run(&mut g, &tap, SIZE, 7, 3.0, 4.5, |_| 0.0, &mut clock);
        assert!(g.presence() <= 0.0, "still {:.2} showing", g.presence());
        assert!(!g.is_active());
        let flat = painted(&mut g, SIZE);
        let want = rgb_color(own);
        assert!(flat.content.iter().all(|c| c.bg == want && (c.symbol() == " " || c.symbol().is_empty())), "not one flat color");
        run(&mut g, &tap, SIZE, 7, 8.0, 1.0, music, &mut clock);
        assert!(g.presence() > 0.9 && g.is_active());
    }

    #[test]
    fn one_song_is_one_sky_and_another_song_another() {
        let a = Field::new(96, 20, 42);
        let b = Field::new(96, 20, 42);
        let c = Field::new(96, 20, 43);
        let places = |f: &Field| f.stars.iter().map(|s| (s.x, s.y, s.size, s.kind)).collect::<Vec<_>>();
        assert_eq!(places(&a), places(&b));
        assert_ne!(places(&a), places(&c));
        let (mut g1, _, _) = playing(SIZE, 42, 4.0);
        let (mut g2, _, _) = playing(SIZE, 42, 4.0);
        let (p1, p2) = (painted(&mut g1, SIZE), painted(&mut g2, SIZE));
        let stars = |b: &Buffer| b.content.iter().filter(|c| c.symbol() != " ").count();
        assert!(stars(&p1).abs_diff(stars(&p2)) < 12);
    }

    #[test]
    fn the_background_is_the_photograph_and_the_same_for_every_song() {
        for size in [(96usize, 20usize), (60, 12), (200, 50)] {
            let (a, b) = (Field::new(size.0, size.1, 3), Field::new(size.0, size.1, 99));
            assert_eq!(a.photo, b.photo, "{size:?}: the background changed with the song");
            assert_eq!(a.cloud, b.cloud);
            assert_eq!(a.ring, b.ring);
            let n = a.photo.len() as f32;
            let lum = |p: &[f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let mean = a.photo.iter().map(lum).sum::<f32>() / n;
            assert!((15.0..70.0).contains(&mean), "{size:?}: mean brightness {mean:.0}");
            let share = |j: usize| a.cloud.iter().filter(|c| c[j] > 0.15).count() as f32 / n;
            for (j, name) in ["warm", "purple", "blue", "cyan"].iter().enumerate() {
                assert!(share(j) > if j == 0 { 0.003 } else { 0.02 }, "{size:?}: hardly any {name} in the background ({:.1}%)", share(j) * 100.0);
            }
        }
        let f = Field::new(96, 20, 1);
        let at = |fx: f32, fy: f32| lum_of(f.photo[((fy * 20.0) as usize).min(19) * 96 + ((fx * 96.0) as usize).min(95)]);
        fn lum_of(p: [f32; 3]) -> f32 {
            0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
        }
        assert!(at(0.62, 0.5) < at(0.45, 0.45) * 0.6, "the dark heart is not where it should be");
    }

    #[test]
    fn the_sky_is_made_like_the_photograph() {
        let f = Field::new(96, 20, 7);
        let n = f.stars.len() as f32;
        let cells = (96 * 20) as f32;
        assert!((0.15..0.32).contains(&(n / cells)), "{:.0}% of the cells are stars", 100.0 * n / cells);
        let count = |p: &dyn Fn(&Star) -> bool| f.stars.iter().filter(|s| p(s)).count() as f32 / n;
        assert!(count(&|s| s.size == 4) < 0.12, "too many of the biggest stars");
        assert!((0.05..0.13).contains(&count(&|s| s.size == 5)), "giants should be a good few");
        assert!(count(&|s| s.size == 0) > 0.15, "too few faint dots");
        let hollow = |s: &Star| (s.x as f32 - 96.0 * 0.53).abs() < 12.0;
        let (blue_in, blue_out) = (f.stars.iter().filter(|s| hollow(s) && s.kind == 2).count() as f32 / f.stars.iter().filter(|s| hollow(s)).count().max(1) as f32,
            f.stars.iter().filter(|s| !hollow(s) && s.kind == 2).count() as f32 / f.stars.iter().filter(|s| !hollow(s)).count().max(1) as f32);
        assert!(blue_in > blue_out * 1.5, "the hollow should be the blue part ({blue_in:.2} against {blue_out:.2})");
        let kinds = |k: &[u8]| f.stars.iter().filter(|s| k.contains(&s.kind)).count() as f32 / n;
        assert!(kinds(&[3]) < 0.12, "too many white stars");
        assert!(kinds(&[0, 4, 2]) > 0.50, "burgundy, indigo and navy should be most of them");
        assert!(kinds(&[5, 6, 7, 8]) > 0.18, "some forest green, cyan, aquamarine and teal");
        assert!((0..KINDS as u8).all(|k| kinds(&[k]) > 0.0), "a color is missing");
        assert!(f.ring.iter().copied().max().unwrap() >= 8 && f.ring.iter().copied().min().unwrap() <= 1);
    }

    #[test]
    fn any_window_size_is_painted_edge_to_edge() {
        for size in [(1usize, 1u16), (2, 1), (3, 2), (40, 1), (7, 30), (96, 20), (200, 60)] {
            let (mut g, _, _) = playing(size, 5, 1.0);
            let buf = painted(&mut g, size);
            assert_eq!(buf.content.len(), size.0 * size.1 as usize);
            let ground = rgb_color(crate::theme::galaxy_colors().ground);
            assert!(buf.content.iter().any(|c| c.bg != ground || c.symbol() != " ") || size.0 * size.1 as usize <= 3, "{size:?}: nothing drawn");
        }
    }

    #[test]
    fn few_cells_change_from_frame_to_frame() {
        for size in [(85usize, 12u16), (96, 20), (160, 40)] {
            let (tap, mut g) = (Tap::new(), Galaxy::new());
            let mut clock = Instant::now();
            run(&mut g, &tap, size, 7, 0.0, 2.0, music, &mut clock);
            let mut last = painted(&mut g, size);
            let (mut changed, mut frames) = (0.0f32, 0);
            for k in 0..240 {
                run(&mut g, &tap, size, 7, 2.0 + k as f32 / 30.0, 1.0 / 30.0, music, &mut clock);
                let now = painted(&mut g, size);
                let n = now.content.iter().zip(&last.content).filter(|(a, b)| a != b).count();
                changed += n as f32 / now.content.len() as f32;
                frames += 1;
                last = now;
            }
            let mean = changed / frames as f32;
            assert!(mean < 0.20, "{size:?}: {:.1}% of the cells change a frame", mean * 100.0);
        }
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn galaxy_cells_dump() {
    use std::time::Duration;
    let out = std::env::var("GALAXY_OUT").unwrap_or_else(|_| "galaxy_cells".into());
    std::fs::create_dir_all(&out).unwrap();
    let cols: usize = std::env::var("GALAXY_COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(96);
    let rows: u16 = std::env::var("GALAXY_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let seed: u64 = std::env::var("GALAXY_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let at: Vec<f32> = std::env::var("GALAXY_AT").unwrap_or_else(|_| "3.2,6.6,10.5".into()).split(',').filter_map(|v| v.parse().ok()).collect();
    {
        let (tap, mut g) = (Tap::new(), Galaxy::new());
        let mut clock = Instant::now();
        let rate = tap.rate() as f32;
        let per = (rate / 30.0) as usize;
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            _ => "000000".into(),
        };
        let mut frame = 0usize;
        for (k, &t) in at.iter().enumerate() {
            while (frame as f32) / 30.0 < t {
                let mut w = tap.writer();
                for i in 0..per {
                    let tt = (frame * per + i) as f32 / rate;
                    let beat = tt % 0.5;
                    let drums = !(5.5..8.0).contains(&tt);
                    let kick = if drums { 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 3.9 * (1.0 - (-18.0 * beat).exp()))).sin() } else { 0.0 };
                    let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * tt).sin()).sum();
                    let chord: f32 = [220.0f32, 277.2, 329.6].iter().flat_map(|&f| (1..4).map(move |k| 0.05 / k as f32 * (TAU * f * k as f32 * tt).sin())).sum();
                    let lead = 0.06 * (TAU * 880.0 * tt).sin() + 0.03 * (TAU * 3520.0 * tt).sin() + 0.02 * (TAU * 9000.0 * tt).sin();
                    let hat = if drums { 0.18 * (-((tt + 0.25) % 0.5) * 40.0).exp() * (TAU * 11000.0 * tt).sin() } else { 0.0 };
                    w.push((kick + bass + chord + lead + hat).clamp(-1.0, 1.0));
                }
                drop(w);
                clock += Duration::from_millis(33);
                g.update(&tap, cols, rows, seed, clock);
                frame += 1;
            }
            let area = Rect::new(0, 0, cols as u16, rows);
            let mut buf = Buffer::empty(area);
            g.paint(&mut buf, area);
            let mut text = format!("{cols} {rows}\n");
            for y in 0..rows {
                for x in 0..cols as u16 {
                    let c = &buf[(x, y)];
                    let sym = if c.symbol().is_empty() { " " } else { c.symbol() };
                    text.push_str(&format!("{sym}{}{}", hex(c.fg), hex(c.bg)));
                }
                text.push('\n');
            }
            let file = format!("{out}/galaxy_{k}_{t:.1}s.cells");
            std::fs::write(file, text).unwrap();
        }
    }
}
