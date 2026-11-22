use crate::bokeh::hash01;
use crate::galaxy::{HEAD, TRAIL_GLYPHS};

const FIRST_AT: f32 = 2.0;
pub const EVERY: f32 = 5.0;
const DENSITY: f32 = 1.0 / 12.0;
const CIRCLE_SHARE: f32 = 0.40;
const CIRCLES: [char; 3] = ['·', '∙', '•'];
const FILLED: [char; 6] = ['✦', '✦', '✦', '✦', '★', '✴'];
const HOLLOW: [char; 3] = ['✧', '✧', '☆'];
pub const SHADES: usize = 4;

const SPEED: (f32, f32) = (5.5, 7.0);
const SPREAD: (usize, usize) = (2, 6);
const TRAIL: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Clear,
    Moon,
    Mountain,
}

pub struct Sky<'a> {
    pub cols: usize,
    pub rows: usize,
    pub open: &'a [bool],
    pub cover: &'a [Cover],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub x: usize,
    pub y: usize,
    pub glyph: char,
    pub shade: u8,
    pub level: f32,
}

#[derive(Clone, Copy, Debug)]
struct Star {
    x: usize,
    y: usize,
    glyph: char,
    shade: u8,
}

#[derive(Clone, Debug)]
struct Meteor {
    path: Vec<Option<(usize, usize)>>,
    speed: f32,
    s: f32,
}

pub struct NightSky {
    seed: Option<u64>,
    layout: (usize, usize),
    stars: Vec<Star>,
    meteors: Vec<Meteor>,
    played: f32,
    next_at: f32,
    count: u64,
}

impl Default for NightSky {
    fn default() -> Self {
        NightSky::new()
    }
}

impl NightSky {
    pub fn new() -> NightSky {
        NightSky { seed: None, layout: (0, 0), stars: Vec::new(), meteors: Vec::new(), played: 0.0, next_at: FIRST_AT, count: 0 }
    }

    pub fn update(&mut self, sky: &Sky, seed: u64, dt: f32, sounding: bool) {
        if self.seed != Some(seed) {
            *self = NightSky { seed: Some(seed), ..NightSky::new() };
        }
        if self.layout != (sky.cols, sky.rows) {
            self.layout = (sky.cols, sky.rows);
            self.stars = place_stars(sky, seed);
            self.meteors.clear();
        }
        for m in &mut self.meteors {
            m.s += m.speed * dt;
        }
        self.meteors.retain(|m| (m.s as usize) < m.path.len() + TRAIL);
        if sounding {
            self.played += dt;
            if self.played >= self.next_at {
                self.next_at += EVERY;
                let n = self.count;
                self.count += 1;
                if let Some(m) = self.plan(sky, seed, n) {
                    self.meteors.clear();
                    self.meteors.push(m);
                }
            }
        }
    }

    pub fn calm(&mut self) {
        self.meteors.clear();
    }

    fn plan(&self, sky: &Sky, seed: u64, n: u64) -> Option<Meteor> {
        if sky.cols < 2 || sky.rows < 2 {
            return None;
        }
        let at = |x: usize, y: usize| sky.cover[y * sky.cols + x];
        let across = SPREAD.0 + (hash01(seed, n, 5) * (SPREAD.1 - SPREAD.0 + 1) as f32) as usize % (SPREAD.1 - SPREAD.0 + 1);
        let fall = |x0: usize, y0: usize, right: bool| {
            let mut path = Vec::new();
            for (i, y) in (y0..sky.rows).enumerate() {
                let Some(x) = (if right { Some(x0 + i * across) } else { x0.checked_sub(i * across) }).filter(|&x| x < sky.cols) else { break };
                path.push((x, y));
                if at(x, y) != Cover::Clear {
                    break;
                }
            }
            path
        };
        let right = (n + (seed & 1)) % 2 == 1;
        let (mut into_moon, mut lands) = (Vec::new(), Vec::new());
        for x0 in (0..sky.cols).filter(|&x| at(x, 0) == Cover::Clear && !self.stars.iter().any(|s| s.y == 0 && s.x.abs_diff(x) <= 1)) {
            let path = fall(x0, 0, right);
            match path.last() {
                _ if path.len() < 3 => {}
                Some(&(x, y)) if at(x, y) == Cover::Moon => into_moon.push(path),
                _ => lands.push(path),
            }
        }
        let pick = |list: &[Vec<(usize, usize)>], k: u64| list.get((hash01(seed, n, k) * list.len() as f32) as usize % list.len().max(1)).cloned();
        let into = if hash01(seed, n, 1) < 0.5 { pick(&into_moon, 2) } else { None };
        let path = into.or_else(|| pick(&lands, 2)).or_else(|| pick(&into_moon, 2))?.into_iter().map(Some).collect();
        Some(Meteor { path, speed: SPEED.0 + (SPEED.1 - SPEED.0) * hash01(seed, n, 3), s: 0.0 })
    }

