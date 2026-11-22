use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::{Chart, Dataset, Widget};
use realfft::{RealFftPlanner, RealToComplex};
use crate::scope_tui::display::oscilloscope::Oscilloscope;
use crate::scope_tui::display::spectroscope::Spectroscope;
use crate::scope_tui::display::vectorscope::Vectorscope;
use crate::scope_tui::display::{update_value_f, update_value_i, Dimension, DisplayMode, GraphConfig};
use crate::scope_tui::music::Note;

use crate::spectrum::Tap;
use crate::theme::ScopeColors;

const CHUNK: u32 = 512;
const CHUNKS: u32 = 4;
const BUFFER: u32 = 2048;
const STALE: Duration = Duration::from_millis(200);

const NOTE_EVERY: f32 = 0.25;
const NOTE_FFT: usize = 8192;
const NOTE_LO: f32 = 27.5;
const NOTE_HI: f32 = 4186.0;
const NOTE_PROMINENCE: f32 = 25.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Oscilloscope,
    Vectorscope,
    Spectroscope,
}

impl Mode {
    pub fn switch(self) -> Mode {
        match self {
            Mode::Oscilloscope => Mode::Vectorscope,
            Mode::Vectorscope => Mode::Spectroscope,
            Mode::Spectroscope => Mode::Oscilloscope,
        }
    }

    pub fn switch_reverse(self) -> Mode {
        match self {
            Mode::Oscilloscope => Mode::Spectroscope,
            Mode::Vectorscope => Mode::Oscilloscope,
            Mode::Spectroscope => Mode::Vectorscope,
        }
    }

    pub const fn slot_shift(self) -> i16 {
        match self {
            Mode::Oscilloscope => -3,
            Mode::Spectroscope => -1,
            Mode::Vectorscope => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Ignored,
    Handled,
    Switched(Mode),
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct Meters {
    pub note: Option<(String, f32, f32)>,
    note_age: f32,
}

pub struct Scopes {
    oscilloscope: Oscilloscope,
    vectorscope: Vectorscope,
    spectroscope: Spectroscope,
    graph: GraphConfig,
    channels: Vec<Vec<f64>>,
    left: Vec<f32>,
    right: Vec<f32>,
    pub meters: Meters,
    tuned: Option<(String, f32)>,
    fft: Arc<dyn RealToComplex<f32>>,
    note_in: [Vec<f32>; 2],
    window: Vec<f32>,
    spectrum: Vec<realfft::num_complex::Complex<f32>>,
    theme: Option<usize>,
    colors: Option<ScopeColors>,
    last: Option<Instant>,
    last_written: usize,
    quiet_since: Option<Instant>,
}

impl Default for Scopes {
    fn default() -> Self {
        Scopes::new()
    }
}

impl Scopes {
    pub fn new() -> Scopes {
        let graph = GraphConfig {
            pause: false,
            samples: CHUNK * CHUNKS,
            initial_samples: CHUNK,
            chunks: CHUNKS,
            sampling_rate: 48_000,
            scale: 1.0,
            scatter: false,
            references: true,
            marker_type: Marker::Braille,
            palette: vec![Color::Red, Color::Yellow, Color::Cyan, Color::Magenta],
            labels_color: Color::Cyan,
            axis_color: Color::DarkGray,
        };
        let n = (CHUNK * CHUNKS) as usize;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(NOTE_FFT);
        let spectrum = fft.make_output_vec();
        let window = (0..NOTE_FFT).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / NOTE_FFT as f32).cos()).collect();
        Scopes {
            oscilloscope: Oscilloscope::default(),
            vectorscope: Vectorscope::default(),
            spectroscope: Spectroscope::new(48_000, BUFFER),
            graph,
            channels: vec![vec![0.0; n]; 2],
            left: vec![0.0; n],
            right: vec![0.0; n],
            meters: Meters::default(),
            tuned: None,
            fft,
            note_in: [vec![0.0; NOTE_FFT], vec![0.0; NOTE_FFT]],
            window,
            spectrum,
            theme: None,
            colors: None,
            last: None,
            last_written: 0,
            quiet_since: None,
        }
    }

    pub fn use_theme(&mut self, key: usize, colors: impl FnOnce() -> ScopeColors) {
        if self.theme != Some(key) {
            let c = colors();
            self.graph.palette = vec![c.left, c.right];
            self.graph.labels_color = c.label;
            self.graph.axis_color = c.axis;
            self.colors = Some(c);
            self.theme = Some(key);
        }
    }

    pub fn is_active(&self, tap: &Tap) -> bool {
        tap.written() != self.last_written || self.channels.iter().flatten().any(|&s| s != 0.0)
    }

    pub fn update(&mut self, tap: &Tap, frozen: bool, now: Instant) {
        let rate = tap.rate().max(1);
        self.graph.sampling_rate = rate;
        self.spectroscope.sampling_rate = rate;
        self.graph.pause = frozen;
        let written = tap.written();
        if written != self.last_written {
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(now);
        }
        let dt = self.last.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32().min(0.25));
        self.last = Some(now);
        if frozen {
            self.quiet_since = None;
            return;
        }
        self.last_written = written;
        let stale = written == 0 || self.quiet_since.is_some_and(|t| now.saturating_duration_since(t) >= STALE);
        if stale {
            self.left.iter_mut().chain(self.right.iter_mut()).for_each(|s| *s = 0.0);
            self.note_in.iter_mut().flatten().for_each(|s| *s = 0.0);
        } else {
            let end = match &self.tuned {
                Some((_, freq)) => {
                    let period = rate as f64 / *freq as f64;
                    written - (written as f64 % period) as usize
                }
                None => written,
            };
            tap.stereo_until(end, &mut self.left, &mut self.right);
            let [nl, nr] = &mut self.note_in;
            tap.stereo_until(written, nl, nr);
        }
        for (row, from) in self.channels.iter_mut().zip([&self.left, &self.right]) {
            row.clear();
            row.extend(from.iter().map(|&s| s as f64));
        }
        self.measure(rate, dt);
    }

