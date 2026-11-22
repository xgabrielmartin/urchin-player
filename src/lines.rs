use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::bokeh::{
    fade_curve, sticky, Bands, AT_REST, CLASSES, FADE_IN, FADE_OUT, MIN_STEP, PULSE_ATTACK, PULSE_FALL, SILENCE_HOLD, SONG_SHARE, STALE,
};
use crate::spectrum::Tap;
use crate::theme::{rgb_color, rgb_mix, LinesColors};

const SPACING: usize = 1;

const DIM_SHARE: f32 = 0.10;
const LIT_SHARE: f32 = 0.85;
const TIP_SHARE: f32 = 0.15;
const LINE: char = '│';
const TIP: char = '╷';

#[derive(Clone, Copy, PartialEq, Debug)]
struct Plan {
    cols: usize,
    rows: usize,
    steps: usize,
}

impl Plan {
    fn new(cols: usize, rows: usize) -> Plan {
        let mut plan = Plan { cols, rows, steps: 1 };
        plan.steps = (0..cols).filter_map(|c| plan.line_at(c)).max().map_or(1, |j| j + 1);
        plan
    }

    fn line_at(&self, c: usize) -> Option<usize> {
        let x = (c as f32 + 0.5 - self.cols as f32 / 2.0).abs();
        let first = if self.cols % 2 == 1 { 0.0 } else { SPACING as f32 - 0.5 };
        if x < first - 0.01 {
            return None;
        }
        let d = (x - first) / SPACING as f32;
        let j = d.round();
        ((d - j).abs() < 0.01).then_some(j as usize)
    }

    fn out(&self, j: usize) -> f32 {
        if self.steps <= 1 { 0.0 } else { j as f32 / (self.steps - 1) as f32 }
    }

    fn pulse_of(&self, j: usize, pulse: &[f32; CLASSES]) -> f32 {
        let at = self.out(j) * (CLASSES - 1) as f32;
        let (lo, frac) = (at.floor() as usize, at.fract());
        let hi = (lo + 1).min(CLASSES - 1);
        pulse[lo.min(CLASSES - 1)] * (1.0 - frac) + pulse[hi] * frac
    }

    fn steps_tall(&self) -> usize {
        self.rows.clamp(1, 200)
    }
}

fn rgb_luma(c: u32) -> f32 {
    0.2126 * ((c >> 16) & 0xff) as f32 + 0.7152 * ((c >> 8) & 0xff) as f32 + 0.0722 * (c & 0xff) as f32
}

struct Look {
    colors: LinesColors,
    fill: Color,
}

impl Look {
    fn new(colors: LinesColors) -> Look {
        Look { colors, fill: rgb_color(colors.fill) }
    }

    fn cell(&self, out: f32, lit: f32, top: f32, shows: f32) -> u32 {
        let c = &self.colors;
        let base = rgb_mix(c.near, c.far, out.clamp(0.0, 1.0));
        let dim = rgb_mix(c.fill, base, DIM_SHARE);
        let away = if rgb_luma(c.fill) < 128.0 { 0xFFFFFF } else { 0x000000 };
        let soft = rgb_mix(c.fill, base, LIT_SHARE);
        let bright = rgb_mix(soft, away, TIP_SHARE);
        let on = rgb_mix(soft, bright, top.clamp(0.0, 1.0));
        rgb_mix(c.fill, rgb_mix(dim, on, lit.clamp(0.0, 1.0)), shows)
    }
}

pub struct Lines {
    bands: Bands,
    columns: usize,
    rows: u16,
    pulse: [f32; CLASSES],
    shown: Vec<u8>,
    rows_shown: usize,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
    presence: f32,
    quiet_for: f32,
    silent: bool,
    look: Option<(usize, Look)>,
}

impl Default for Lines {
    fn default() -> Self {
        Lines::new()
    }
}

impl Lines {
    pub fn new() -> Lines {
        Lines {
            bands: Bands::new(),
            columns: 0,
            rows: 0,
            pulse: [0.0; CLASSES],
            shown: Vec::new(),
            rows_shown: 0,
            last: None,
            last_written: 0,
            quiet_since: None,
            presence: 0.0,
            quiet_for: SILENCE_HOLD,
            silent: true,
            look: None,
        }
    }