    pub fn marks(&self, out: &mut Vec<Mark>) {
        out.clear();
        for star in &self.stars {
            out.push(Mark { x: star.x, y: star.y, glyph: star.glyph, shade: star.shade, level: 1.0 });
        }
        for m in &self.meteors {
            let step = m.s as usize;
            let Some(seen) = (0..=step.min(m.path.len() - 1)).rev().find(|&i| m.path[i].is_some()) else { continue };
            let gone_in = step - seen;
            if gone_in == 0 {
                let (x, y) = m.path[seen].unwrap_or_default();
                out.push(Mark { x, y, glyph: HEAD, shade: 0, level: 1.0 });
            }
            for back in 1..=TRAIL {
                let Some(o) = back.checked_sub(gone_in).filter(|&o| o > 0) else { continue };
                let Some(&Some((x, y))) = seen.checked_sub(o).and_then(|i| m.path.get(i)) else { continue };
                let glyph = TRAIL_GLYPHS[back - 1].1;
                out.push(Mark { x, y, glyph, shade: back.min(SHADES - 1) as u8, level: if back == TRAIL { 0.6 } else { 1.0 } });
            }
        }
    }
}

fn place_stars(sky: &Sky, seed: u64) -> Vec<Star> {
    let mut stars: Vec<Star> = Vec::new();
    for y in 0..sky.rows {
        for x in 0..sky.cols {
            if !sky.open[y * sky.cols + x] || hash01(seed, x as u64, y as u64) >= DENSITY {
                continue;
            }
            if stars.iter().any(|s| s.x.abs_diff(x) <= 1 && s.y.abs_diff(y) <= 1) {
                continue;
            }
            stars.push(Star { x, y, glyph: ' ', shade: 0 });
        }
    }
    let spots: Vec<(u64, u64)> = stars.iter().map(|s| (s.x as u64, s.y as u64)).collect();
    let key = |i: &usize, k: u64| hash01(seed ^ k, spots[*i].0, spots[*i].1);
    let mut order: Vec<usize> = (0..stars.len()).collect();
    order.sort_by(|a, b| key(a, 0x5eed).total_cmp(&key(b, 0x5eed)));
    for (n, &i) in order.iter().enumerate() {
        stars[i].shade = if n.is_multiple_of(2) { 0 } else { (1 + (n / 2) % (SHADES - 1)) as u8 };
    }
    order.sort_by(|a, b| key(a, 0x5aa9).total_cmp(&key(b, 0x5aa9)));
    let circles = (stars.len() as f32 * CIRCLE_SHARE).round() as usize;
    for (n, &i) in order.iter().enumerate() {
        let pick = |set: &[char]| set[(hash01(seed, i as u64, 7) * set.len() as f32) as usize % set.len()];
        let star = n.saturating_sub(circles);
        stars[i].glyph = if n < circles {
            pick(&CIRCLES)
        } else if star.is_multiple_of(2) {
            FILLED[(star / 2) % FILLED.len()]
        } else {
            HOLLOW[(star / 2) % HOLLOW.len()]
        };
    }
    stars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky_cells(cols: usize, rows: usize) -> (Vec<bool>, Vec<Cover>) {
        let cover: Vec<Cover> = (0..cols * rows)
            .map(|i| {
                let (x, y) = (i % cols, i / cols);
                if (x as f32 - cols as f32 / 2.0).abs() < 12.0 && y + 3 >= rows {
                    Cover::Moon
                } else if (x < 10 || x + 10 >= cols) && y + 2 >= rows {
                    Cover::Mountain
                } else {
                    Cover::Clear
                }
            })
            .collect();
        (cover.iter().map(|c| *c == Cover::Clear).collect(), cover)
    }

    fn play(sky: &mut NightSky, s: &Sky, seed: u64, secs: f32) -> Vec<Vec<Mark>> {
        let mut frames = Vec::new();
        for _ in 0..(secs * 30.0).round() as usize {
            sky.update(s, seed, 1.0 / 30.0, true);
            let mut marks = Vec::new();
            sky.marks(&mut marks);
            frames.push(marks);
        }
        frames
    }

    #[test]
    fn half_the_stars_are_white_and_as_many_are_filled_as_hollow() {
        for seed in 1..20 {
            let (open, cover) = sky_cells(90, 6);
            let sky = Sky { cols: 90, rows: 6, open: &open, cover: &cover };
            let stars = place_stars(&sky, seed);
            assert!(stars.len() >= 8, "seed {seed}: only {} stars", stars.len());
            let white = stars.iter().filter(|s| s.shade == 0).count();
            assert!(white.abs_diff(stars.len() - white) <= 1, "seed {seed}: {white} white of {}", stars.len());
            let greys: Vec<usize> = (1..SHADES as u8).map(|g| stars.iter().filter(|s| s.shade == g).count()).collect();
            assert!(greys.iter().max().unwrap() - greys.iter().min().unwrap() <= 1, "seed {seed}: greys {greys:?}");
            let filled = stars.iter().filter(|s| FILLED.contains(&s.glyph)).count();
            let hollow = stars.iter().filter(|s| HOLLOW.contains(&s.glyph)).count();
            assert!(filled.abs_diff(hollow) <= 1 && filled > 0, "seed {seed}: {filled} filled, {hollow} hollow");
            assert!(stars.iter().all(|s| open[s.y * 90 + s.x]), "a star off the open sky");
            let four = stars.iter().filter(|s| matches!(s.glyph, '✦' | '✧')).count();
            let other = stars.iter().filter(|s| matches!(s.glyph, '★' | '✴' | '☆')).count();
            let circles = stars.iter().filter(|s| CIRCLES.contains(&s.glyph)).count();
            assert!(four > other && four >= circles, "seed {seed}: {four} four-pointed, {other} other stars, {circles} circles");
            for a in &stars {
                assert!(stars.iter().filter(|b| b.x.abs_diff(a.x) <= 1 && b.y.abs_diff(a.y) <= 1).count() == 1, "stars touch");
            }
        }
    }

    #[test]
    fn the_first_shooting_star_comes_at_2_seconds_then_every_5() {
        let (open, cover) = sky_cells(90, 6);
        let sky = Sky { cols: 90, rows: 6, open: &open, cover: &cover };
        let mut night = NightSky::new();
        let (mut frames, mut starts) = (Vec::new(), Vec::new());
        for i in 0..50 * 30 {
            let had = night.count;
            frames.extend(play(&mut night, &sky, 9, 1.0 / 30.0));
            if night.count != had {
                starts.push(i);
            }
        }
        let secs: Vec<f32> = starts.iter().map(|&i| (i + 1) as f32 / 30.0).collect();
        assert_eq!(secs.len(), 10, "{secs:?}");
        for (i, s) in secs.iter().enumerate() {
            assert!((s - (2.0 + 5.0 * i as f32)).abs() < 0.1, "shooting star at {s}");
        }
        for f in &frames {
            let shooting = &f[night.stars.len()..];
            assert!(shooting.len() <= TRAIL + 1);
            assert!(shooting.iter().filter(|m| m.glyph == HEAD).count() <= 1, "two shooting stars at once");
            for w in shooting.windows(2) {
                assert_eq!(w[0].y, w[1].y + 1, "two on one row, or a row skipped: {shooting:?}");
                let across = w[0].x.abs_diff(w[1].x);
                assert!((SPREAD.0..=SPREAD.1).contains(&across), "{across} across: {shooting:?}");
                assert_eq!(across, shooting[0].x.abs_diff(shooting[1].x), "uneven: {shooting:?}");
            }
            if shooting.len() > 2 {
                assert_eq!(shooting[0].x < shooting[1].x, shooting[1].x < shooting[2].x, "bends: {shooting:?}");
            }
        }
    }

    #[test]
    fn a_pause_freezes_a_shooting_star_and_a_new_song_gets_a_new_sky() {
        let (open, cover) = sky_cells(90, 6);
        let sky = Sky { cols: 90, rows: 6, open: &open, cover: &cover };
        let mut night = NightSky::new();
        play(&mut night, &sky, 2, 2.3);
        assert_eq!(night.meteors.len(), 1);
        let mut before = Vec::new();
        night.marks(&mut before);
        for _ in 0..30 {
            night.update(&sky, 2, 0.0, true);
        }
        let mut after = Vec::new();
        night.marks(&mut after);
        assert_eq!(before, after);
        let old: Vec<_> = night.stars.iter().map(|s| (s.x, s.y, s.glyph)).collect();
        night.update(&sky, 3, 0.0, true);
        let new: Vec<_> = night.stars.iter().map(|s| (s.x, s.y, s.glyph)).collect();
        assert_ne!(old, new);
        assert!(night.meteors.is_empty() && night.played == 0.0);
    }

    #[test]
    fn no_shooting_star_while_silent() {
        let (open, cover) = sky_cells(90, 6);
        let sky = Sky { cols: 90, rows: 6, open: &open, cover: &cover };
        let mut night = NightSky::new();
        for _ in 0..(60 * 30) {
            night.update(&sky, 1, 1.0 / 30.0, false);
        }
        assert!(night.meteors.is_empty() && night.count == 0);
    }
}