    fn measure(&mut self, rate: u32, dt: f32) {
        let m = &mut self.meters;
        m.note_age += dt;
        if m.note_age >= NOTE_EVERY || m.note.is_none() {
            m.note_age = 0.0;
            m.note = dominant(&*self.fft, &self.note_in, &self.window, &mut self.spectrum, rate);
        }
    }

    pub fn key(&mut self, mode: Mode, key: KeyEvent) -> Key {
        let magnitude = match key.modifiers {
            KeyModifiers::SHIFT => 10.0,
            KeyModifiers::CONTROL => 5.0,
            KeyModifiers::ALT => 0.2,
            _ => 1.0,
        };
        let g = &mut self.graph;
        let max_samples = g.chunks * g.initial_samples * 2;
        let mut out = Key::Handled;
        match key.code {
            KeyCode::Up => update_value_f(&mut g.scale, 0.01, magnitude, 0.0..10.0),
            KeyCode::Down => update_value_f(&mut g.scale, -0.01, magnitude, 0.0..10.0),
            KeyCode::Right => update_value_i(&mut g.samples, true, 25, magnitude, 0..max_samples),
            KeyCode::Left => update_value_i(&mut g.samples, false, 25, magnitude, 0..max_samples),
            KeyCode::Char('q') => out = Key::Quit,
            KeyCode::Char('s') => g.scatter = !g.scatter,
            KeyCode::Char('r') => g.references = !g.references,
            KeyCode::Tab => out = Key::Switched(mode.switch()),
            KeyCode::BackTab => out = Key::Switched(mode.switch_reverse()),
            KeyCode::Esc => {
                g.samples = g.chunks * g.initial_samples;
                g.scale = 1.;
                self.tuned = None;
            }
            KeyCode::Char('u') if mode == Mode::Oscilloscope => {
                self.tuned = match (&self.tuned, &self.meters.note) {
                    (None, Some((name, freq, _))) => Some((name.clone(), *freq)),
                    _ => None,
                };
            }
            _ => out = Key::Ignored,
        }
        let mode_key = match mode {
            Mode::Oscilloscope => matches!(
                key.code,
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Esc | KeyCode::Char('t' | 'e' | 'p' | '=' | '-' | '+' | '_')
            ),
            Mode::Vectorscope => matches!(key.code, KeyCode::Char('g')),
            Mode::Spectroscope => matches!(key.code, KeyCode::PageUp | KeyCode::PageDown | KeyCode::Char('w' | 'l' | 'p')),
        };
        if mode_key {
            self.display(mode).handle(Event::Key(key));
            if out == Key::Ignored {
                out = Key::Handled;
            }
        }
        out
    }