    pub fn use_theme(&mut self, key: usize, colors: impl FnOnce() -> LinesColors) {
        if self.look.as_ref().map(|l| l.0) != Some(key) {
            self.look = Some((key, Look::new(colors())));
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

    pub fn update(&mut self, tap: &Tap, columns: usize, rows: u16, now: Instant) {
        (self.columns, self.rows) = (columns.max(1), rows.max(1));
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
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        let Some((_, look)) = &self.look else { return };
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

        let shows = fade_curve(self.presence);
        if shows <= 0.0 {
            self.shown.clear();
            for y in 0..rows {
                for c in 0..cols {
                    put(area.x as usize + c, area.y as usize + y, ' ', look.fill, look.fill);
                }
            }
            return;
        }

        let plan = Plan::new(cols, rows);
        let n = plan.steps;
        let tall = plan.steps_tall();
        if self.shown.len() != n || self.rows_shown != rows {
            self.shown = vec![0; n];
            self.rows_shown = rows;
        }
        let height: Vec<usize> = self
            .shown
            .iter_mut()
            .enumerate()
            .map(|(j, shown)| {
                let want = (plan.pulse_of(j, &self.pulse) * tall as f32 + 0.5) as usize;
                sticky(shown, want.min(tall), tall)
            })
            .collect();

        for x in 0..cols {
            let line = plan.line_at(x);
            for y in 0..rows {
                let (ox, oy) = (area.x as usize + x, area.y as usize + y);
                match line {
                    Some(j) => {
                        let from_bottom = rows - 1 - y;
                        let (glyph, lit, top) = match height[j] {
                            h if h == 0 || from_bottom >= h => (LINE, 0.0, 0.0),
                            h if from_bottom + 1 == h => (TIP, 1.0, 1.0),
                            _ => (LINE, 1.0, 0.0),
                        };
                        put(ox, oy, glyph, rgb_color(look.cell(plan.out(j), lit, top, shows)), look.fill);
                    }
                    None => put(ox, oy, ' ', look.fill, look.fill),
                }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{index_of, THEMES};
    use std::f32::consts::TAU;
    use std::time::Duration;

    fn rate_of(tap: &Tap) -> f32 {
        tap.rate() as f32
    }

    fn run(s: &mut Lines, tap: &Tap, (columns, rows): (usize, u16), start: f32, secs: f32, signal: impl Fn(f32) -> f32, clock: &mut Instant) {
        let rate = rate_of(tap);
        let per = (rate / 30.0) as usize;
        for f in 0..(secs * 30.0) as usize {
            let mut w = tap.writer();
            for i in 0..per {
                w.push(signal(start + (f * per + i) as f32 / rate));
            }
            drop(w);
            *clock += Duration::from_millis(33);
            s.update(tap, columns, rows, *clock);
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

    fn dracula() -> usize {
        index_of("Dracula").unwrap()
    }

    fn painted(s: &mut Lines, theme: usize, (columns, rows): (usize, u16)) -> Buffer {
        s.use_theme(theme, || THEMES[theme].lines_colors());
        let area = Rect::new(0, 0, columns as u16, rows);
        let mut buf = Buffer::empty(area);
        s.paint(&mut buf, area);
        buf
    }

    fn playing(size: (usize, u16)) -> (Lines, Tap) {
        let (tap, mut s) = (Tap::new(), Lines::new());
        let mut clock = Instant::now();
        run(&mut s, &tap, size, 0.0, 1.5, music, &mut clock);
        (s, tap)
    }

    fn symbol(buf: &Buffer, x: u16, y: u16) -> String {
        let s = buf[(x, y)].symbol();
        if s.is_empty() { " ".into() } else { s.into() }
    }

    #[test]
    fn nothing_played_is_one_flat_color() {
        let (tap, mut s) = (Tap::new(), Lines::new());
        s.update(&tap, 85, 12, Instant::now());
        let buf = painted(&mut s, dracula(), (85, 12));
        let fill = rgb_color(THEMES[dracula()].lines_colors().fill);
        assert!(buf.content.iter().all(|c| c.bg == fill && c.fg == fill && c.symbol() == " "));
        assert!(!s.is_active());
    }

    #[test]
    fn the_lines_are_only_light_verticals_and_half_height_tips() {
        let (mut s, _tap) = playing((85, 12));
        let buf = painted(&mut s, dracula(), (85, 12));
        let mut lines = 0;
        for y in 0..12 {
            for x in 0..85 {
                let g = symbol(&buf, x, y);
                assert!(" │╷".contains(&g), "unexpected {g:?} at {x},{y}");
                if g != " " {
                    lines += 1;
                }
            }
        }
        assert_eq!(lines, (2 * (42 / SPACING) + 1) * 12, "{lines} line characters");
    }

    #[test]
    fn every_line_runs_the_whole_height_at_an_even_spacing_and_the_picture_is_symmetric() {
        for &(cols, rows) in &[(85usize, 12u16), (84, 12), (120, 24), (200, 50), (31, 5)] {
            let (mut s, _tap) = playing((cols, rows));
            let buf = painted(&mut s, dracula(), (cols, rows));
            let mut at = Vec::new();
            for x in 0..cols as u16 {
                let col: Vec<String> = (0..rows).map(|y| symbol(&buf, x, y)).collect();
                let lit = col.iter().filter(|g| *g != " ").count();
                assert!(lit == 0 || lit == rows as usize, "{cols}x{rows}: column {x} is only partly a line");
                assert!(col.iter().filter(|g| *g != "│").count() <= 1 && col.iter().all(|g| g == "│" || g == "╷"), "{cols}x{rows}: column {x} has more than one tip");
                if lit > 0 {
                    at.push(x as usize);
                }
            }
            assert!(at.windows(2).all(|w| w[1] - w[0] == SPACING || (w[1] - w[0] == 2 * SPACING - 1 && cols % 2 == 0)), "{cols}x{rows}: {at:?}");
            for &x in &at {
                assert!(at.contains(&(cols - 1 - x)), "{cols}x{rows}: column {x} has no twin");
            }
        }
    }

    #[test]
    fn the_lit_part_brightens_to_the_tip_in_the_same_color() {
        let hue = |c: u32| {
            let (r, g, b) = (((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32);
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            let d = hi - lo;
            if d <= 0.0 {
                0.0
            } else if hi == r {
                60.0 * ((g - b) / d).rem_euclid(6.0)
            } else if hi == g {
                60.0 * ((b - r) / d + 2.0)
            } else {
                60.0 * ((r - g) / d + 4.0)
            }
        };
        for name in ["Dracula", "Nord", "Gruvbox Dark", "Solarized Dark", "Solarized Light", "Gruvbox Light"] {
            let Some(theme) = index_of(name) else { continue };
            let look = Look::new(THEMES[theme].lines_colors());
            let fill = look.colors.fill;
            let away = |c: u32| (rgb_luma(c) - rgb_luma(fill)).abs();
            for out in [0.0f32, 0.5, 1.0] {
                let (dim, bottom, tip) = (look.cell(out, 0.0, 0.0, 1.0), look.cell(out, 1.0, 0.0, 1.0), look.cell(out, 1.0, 1.0, 1.0));
                assert!(away(bottom) > 1.5 * away(dim), "{name}: {dim:06x} to {bottom:06x}");
                assert!(away(tip) > 1.04 * away(bottom), "{name}: {bottom:06x} to the tip {tip:06x}");
                let (h0, h1) = (hue(bottom), hue(tip));
                assert!((h0 - h1).abs().min(360.0 - (h0 - h1).abs()) < 12.0, "{name}: the hue went from {h0} to {h1}");
            }
        }
    }

    #[test]
    fn a_line_lights_from_the_bottom_up_with_the_tip_brightest() {
        let (mut s, _tap) = playing((85, 12));
        let theme = dracula();
        let buf = painted(&mut s, theme, (85, 12));
        let fill = THEMES[theme].lines_colors().fill;
        let away = |x: u16, y: u16| match buf[(x, y)].fg {
            Color::Rgb(r, g, b) => (rgb_luma(((r as u32) << 16) | ((g as u32) << 8) | b as u32) - rgb_luma(fill)).abs(),
            _ => 0.0,
        };
        let x = 42u16;
        let col: Vec<f32> = (0..12u16).rev().map(|y| away(x, y)).collect();
        let top_i = (0..col.len()).fold(0, |m, i| if col[i] > col[m] { i } else { m });
        assert!(top_i > 0, "nothing lit: {col:?}");
        assert!(col[..=top_i].windows(2).all(|w| w[1] >= w[0] - 0.01), "{col:?}");
        let dim = col[col.len() - 1];
        assert!(col[top_i + 1..].iter().all(|&v| (v - dim).abs() < 0.01), "{col:?}");
        assert!(col[0] > 1.5 * dim && col[top_i] > col[0], "{col:?}");
        let (tap, mut b) = (Tap::new(), Lines::new());
        let mut clock = Instant::now();
        run(&mut b, &tap, (85, 12), 0.0, 1.5, |t| 0.5 * (TAU * 60.0 * t).sin(), &mut clock);
        let buf = painted(&mut b, theme, (85, 12));
        let away = |x: u16, y: u16| match buf[(x, y)].fg {
            Color::Rgb(r, g, b) => (rgb_luma(((r as u32) << 16) | ((g as u32) << 8) | b as u32) - rgb_luma(fill)).abs(),
            _ => 0.0,
        };
        let lit = |x: u16| (0..12u16).filter(|&y| away(x, y) > away(x, 0) + 1.0).count();
        assert!(lit(42) > lit(0) + 2, "{} against {}", lit(42), lit(0));
    }

    #[test]
    fn each_range_moves_the_lines_that_sit_at_it() {
        let (tap, mut s) = (Tap::new(), Lines::new());
        let mut clock = Instant::now();
        let rate = rate_of(&tap);
        let bass = move |t: f32| 0.5 * (TAU * 60.0 * t).sin();
        let treble = move |t: f32| 0.5 * (TAU * 9_000.0 * t).sin();
        run(&mut s, &tap, (85, 12), 0.0, 1.5, bass, &mut clock);
        let p = s.pulses();
        assert!(p[0] > 0.5 && p[9] < 0.2, "a 60 Hz tone: {p:?}");
        run(&mut s, &tap, (85, 12), 1.5, 1.5, treble, &mut clock);
        let p = s.pulses();
        assert!(p[8] > 0.5 && p[0] < 0.2, "a 9 kHz tone: {p:?} (rate {rate})");
    }

    #[test]
    fn it_fits_any_window() {
        for &(cols, rows) in &[(1usize, 1u16), (8, 2), (20, 4), (40, 6), (85, 12), (120, 24), (200, 50), (300, 12), (30, 60)] {
            let (mut s, _tap) = playing((cols, rows));
            let buf = painted(&mut s, dracula(), (cols, rows));
            assert_eq!(buf.content.len(), cols * rows as usize);
            let fill = rgb_color(THEMES[dracula()].lines_colors().fill);
            let lit = buf.content.iter().filter(|c| c.bg != fill || c.fg != fill).count();
            assert!(cols < 20 || lit > 0, "{cols}x{rows}: nothing drawn");
        }
    }

    #[test]
    fn the_picture_fades_to_the_flat_color_when_the_music_stops() {
        let (mut s, tap) = playing((85, 12));
        assert!(s.is_active() && s.presence() > 0.99);
        let mut clock = Instant::now() + Duration::from_secs(5);
        run(&mut s, &tap, (85, 12), 0.0, 4.0, |_| 0.0, &mut clock);
        assert!(s.presence() < 0.01, "presence {}", s.presence());
        let buf = painted(&mut s, dracula(), (85, 12));
        let fill = rgb_color(THEMES[dracula()].lines_colors().fill);
        assert!(buf.content.iter().all(|c| c.bg == fill && c.symbol() == " "));
        assert!(!s.is_active());
    }

    #[test]
    fn the_theme_is_only_read_when_it_changes() {
        let mut s = Lines::new();
        let mut asked = 0;
        for t in [0usize, 0, 0, 1, 1, 0] {
            s.use_theme(t, || {
                asked += 1;
                THEMES[t].lines_colors()
            });
        }
        assert_eq!(asked, 3);
    }

    #[test]
    fn a_redraw_that_comes_too_soon_changes_nothing() {
        let (tap, mut s) = (Tap::new(), Lines::new());
        let mut clock = Instant::now();
        run(&mut s, &tap, (85, 12), 0.0, 1.0, music, &mut clock);
        let before = s.pulses();
        s.update(&tap, 85, 12, clock + Duration::from_millis(5));
        assert_eq!(before, s.pulses());
    }

    #[test]
    fn every_theme_paints_lines() {
        let (mut s, _tap) = playing((85, 12));
        for (t, _) in THEMES.iter().enumerate() {
            let buf = painted(&mut s, t, (85, 12));
            let has = |p: &dyn Fn(&str) -> bool| (0..12u16).any(|y| (0..85u16).any(|x| p(&symbol(&buf, x, y))));
            assert!(has(&|g| g == "│" || g == "╷"), "{}: no lines", THEMES[t].name);
        }
    }

    #[test]
    fn a_line_every_spacing_columns() {
        let plan = Plan::new(120, 24);
        assert_eq!((0..120).filter(|&c| plan.line_at(c).is_some()).count(), 120 / SPACING);
    }

    #[test]
    fn a_line_follows_the_range_it_sits_at() {
        let plan = Plan::new(120, 24);
        let mut pulse = [0.0f32; CLASSES];
        pulse[0] = 1.0;
        assert!(plan.pulse_of(0, &pulse) > 0.99);
        assert!(plan.pulse_of(plan.steps - 1, &pulse) < 0.01);
        pulse = [0.0; CLASSES];
        pulse[CLASSES - 1] = 1.0;
        assert!(plan.pulse_of(plan.steps - 1, &pulse) > 0.99 && plan.pulse_of(0, &pulse) < 0.01);
    }

    #[test]
    #[ignore]
    fn dump_a_picture() {
        let Ok(path) = std::env::var("URCHIN_LINES_DUMP") else { return };
        let theme = index_of(&std::env::var("URCHIN_LINES_THEME").unwrap_or_else(|_| "Dracula".into())).expect("no such theme");
        let size = std::env::var("URCHIN_LINES_SIZE").unwrap_or_else(|_| "85x12".into());
        let (c, r) = size.split_once('x').expect("size like 85x12");
        let (cols, rows) = (c.parse::<usize>().unwrap(), r.parse::<u16>().unwrap());
        let (mut s, _tap) = playing((cols, rows));
        let buf = painted(&mut s, theme, (cols, rows));
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
