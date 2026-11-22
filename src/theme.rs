use ratatui::style::Color;

pub struct ThemeData {
    pub name: &'static str,
    pub bg: u32,
    pub fg: u32,
    pub sel_bg: Option<u32>,
    pub sel_fg: Option<u32>,
    pub ansi: [u32; 16],
}

include!(concat!(env!("OUT_DIR"), "/themes_data.rs"));

pub const DEFAULT_THEME: &str = "HaX0R Blue";

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub panel: Color,
    pub raised: Color,
    pub frame: Color,
    pub dim: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub lead: Color,
    pub lead_fg: Color,
    pub sel_bg: Color,
    pub sel_fg: Color,
    pub cursor: Color,
    pub hover_bg: Color,
    pub playing: Color,
    pub folder: Color,
    pub header: Color,
    pub warn: Color,
    pub error: Color,
    pub ok: Color,
    pub analyzer: AnalyzerColors,
}

#[derive(Clone, Copy, Debug)]
pub struct AnalyzerColors {
    pub glass: Color,
    pub lit: [Color; 3],
    pub unlit: Color,
    pub label: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shade {
    Main = 0,
    High = 1,
    Top = 2,
}

impl Shade {
    pub fn of(seg: u16, lit: u16) -> Option<Shade> {
        match lit.checked_sub(seg + 1)? {
            0 => Some(Shade::Top),
            1 => Some(Shade::High),
            _ => Some(Shade::Main),
        }
    }
}

impl AnalyzerColors {
    pub fn lit(&self, shade: Shade) -> Color {
        self.lit[shade as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TraceColors {
    pub stops: [u32; 6],
}

pub const BOKEH_HUES: usize = 10;

pub const BOKEH_GROUND: u32 = 0x000000;

const BOKEH_STOPS_AT: [f32; 4] = [0.0, 0.30, 0.62, 1.0];

#[derive(Clone, Copy, Debug)]
pub struct BokehColors {
    pub ramps: [[u32; 4]; BOKEH_HUES],
}

impl BokehColors {
    pub fn at(&self, size: usize, t: f32) -> Color {
        let stops = &self.ramps[size.min(BOKEH_HUES - 1)];
        let t = t.clamp(0.0, 1.0);
        for i in 0..3 {
            let (from, to) = (BOKEH_STOPS_AT[i], BOKEH_STOPS_AT[i + 1]);
            if t <= to {
                return to_color(mix(stops[i], stops[i + 1], (t - from) / (to - from)));
            }
        }
        to_color(stops[3])
    }
}

const SYNTHWAVE_COLORS: ThemeData = ThemeData {
    name: "Synthwave",
    bg: 0x1a0933,
    fg: 0xf5e9ff,
    sel_bg: None,
    sel_fg: None,
    ansi: [
        0x1a0933, 0xff4d7d, 0x2ee6a6, 0xffb800, 0x3a8dff, 0xff3cac, 0x2de2e6, 0xe9dcff, 0x4a2c6d, 0xff7a9c, 0x5cffc8, 0xffd93d, 0x36c6ff, 0xff6ec7,
        0x7df9ff, 0xfff4fb,
    ],
};

pub const GALAXY_KINDS: usize = 9;
pub const GALAXY_STOPS: usize = 6;

#[derive(Clone, Copy, Debug)]
pub struct GalaxyColors {
    pub ground: u32,
    pub stars: [[u32; GALAXY_STOPS]; GALAXY_KINDS],
}

const GALAXY_COLORS: GalaxyColors = GalaxyColors {
    ground: 0x06021a,
    stars: [
        [0x4a0b1b, 0x6e1129, 0x931a38, 0xb8284a, 0xdd4a66, 0xf58a9c],
        [0x6b4d0c, 0x9a7213, 0xc79a1c, 0xe6b930, 0xf7d25f, 0xfff0b3],
        [0x14214f, 0x1d3377, 0x2b4aa3, 0x3f66d0, 0x6b91ee, 0xa9c2ff],
        [0x4a4a55, 0x7b7b88, 0xa9a9b6, 0xd2d2dc, 0xeeeef5, 0xffffff],
        [0x2e1058, 0x44198a, 0x5e2cb8, 0x7a4fd9, 0x9d7cf0, 0xc8b4ff],
        [0x123a1c, 0x1b5a2a, 0x2a8040, 0x45a85c, 0x71cc86, 0xaef0bd],
        [0x0a4a66, 0x0c6e96, 0x1294c4, 0x2cb8ea, 0x6fd6ff, 0xbdeeff],
        [0x16604f, 0x23917b, 0x3cc2a6, 0x62e6c6, 0x9bf7de, 0xd4fff2],
        [0x0b4647, 0x106d6f, 0x1a9a9a, 0x2ec4c0, 0x63e3dc, 0xb0f7f2],
    ],
};

pub fn galaxy_colors() -> GalaxyColors {
    GALAXY_COLORS
}

#[derive(Clone, Copy, Debug)]
pub struct ClubLightsColors {
    pub ground: u32,
    pub beams: [u32; 10],
}

const CLUB_LIGHTS_COLORS: ClubLightsColors = ClubLightsColors {
    ground: 0x0c0c1e,
    beams: [0xff3b30, 0xff7a1a, 0xffc21a, 0xa6ff2e, 0x25f07a, 0x1ee6e0, 0x2aa0ff, 0x7a5cff, 0xd23cff, 0xff3ca8],
};

pub fn club_lights_colors() -> ClubLightsColors {
    CLUB_LIGHTS_COLORS
}

#[derive(Clone, Copy, Debug)]
pub struct LinesColors {
    pub fill: u32,
    pub near: u32,
    pub far: u32,
}

fn hue_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

fn rgb_distance(a: u32, b: u32) -> f32 {
    let (a, b) = (rgb(a), rgb(b));
    (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs()
}

fn lifted_off(c: u32, ground: u32, min_contrast: f32) -> u32 {
    let towards = if contrast(0xFFFFFF, ground) >= contrast(0x000000, ground) { 0xFFFFFF } else { 0x000000 };
    let mut out = c;
    for _ in 0..30 {
        if contrast(out, ground) >= min_contrast {
            break;
        }
        out = mix(out, towards, 0.10);
    }
    out
}

impl ThemeData {
    pub fn lines_colors(&self) -> LinesColors {
        const SCHEMES: [[f32; 3]; 4] = [[120.0; 3], [60.0, 150.0, 150.0], [90.0, 135.0, 135.0], [30.0, 150.0, 180.0]];
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let dark = luminance(bg) < 0.4;

        let mut accents: Vec<(f32, f32, u32)> = [1usize, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14]
            .into_iter()
            .filter_map(|i| {
                let (hue, sat, val) = hsv(a[i]);
                (sat >= 0.22 && val >= 0.25).then_some((hue, sat, a[i]))
            })
            .collect();
        accents.sort_by(|x, y| y.1.total_cmp(&x.1));
        let mut uniq: Vec<(f32, f32, u32)> = Vec::new();
        for p in accents {
            if uniq.iter().all(|q| hue_gap(p.0, q.0) >= 20.0) {
                uniq.push(p);
            }
        }
        let mut best: Option<([usize; 3], f32)> = None;
        for i in 0..uniq.len() {
            for j in i + 1..uniq.len() {
                for k in j + 1..uniq.len() {
                    let mut gaps = [hue_gap(uniq[i].0, uniq[j].0), hue_gap(uniq[i].0, uniq[k].0), hue_gap(uniq[j].0, uniq[k].0)];
                    gaps.sort_by(f32::total_cmp);
                    let off = SCHEMES.iter().map(|s| (0..3).map(|n| (gaps[n] - s[n]).abs()).sum::<f32>()).fold(f32::MAX, f32::min);
                    let chroma = (uniq[i].1 + uniq[j].1 + uniq[k].1) / 3.0;
                    let score = -off + 25.0 * chroma + 0.1 * gaps[1].min(150.0);
                    if best.is_none_or(|(_, b)| score > b) {
                        best = Some(([i, j, k], score));
                    }
                }
            }
        }
        let (line_a, line_b, _third) = match best {
            Some(([i, j, k], _)) => {
                let pairs = [(hue_gap(uniq[i].0, uniq[j].0), i, j, k), (hue_gap(uniq[i].0, uniq[k].0), i, k, j), (hue_gap(uniq[j].0, uniq[k].0), j, k, i)];
                let &(_, x, y, z) = pairs.iter().fold(&pairs[0], |m, p| if p.0 > m.0 { p } else { m });
                let (x, y) = if hue_gap(uniq[x].0, 30.0) > hue_gap(uniq[y].0, 30.0) { (y, x) } else { (x, y) };
                (uniq[x].2, uniq[y].2, uniq[z].2)
            }
            None => (fg, mix(fg, a[12], 0.5), mix(bg, fg, 0.3)),
        };
        let fill = bg;
        let near = lifted_off(line_a, fill, 3.0);
        let mut far = lifted_off(line_b, fill, 3.0);
        if rgb_distance(near, far) < 60.0 {
            let away = if dark { 0xFFFFFF } else { 0x000000 };
            let shades = [mix(near, fill, 0.45), mix(near, fill, 0.30), mix(near, away, 0.45), mix(near, away, 0.65), mix(near, fill, 0.60)];
            let fits = |c: u32| contrast(c, fill) >= 3.0 && rgb_distance(near, c) >= 60.0;
            far = shades.into_iter().find(|&c| fits(c)).unwrap_or(shades[2]);
        }
        LinesColors {
            fill,
            near,
            far,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScopeColors {
    pub ground: Color,
    pub left: Color,
    pub right: Color,
    pub axis: Color,
    pub label: Color,
}

impl ThemeData {
    pub fn scope_colors(&self) -> ScopeColors {
        let l = self.lines_colors();
        ScopeColors {
            ground: to_color(self.bg),
            left: to_color(l.near),
            right: to_color(l.far),
            axis: to_color(mix(self.bg, self.fg, 0.28)),
            label: to_color(mix(self.bg, self.fg, 0.55)),
        }
    }
}

pub fn synthwave_colors() -> SynthColors {
    SYNTHWAVE_COLORS.synth_colors()
}

const SYNTHMOONWAVE_COLORS: SynthColors = SynthColors {
    ground: 0x04051a,
    sky_deep: 0x020a38,
    sky_low: 0x041a5e,
    halo: 0x0e33a0,
    sun: 0xc3cde0,
    sun_hot: 0xf4f6fb,
    grid: 0x3f4c98,
    grid_hot: 0x6c78c4,
    mount: 0x0d0d3e,
    floor_deep: 0x0a0a30,
    floor_glow: 0x1c1c66,
    stars: [0xffffff, 0xc8c8c8, 0x9b9b9b, 0x6e6e6e],
};

pub fn synthmoonwave_colors() -> SynthColors {
    SYNTHMOONWAVE_COLORS
}

#[derive(Clone, Copy, Debug)]
pub struct SynthColors {
    pub ground: u32,
    pub sky_deep: u32,
    pub sky_low: u32,
    pub halo: u32,
    pub sun: u32,
    pub sun_hot: u32,
    pub grid: u32,
    pub grid_hot: u32,
    pub mount: u32,
    pub floor_deep: u32,
    pub floor_glow: u32,
    pub stars: [u32; 4],
}

pub fn rgb_mix(a: u32, b: u32, t: f32) -> u32 {
    mix(a, b, t)
}

pub fn rgb_color(c: u32) -> Color {
    to_color(c)
}

const TRACE_STOPS_AT: [f32; 6] = [0.0, 0.10, 0.30, 0.55, 0.80, 1.0];

impl TraceColors {
    pub fn at(&self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        for i in 0..5 {
            let (from, to) = (TRACE_STOPS_AT[i], TRACE_STOPS_AT[i + 1]);
            if t <= to {
                return to_color(mix(self.stops[i], self.stops[i + 1], (t - from) / (to - from)));
            }
        }
        to_color(self.stops[5])
    }
}

fn rgb(c: u32) -> (f32, f32, f32) {
    (((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32)
}

fn to_color(c: u32) -> Color {
    let (r, g, b) = rgb(c);
    Color::Rgb(r as u8, g as u8, b as u8)
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let m = |x: f32, y: f32| (x + (y - x) * t).round().clamp(0.0, 255.0) as u32;
    (m(ar, br) << 16) | (m(ag, bg) << 8) | m(ab, bb)
}

fn luminance(c: u32) -> f32 {
    let (r, g, b) = rgb(c);
    let ch = |v: f32| {
        let v = v / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
}

fn contrast(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn cursor_shade(a: &[u32; 16], bg: u32, fg: u32) -> u32 {
    const MIN_CONTRAST: f32 = 3.0;
    const MIN_APART: f32 = 90.0;
    let apart = |x: u32, y: u32| {
        let ((xr, xg, xb), (yr, yg, yb)) = (rgb(x), rgb(y));
        ((xr - yr).powi(2) + (xg - yg).powi(2) + (xb - yb).powi(2)).sqrt()
    };
    let taken = [a[10], a[12], fg];
    for i in [13, 14, 11, 9, 5, 6, 3, 1] {
        let c = a[i];
        if contrast(c, bg) >= MIN_CONTRAST && taken.iter().all(|&t| apart(c, t) >= MIN_APART) {
            return c;
        }
    }
    let towards = if luminance(bg) < luminance(fg) { 0xFFFFFF } else { 0x000000 };
    let score = |c: u32| apart(c, a[10]).min(apart(c, a[12]));
    a.iter()
        .copied()
        .chain([fg, towards])
        .filter(|&c| contrast(c, bg) >= MIN_CONTRAST)
        .max_by(|&x, &y| score(x).total_cmp(&score(y)))
        .unwrap_or(fg)
}

fn readable_on(c: u32, bg: u32, fg: u32) -> u32 {
    if contrast(c, bg) >= contrast(c, fg) { bg } else { fg }
}

fn glass(bg: u32, fg: u32) -> u32 {
    const MAX_LUMINANCE: f32 = 0.025;
    let base = if luminance(fg) < luminance(bg) { fg } else { bg };
    let mut c = base;
    let mut t = 0.0;
    while luminance(c) > MAX_LUMINANCE && t < 1.0 {
        t += 0.05;
        c = mix(base, 0x000000, t);
    }
    c
}

pub const HIGH_SHADE: f32 = 0.25;
pub const TOP_SHADE: f32 = 0.45;
pub const UNLIT_SHADE: f32 = 0.88;

fn analyzer_colors(bg: u32, fg: u32, accent: u32) -> AnalyzerColors {
    let glass = glass(bg, fg);
    let main = lit_on(accent, glass, 4.0);
    let lit = [main, mix(main, glass, HIGH_SHADE), mix(main, glass, TOP_SHADE)];
    AnalyzerColors {
        glass: to_color(glass),
        lit: lit.map(to_color),
        unlit: to_color(mix(main, glass, UNLIT_SHADE)),
        label: to_color(lit_on(mix(bg, fg, 0.55), glass, 3.5)),
    }
}

fn lit_on(c: u32, glass: u32, min_contrast: f32) -> u32 {
    let mut out = c;
    let mut t = 0.0;
    while contrast(out, glass) < min_contrast && t < 1.0 {
        t += 0.05;
        out = mix(c, 0xFFFFFF, t);
    }
    out
}

fn leading_shade(accent: u32, track: u32, bg: u32, fg: u32) -> u32 {
    const SHARE: f32 = 0.6;
    let towards = if luminance(bg) < luminance(fg) { 0xFFFFFF } else { 0x000000 };
    let lead = mix(accent, towards, SHARE);
    if contrast(lead, track) >= contrast(accent, track) { lead } else { accent }
}

fn frame_shade(bg: u32, fg: u32) -> u32 {
    let mut t = 0.38;
    while contrast(mix(bg, fg, t), bg) < 1.8 && t < 0.55 {
        t += 0.01;
    }
    mix(bg, fg, t.min(0.55))
}

#[cfg(test)]
pub fn contrast_of(a: Color, b: Color) -> f32 {
    let u = |c: Color| match c {
        Color::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        _ => 0,
    };
    contrast(u(a), u(b))
}

#[cfg(test)]
pub fn luminance_of(c: Color) -> f32 {
    match c {
        Color::Rgb(r, g, b) => luminance(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
        _ => 0.0,
    }
}

impl ThemeData {
    pub fn palette(&self) -> Palette {
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let sel_bg = self.sel_bg.unwrap_or(a[4]);
        let sel_fg = self.sel_fg.unwrap_or_else(|| readable_on(sel_bg, bg, fg));
        let accent = a[12];
        let track = mix(bg, fg, 0.16);
        let lead = leading_shade(accent, track, bg, fg);
        Palette {
            bg: to_color(bg),
            fg: to_color(fg),
            panel: to_color(mix(bg, fg, 0.07)),
            raised: to_color(track),
            frame: to_color(frame_shade(bg, fg)),
            dim: to_color(mix(bg, fg, 0.55)),
            accent: to_color(accent),
            accent_fg: to_color(readable_on(accent, bg, fg)),
            lead: to_color(lead),
            lead_fg: to_color(readable_on(lead, bg, fg)),
            sel_bg: to_color(sel_bg),
            sel_fg: to_color(sel_fg),
            cursor: to_color(cursor_shade(a, bg, fg)),
            hover_bg: to_color(mix(bg, fg, 0.10)),
            playing: to_color(a[10]),
            folder: to_color(a[12]),
            header: to_color(a[11]),
            warn: to_color(a[3]),
            error: to_color(a[1]),
            ok: to_color(a[2]),
            analyzer: analyzer_colors(bg, fg, accent),
        }
    }

    pub fn trace_colors(&self) -> TraceColors {
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let glass = glass(bg, fg);
        let main = lit_on(a[12], glass, 4.0);
        let core = hot_core(main, glass, bg, fg, a);
        let mut stops = [glass, mix(main, glass, 0.80), mix(main, glass, 0.52), main, mix(main, core, 0.45), mix(main, core, 0.88)];
        for i in 1..stops.len() {
            stops[i] = lifted_to(stops[i], luminance(stops[i - 1]));
        }
        TraceColors { stops }
    }

    pub fn bokeh_colors(&self) -> BokehColors {
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let ground = BOKEH_GROUND;
        let main = lit_on(a[12], ground, 4.0);
        let core = hot_core(main, ground, bg, fg, a);
        bokeh_ramps(a, ground, main, core, false)
    }

    pub fn bokeh_light_colors(&self) -> BokehColors {
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let light = if luminance(bg) >= luminance(fg) { bg } else { fg };
        let ground = lifted_to(light, BOKEH_LIGHT_MIN);
        let main = dark_on(a[4], ground, 4.0);
        let dark = if luminance(bg) < luminance(fg) { bg } else { fg };
        let core = if luminance(dark) + 0.04 < luminance(main) && contrast(dark, ground) >= 7.0 { dark } else { mix(main, 0x000000, 0.6) };
        bokeh_ramps(a, ground, main, core, true)
    }
}

const BOKEH_LIGHT_MIN: f32 = 0.55;

fn dark_on(c: u32, ground: u32, min_contrast: f32) -> u32 {
    let mut out = c;
    let mut t = 0.0;
    while contrast(out, ground) < min_contrast && t < 1.0 {
        t += 0.05;
        out = mix(c, 0x000000, t);
    }
    out
}

fn lowered_to(c: u32, ceiling: f32) -> u32 {
    let mut out = c;
    let mut t = 0.0;
    while luminance(out) > ceiling && t < 1.0 {
        t += 0.05;
        out = mix(c, 0x000000, t);
    }
    out
}

fn bokeh_ramps(a: &[u32; 16], ground: u32, main: u32, core: u32, light: bool) -> BokehColors {
    const MIN_GAP: f32 = 24.0;
    let fit = |c: u32| if light { dark_on(c, ground, 3.0) } else { lit_on(c, ground, 3.0) };
    let mut candidates: Vec<(f32, f32, u32)> = [1usize, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14]
        .into_iter()
        .filter_map(|i| {
            let (hue, sat, val) = hsv(a[i]);
            let fitted = fit(a[i]);
            let strength = if light { 1.0 - luminance(fitted) } else { luminance(fitted) };
            (sat >= 0.22 && val >= 0.15).then(|| (hue, sat * (0.5 + 0.5 * strength.sqrt()), fitted))
        })
        .collect();
    candidates.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut picked: Vec<(f32, u32)> = Vec::new();
    for (hue, _, color) in candidates {
        let apart = picked.iter().all(|(h, _)| {
            let d = (hue - h).abs();
            d.min(360.0 - d) >= MIN_GAP
        });
        if apart && picked.len() < BOKEH_HUES {
            picked.push((hue, color));
        }
    }
    let key = |h: f32| (h + 30.0).rem_euclid(360.0);
    picked.sort_by(|x, y| key(x.0).total_cmp(&key(y.0)));
    let mut hues: Vec<u32> = picked.iter().map(|p| p.1).collect();
    if hues.len() >= 2 {
        let mut keys: Vec<f32> = picked.iter().map(|p| key(p.0)).collect();
        while hues.len() < BOKEH_HUES {
            let at = (0..hues.len() - 1).max_by(|&i, &j| (keys[i + 1] - keys[i]).total_cmp(&(keys[j + 1] - keys[j]))).unwrap_or(0);
            let blend = fit(mix(hues[at], hues[at + 1], 0.5));
            hues.insert(at + 1, blend);
            keys.insert(at + 1, (keys[at] + keys[at + 1]) / 2.0);
        }
    } else {
        hues = (0..BOKEH_HUES).map(|k| mix(main, core, 0.62 * k as f32 / (BOKEH_HUES - 1) as f32)).collect();
    }

    let ramp = |h: u32| {
        let mut s;
        if light {
            let deep = if luminance(core) < luminance(h) { core } else { mix(h, 0x000000, 0.5) };
            s = [ground, mix(h, ground, 0.60), h, mix(h, deep, 0.55)];
            for i in 1..4 {
                s[i] = lowered_to(s[i], luminance(s[i - 1]));
            }
        } else {
            let hot = if luminance(core) > luminance(h) { core } else { mix(h, 0xFFFFFF, 0.5) };
            s = [ground, mix(h, ground, 0.60), h, mix(h, hot, 0.55)];
            for i in 1..4 {
                s[i] = lifted_to(s[i], luminance(s[i - 1]));
            }
        }
        s
    };
    BokehColors { ramps: std::array::from_fn(|i| ramp(hues[i])) }
}

impl ThemeData {
}

impl ThemeData {
    pub fn synth_colors(&self) -> SynthColors {
        let (bg, fg, a) = (self.bg, self.fg, &self.ansi);
        let glass = glass(bg, fg);
        let main = lit_on(a[12], glass, 4.0);
        let core = hot_core(main, glass, bg, fg, a);
        let ground = mix(glass, main, 0.16);
        let warm = accent_near(a, 48.0, ground).unwrap_or_else(|| mix(main, core, 0.45));
        let magenta = accent_near(a, 322.0, ground).unwrap_or(main);
        let cool = accent_near(a, 195.0, ground).unwrap_or(main);
        let sun = lit_on(warm, ground, 4.0);
        let grid = lit_on(cool, ground, 4.5);
        SynthColors {
            ground,
            sky_deep: mix(glass, magenta, 0.40),
            sky_low: mix(glass, magenta, 0.72),
            halo: mix(magenta, warm, 0.55),
            sun,
            sun_hot: lifted_to(mix(sun, core, 0.50), luminance(sun)),
            grid,
            grid_hot: lifted_to(mix(grid, core, 0.55), luminance(grid)),
            mount: mix(glass, cool, 0.22),
            floor_deep: mix(glass, cool, 0.20),
            floor_glow: mix(cool, core, 0.62),
            stars: [fg, mix(fg, bg, 0.25), mix(fg, bg, 0.45), mix(fg, bg, 0.65)],
        }
    }
}

fn accent_near(a: &[u32; 16], target: f32, ground: u32) -> Option<u32> {
    [1usize, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14]
        .into_iter()
        .filter_map(|i| {
            let (hue, sat, val) = hsv(a[i]);
            let d = (hue - target).abs();
            let d = d.min(360.0 - d);
            (sat >= 0.22 && val >= 0.15 && d <= 75.0).then(|| (d - 25.0 * sat, a[i]))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, c)| lit_on(c, ground, 3.0))
}

fn hsv(c: u32) -> (f32, f32, f32) {
    let (r, g, b) = rgb(c);
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let d = hi - lo;
    let hue = if d <= 0.0 {
        0.0
    } else if hi == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if hi == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (hue, if hi <= 0.0 { 0.0 } else { d / hi }, hi)
}

fn lifted_to(c: u32, floor: f32) -> u32 {
    let mut out = c;
    let mut t = 0.0;
    while luminance(out) < floor && t < 1.0 {
        t += 0.05;
        out = mix(c, 0xFFFFFF, t);
    }
    out
}

fn hot_core(main: u32, glass: u32, bg: u32, fg: u32, ansi: &[u32; 16]) -> u32 {
    let need = luminance(main) + 0.08;
    [fg, ansi[15], ansi[7], bg]
        .into_iter()
        .find(|&c| luminance(c) >= need && contrast(c, glass) >= 7.0)
        .unwrap_or_else(|| mix(main, 0xFFFFFF, 0.75))
}

pub fn index_of(name: &str) -> Option<usize> {
    THEMES.iter().position(|t| t.name.eq_ignore_ascii_case(name))
}

pub fn default_index() -> usize {
    index_of(DEFAULT_THEME).unwrap_or(0)
}

#[cfg(test)]
mod trace_colors_tests {
    use super::*;

    fn check(label: &str, bg: u32, fg: u32, ansi: [u32; 16]) {
        let theme = ThemeData { name: "test", bg, fg, sel_bg: None, sel_fg: None, ansi };
        let s = theme.trace_colors().stops;
        let lum = |i: usize| luminance(s[i]);
        assert!(lum(0) <= 0.03, "{label}: window not dark ({:06x})", s[0]);
        assert!(contrast(s[3], s[0]) >= 4.0, "{label}: body too faint ({:.2})", contrast(s[3], s[0]));
        for i in 0..5 {
            assert!(lum(i + 1) >= lum(i), "{label}: stop {} ({:06x}) is darker than stop {i} ({:06x})", i + 1, s[i + 1], s[i]);
        }
        assert!(lum(5) >= lum(3), "{label}: the hot end is darker than the body");
        assert!(contrast(s[5], s[0]) >= 4.0, "{label}: the hot end is faint ({:.2})", contrast(s[5], s[0]));
        assert!(lum(1) <= lum(3) && lum(2) <= lum(3), "{label}: dim stops brighter than the body");
    }

    #[test]
    fn the_ramp_works_for_every_theme() {
        for t in THEMES {
            check(t.name, t.bg, t.fg, t.ansi);
            let ramp = t.trace_colors();
            let mut top = 0.0f32;
            for step in 0..=200 {
                let now = luminance_of(ramp.at(step as f32 / 200.0));
                assert!(now >= top - 0.012, "{}: the ramp dips at {step}/200 ({now:.3} after {top:.3})", t.name);
                top = top.max(now);
            }
        }
        assert!(THEMES.len() > 900);
    }

    #[test]
    fn the_ramp_works_for_themes_that_do_not_exist_yet() {
        let same = |c: u32| [c; 16];
        check("all black", 0x000000, 0x000000, same(0x000000));
        check("all white", 0xFFFFFF, 0xFFFFFF, same(0xFFFFFF));
        check("all mid gray", 0x808080, 0x808080, same(0x808080));
        check("white on black, gray accent", 0x000000, 0xFFFFFF, same(0x777777));
        check("black on white, white accent", 0xFFFFFF, 0x000000, same(0xFFFFFF));
        check("accent is the background", 0x1E1E2E, 0xCDD6F4, same(0x1E1E2E));
        check("accent is the text", 0x1E1E2E, 0xCDD6F4, same(0xCDD6F4));
        check("pure neon on black", 0x000000, 0x00FF00, same(0xFF00FF));
        check("pure blue", 0x000010, 0x0000FF, same(0x0000FF));
        check("pure red on red", 0x400000, 0xFF0000, same(0xFF0000));
        check("dark window, dark text", 0x101010, 0x202020, same(0x303030));
        check("light window, light text", 0xF0F0F0, 0xE0E0E0, same(0xD0D0D0));
        check("inverted: light text on dark and ansi all dark", 0x000000, 0xFFFFFF, same(0x010101));

        let mut x: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 40) as u32
        };
        for n in 0..4000 {
            let (bg, fg) = (next(), next());
            let mut ansi = [0u32; 16];
            for c in &mut ansi {
                *c = next();
            }
            check(&format!("random theme {n}"), bg, fg, ansi);
        }
    }

    #[test]
    fn the_ramp_is_plain_colors_from_the_window_to_the_hot_end() {
        for t in THEMES {
            let ramp = t.trace_colors();
            assert_eq!(ramp.at(0.0), to_color(ramp.stops[0]), "{}", t.name);
            assert_eq!(ramp.at(1.0), to_color(ramp.stops[5]), "{}", t.name);
            assert_eq!(ramp.at(0.55), to_color(ramp.stops[3]), "{}", t.name);
            for step in 0..=100 {
                assert!(matches!(ramp.at(step as f32 / 100.0), Color::Rgb(..)));
            }
            assert_eq!(ramp.at(-3.0), ramp.at(0.0));
            assert_eq!(ramp.at(7.0), ramp.at(1.0));
        }
    }

    #[test]
    fn the_ramp_depends_on_the_colors_and_not_the_name() {
        let dracula = THEMES.iter().find(|t| t.name == "Dracula").unwrap();
        let renamed = ThemeData { name: "A theme from 2031", bg: dracula.bg, fg: dracula.fg, sel_bg: None, sel_fg: None, ansi: dracula.ansi };
        assert_eq!(renamed.trace_colors().stops, dracula.trace_colors().stops);
        let other = THEMES.iter().find(|t| t.name == "Nord").unwrap();
        assert_ne!(other.trace_colors().stops, dracula.trace_colors().stops);
        let mut ramps: Vec<[u32; 6]> = THEMES.iter().map(|t| t.trace_colors().stops).collect();
        ramps.sort_unstable();
        ramps.dedup();
        assert!(ramps.len() > 500, "only {} different ramps in {} themes", ramps.len(), THEMES.len());
    }
}

#[cfg(test)]
mod bokeh_colors_tests {
    use super::*;

    #[test]
    fn the_ramps_work_for_every_theme() {
        for t in THEMES {
            let c = t.bokeh_colors();
            let ground = c.ramps[0][0];
            assert_eq!(ground, 0x000000, "{}: the background is not black", t.name);
            for (i, s) in c.ramps.iter().enumerate() {
                assert_eq!(s[0], ground, "{}: size {i} has a ground of its own", t.name);
                for k in 1..4 {
                    assert!(luminance(s[k]) >= luminance(s[k - 1]), "{}: size {i}, stop {k} is darker than the one below", t.name);
                }
                assert!(contrast(s[2], ground) >= 2.9, "{}: size {i} is faint on the ground ({:.2})", t.name, contrast(s[2], ground));
            }
        }
    }
}

#[cfg(test)]
mod bokeh_light_colors_tests {
    use super::*;

    #[test]
    fn the_light_ramps_work_for_every_theme() {
        for t in THEMES {
            let c = t.bokeh_light_colors();
            let ground = c.ramps[0][0];
            assert!(luminance(ground) >= BOKEH_LIGHT_MIN - 0.01, "{}: the background {ground:06x} is not light", t.name);
            let light = if luminance(t.bg) >= luminance(t.fg) { t.bg } else { t.fg };
            assert!(luminance(light) < BOKEH_LIGHT_MIN || ground == light, "{}: not the theme's own light color", t.name);
            for (i, s) in c.ramps.iter().enumerate() {
                assert_eq!(s[0], ground, "{}: size {i} has a ground of its own", t.name);
                for k in 1..4 {
                    assert!(luminance(s[k]) <= luminance(s[k - 1]), "{}: size {i}, stop {k} is lighter than the one below", t.name);
                }
                assert!(contrast(s[2], ground) >= 2.9, "{}: size {i} is faint on the ground ({:.2})", t.name, contrast(s[2], ground));
            }
        }
    }

    #[test]
    fn the_light_background_follows_the_theme() {
        let mut grounds: Vec<u32> = THEMES.iter().map(|t| t.bokeh_light_colors().ramps[0][0]).collect();
        grounds.sort_unstable();
        grounds.dedup();
        assert!(grounds.len() > 100, "only {} backgrounds", grounds.len());
    }
}

#[cfg(test)]
mod synth_colors_tests {
    use super::*;

    #[test]
    fn the_scene_takes_its_own_hues() {
        let c = synthwave_colors();
        let hue = |x: u32| hsv(x).0;
        assert!((35.0..65.0).contains(&hue(c.sun)), "sun hue {}", hue(c.sun));
        assert!((190.0..215.0).contains(&hue(c.grid)), "grid hue {}", hue(c.grid));
        assert!((285.0..340.0).contains(&hue(c.sky_low)), "sky hue {}", hue(c.sky_low));
    }

    #[test]
    fn the_moon_scene_is_midnight_and_phthalo() {
        let c = synthmoonwave_colors();
        let hue = |x: u32| hsv(x).0;
        assert!(luminance(c.sun) > 0.55 && hsv(c.sun).1 < 0.15, "moon {:06x}", c.sun);
        assert!(luminance(c.sun_hot) > luminance(c.sun));
        for (name, x) in [("sky_deep", c.sky_deep), ("sky_low", c.sky_low), ("halo", c.halo)] {
            assert!((222.0..236.0).contains(&hue(x)), "{name} hue {}", hue(x));
            assert!(hsv(x).1 > 0.85, "{name} should be a saturated blue");
        }
        assert!(luminance(c.ground) < luminance(c.sky_deep));
        assert!(luminance(c.sky_deep) < luminance(c.sky_low) && luminance(c.sky_low) < luminance(c.halo));
        assert!(luminance(c.halo) < 0.06, "halo too light: {}", luminance(c.halo));
        for (name, x) in [("mount", c.mount), ("floor_deep", c.floor_deep), ("floor_glow", c.floor_glow)] {
            assert!((230.0..245.0).contains(&hue(x)), "{name} hue {}", hue(x));
            assert!(luminance(x) < 0.025, "{name} too light: {}", luminance(x));
        }
        assert!(contrast(c.grid, c.floor_deep) >= 2.0 && contrast(c.grid_hot, c.floor_deep) >= 3.5);
    }
}

#[cfg(test)]
mod lines_colors_tests {
    use super::*;

    #[test]
    fn every_theme_gives_lines_that_stand_out_and_differ() {
        for t in THEMES {
            let c = t.lines_colors();
            assert!(contrast(c.near, c.fill) >= 2.5, "{}: near {:06x} on {:06x} ({:.2})", t.name, c.near, c.fill, contrast(c.near, c.fill));
            assert!(contrast(c.far, c.fill) >= 2.5, "{}: far {:06x} on {:06x} ({:.2})", t.name, c.far, c.fill, contrast(c.far, c.fill));
            assert!(rgb_distance(c.near, c.far) >= 40.0, "{}: nearly one color for both kinds of line ({:06x}, {:06x})", t.name, c.near, c.far);
        }
    }

    #[test]
    fn the_colors_follow_the_theme() {
        let mut sets: Vec<[u32; 3]> = THEMES.iter().map(|t| {
            let c = t.lines_colors();
            [c.fill, c.near, c.far]
        }).collect();
        sets.sort_unstable();
        sets.dedup();
        assert!(sets.len() > 500, "only {} different sets in {} themes", sets.len(), THEMES.len());
    }
}

#[cfg(test)]
mod galaxy_colors_tests {
    use super::*;

    #[test]
    fn the_galaxy_colors_are_rich_and_brighten() {
        let own = galaxy_colors();
        assert!(own.stars.iter().all(|s| contrast(s[5], own.ground) >= 4.0 && contrast(s[3], own.ground) >= 2.0));
        assert!(own.stars.iter().all(|s| s.windows(2).all(|w| luminance(w[1]) > luminance(w[0]))), "every color should brighten shade by shade");
    }
}