    #[cfg(test)]
    pub fn channels_flat(&self) -> bool {
        self.channels.iter().flatten().all(|&s| s == 0.0)
    }

    #[cfg(test)]
    pub fn zoom(&self) -> f64 {
        self.graph.scale
    }

    pub fn wheel(&mut self, up: bool) {
        update_value_f(&mut self.graph.scale, if up { 0.01 } else { -0.01 }, 5.0, 0.0..10.0);
    }

    fn display(&mut self, mode: Mode) -> &mut dyn DisplayMode {
        match mode {
            Mode::Oscilloscope => &mut self.oscilloscope,
            Mode::Vectorscope => &mut self.vectorscope,
            Mode::Spectroscope => &mut self.spectroscope,
        }
    }

    pub fn paint(&mut self, buf: &mut Buffer, area: Rect, mode: Mode) {
        let area = area.intersection(buf.area);
        let Some(colors) = self.colors else { return };
        if area.is_empty() {
            return;
        }
        let ground = Style::default().bg(colors.ground).fg(colors.label);
        buf.set_style(area, ground);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol(" ");
            }
        }
        let graph = self.graph.clone();
        let channels = std::mem::take(&mut self.channels);
        let mut datasets = Vec::new();
        {
            let display = self.display(mode);
            if graph.references {
                datasets.append(&mut display.references(&graph));
            }
            datasets.append(&mut display.process(&graph, &channels));
        }
        self.channels = channels;

        let display = self.display(mode);
        Chart::new(datasets.iter().map(Dataset::from).collect())
            .style(ground)
            .x_axis(display.axis(&graph, Dimension::X))
            .y_axis(display.axis(&graph, Dimension::Y))
            .legend_position(None)
            .render(area, buf);
    }
}

fn dominant(
    fft: &dyn RealToComplex<f32>,
    channels: &[Vec<f32>; 2],
    window: &[f32],
    spectrum: &mut [realfft::num_complex::Complex<f32>],
    rate: u32,
) -> Option<(String, f32, f32)> {
    let mut power = vec![0.0f32; spectrum.len()];
    for ch in channels {
        let mut input: Vec<f32> = ch.iter().zip(window).map(|(s, w)| s * w).collect();
        fft.process(&mut input, spectrum).ok()?;
        power.iter_mut().zip(spectrum.iter()).for_each(|(p, c)| *p += c.norm_sqr());
    }
    let bin = rate as f32 / NOTE_FFT as f32;
    let lo = (NOTE_LO / bin).floor().max(1.0) as usize;
    let hi = ((NOTE_HI / bin).ceil() as usize).min(spectrum.len() - 2);
    let (k, &top) = power[lo..=hi].iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let k = k + lo;
    let mean = power[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
    if top <= 1e-9 || top < NOTE_PROMINENCE * mean {
        return None;
    }
    let (a, b, c) = (power[k - 1].max(1e-20).ln(), top.ln(), power[k + 1].max(1e-20).ln());
    let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
    let freq = (k as f32 + if shift.is_finite() { shift.clamp(-0.5, 0.5) } else { 0.0 }) * bin;
    let (note, cents) = Note::from_frequency(freq)?;
    Some((note.to_string(), freq, cents))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(tap: &Tap, secs: f32, f: impl Fn(f32) -> (f32, f32)) {
        let mut w = tap.writer();
        for i in 0..(secs * 48_000.0) as usize {
            let (l, r) = f(i as f32 / 48_000.0);
            w.push_stereo((l + r) * 0.5, l, r);
        }
    }

    fn run(f: impl Fn(f32) -> (f32, f32)) -> Scopes {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = Scopes::new();
        let t0 = Instant::now();
        for i in 0..40 {
            feed(&tap, 0.033, |t| f(t + i as f32 * 0.033));
            s.update(&tap, false, t0 + Duration::from_millis(33 * i as u64));
        }
        s
    }

    use std::f32::consts::TAU;

    #[test]
    fn the_dominant_note_is_measured() {
        let s = run(|t| {
            let x = 0.5012 * (TAU * 220.0 * t).sin();
            (x, x)
        });
        let (name, freq, cents) = s.meters.note.unwrap();
        assert_eq!(name, "A3");
        assert!((freq - 220.0).abs() < 1.0 && cents.abs() < 8.0, "{freq} {cents}");

        let s = run(|t| {
            let x = (TAU * 110.0 * t).sin();
            (x, -x)
        });
        assert_eq!(s.meters.note.unwrap().0, "A2");
    }

    #[test]
    fn silence_reads_no_note() {
        let s = run(|_| (0.0, 0.0));
        assert!(s.meters.note.is_none());
    }

    #[test]
    fn tuning_holds_a_note_still() {
        let tap = Tap::new();
        tap.set_rate(48_000);
        let mut s = Scopes::new();
        let t0 = Instant::now();
        let mut at = 0usize;
        let mut step = |s: &mut Scopes, i: u64| {
            let mut w = tap.writer();
            for _ in 0..1601 {
                let x = 0.5 * (TAU * 220.0 * at as f32 / 48_000.0).sin();
                w.push_stereo(x, x, x);
                at += 1;
            }
            drop(w);
            s.update(&tap, false, t0 + Duration::from_millis(33 * i));
            s.channels[0].clone()
        };
        for i in 0..10 {
            step(&mut s, i);
        }
        let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(s.key(Mode::Oscilloscope, k('u')), Key::Handled);
        assert_eq!(s.tuned.as_ref().map(|t| t.0.as_str()), Some("A3"));
        let a = step(&mut s, 10);
        let b = step(&mut s, 11);
        let off = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f64, f64::max);
        assert!(off < 0.02, "the trace moved by {off} between frames");
        s.key(Mode::Oscilloscope, k('u'));
        assert!(s.tuned.is_none());
    }

    const COLORS: ScopeColors = ScopeColors {
        ground: Color::Rgb(0, 0, 0),
        left: Color::Rgb(1, 0, 0),
        right: Color::Rgb(2, 0, 0),
        label: Color::Rgb(3, 0, 0),
        axis: Color::Rgb(4, 0, 0),
    };

    #[test]
    fn every_scope_is_the_trace_alone() {
        let area = Rect::new(0, 0, 90, 10);
        let mut s = run(|t| {
            let x = 0.4 * (TAU * 220.0 * t).sin() + 0.2 * (TAU * 1_900.0 * t).sin();
            (x, 0.5 * x + 0.3 * (TAU * 330.0 * t).sin())
        });
        s.use_theme(0, || COLORS);
        for (mode, min_trace_rows) in [(Mode::Oscilloscope, 4), (Mode::Spectroscope, 4), (Mode::Vectorscope, 3)] {
            let mut buf = Buffer::empty(area);
            s.paint(&mut buf, area, mode);
            let (mut trace_rows, mut axis_rows) = (0, 0);
            for y in 0..area.height {
                let cells: Vec<_> = (0..area.width).map(|x| &buf[(x, y)]).collect();
                let text: String = cells.iter().map(|c| c.symbol()).collect();
                assert!(
                    text.chars().all(|ch| ch == ' ' || ('\u{2800}'..='\u{28FF}').contains(&ch)),
                    "{mode:?} row {y}: {text:?}"
                );
                assert!(cells.iter().all(|c| c.bg == COLORS.ground), "{mode:?} row {y}");
                trace_rows += usize::from(cells.iter().any(|c| c.fg == COLORS.left || c.fg == COLORS.right));
                axis_rows += usize::from(cells.iter().any(|c| c.fg == COLORS.axis));
            }
            assert!(trace_rows >= min_trace_rows, "{mode:?}: {trace_rows} rows of trace");
            assert_eq!(axis_rows, usize::from(mode == Mode::Oscilloscope), "{mode:?}");
        }
    }

    #[test]
    fn scopes_move_their_slot_by_the_rows_they_dont_draw() {
        assert_eq!(Mode::Oscilloscope.slot_shift(), -3);
        assert_eq!(Mode::Spectroscope.slot_shift(), -1);
        assert_eq!(Mode::Vectorscope.slot_shift(), 1);
    }

    #[test]
    fn h_is_not_a_scope_key() {
        let mut s = Scopes::new();
        let h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        for mode in [Mode::Oscilloscope, Mode::Vectorscope, Mode::Spectroscope] {
            assert_eq!(s.key(mode, h), Key::Ignored, "{mode:?}");
        }
    }
}
