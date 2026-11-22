use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::bokeh::Bokeh;
use crate::club_lights::ClubLights;
use crate::galaxy::Galaxy;
use crate::lines::Lines;
use crate::scopes::Scopes;
use crate::settings::Visual;
use crate::spectrum::{Analyzer, Tap};
use crate::synthwave::{self, Synthwave};
use crate::theme::{self, THEMES};
use crate::viz::Trace;

pub trait Picture {
    fn use_theme(&mut self, _theme: usize) {}
    fn is_active(&self) -> bool;
    fn hold(&mut self, now: Instant);
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant);
    fn paint(&mut self, buf: &mut Buffer, area: Rect);
}

impl Picture for Trace {
    fn use_theme(&mut self, theme: usize) {
        Trace::use_theme(self, theme, || THEMES[theme].trace_colors());
    }
    fn is_active(&self) -> bool {
        Trace::is_active(self)
    }
    fn hold(&mut self, now: Instant) {
        Trace::hold(self, now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        Trace::update(self, tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        Trace::paint(self, buf, area);
    }
}

struct BokehOn {
    bokeh: Bokeh,
    light: bool,
}

impl Picture for BokehOn {
    fn use_theme(&mut self, theme: usize) {
        let light = self.light;
        self.bokeh.use_theme(theme, || if light { THEMES[theme].bokeh_light_colors() } else { THEMES[theme].bokeh_colors() });
    }
    fn is_active(&self) -> bool {
        self.bokeh.is_active()
    }
    fn hold(&mut self, now: Instant) {
        self.bokeh.hold(now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        self.bokeh.update(tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        self.bokeh.paint(buf, area);
    }
}

impl Picture for Lines {
    fn use_theme(&mut self, theme: usize) {
        Lines::use_theme(self, theme, || THEMES[theme].lines_colors());
    }
    fn is_active(&self) -> bool {
        Lines::is_active(self)
    }
    fn hold(&mut self, now: Instant) {
        Lines::hold(self, now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, _seed: u64, now: Instant) {
        Lines::update(self, tap, columns, rows, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        Lines::paint(self, buf, area);
    }
}

impl Picture for Synthwave {
    fn use_theme(&mut self, _theme: usize) {
        self.use_colors(synthwave::OWN_COLORS, theme::synthwave_colors);
    }
    fn is_active(&self) -> bool {
        Synthwave::is_active(self)
    }
    fn hold(&mut self, now: Instant) {
        Synthwave::hold(self, now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        Synthwave::update(self, tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        Synthwave::paint(self, buf, area);
    }
}

struct SynthMoonWave(Synthwave);

impl Picture for SynthMoonWave {
    fn use_theme(&mut self, _theme: usize) {
        self.0.use_colors(synthwave::MOON_COLORS, theme::synthmoonwave_colors);
    }
    fn is_active(&self) -> bool {
        self.0.is_active()
    }
    fn hold(&mut self, now: Instant) {
        self.0.hold(now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        self.0.update(tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        self.0.paint(buf, area);
    }
}

impl Picture for Galaxy {
    fn is_active(&self) -> bool {
        Galaxy::is_active(self)
    }
    fn hold(&mut self, now: Instant) {
        Galaxy::hold(self, now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        Galaxy::update(self, tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        Galaxy::paint(self, buf, area);
    }
}

impl Picture for ClubLights {
    fn is_active(&self) -> bool {
        ClubLights::is_active(self)
    }
    fn hold(&mut self, now: Instant) {
        ClubLights::hold(self, now);
    }
    fn update(&mut self, tap: &Tap, columns: usize, rows: u16, seed: u64, now: Instant) {
        ClubLights::update(self, tap, columns, rows, seed, now);
    }
    fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        ClubLights::paint(self, buf, area);
    }
}

enum Slot {
    Spectrum(Box<Analyzer>),
    Scopes(Box<Scopes>),
    Picture(Visual, Box<dyn Picture>),
}

impl Slot {
    fn holds(&self, visual: Visual) -> bool {
        match self {
            Slot::Spectrum(_) => visual == Visual::Spectrum,
            Slot::Scopes(_) => visual.scope_mode().is_some(),
            Slot::Picture(v, _) => *v == visual,
        }
    }

    fn build(visual: Visual) -> Slot {
        let picture: Box<dyn Picture> = match visual {
            Visual::Spectrum => return Slot::Spectrum(Box::default()),
            Visual::Oscilloscope | Visual::Vectorscope | Visual::Spectroscope => return Slot::Scopes(Box::default()),
            Visual::Trace => Box::new(Trace::new()),
            Visual::Bokeh => Box::new(BokehOn { bokeh: Bokeh::new(), light: false }),
            Visual::BokehLight => Box::new(BokehOn { bokeh: Bokeh::new(), light: true }),
            Visual::Lines => Box::new(Lines::new()),
            Visual::Synthwave => Box::new(Synthwave::new()),
            Visual::SynthMoonWave => Box::new(SynthMoonWave(Synthwave::moonlit())),
            Visual::Galaxy => Box::new(Galaxy::new()),
            Visual::ClubLights => Box::new(ClubLights::new()),
        };
        Slot::Picture(visual, picture)
    }
}

#[derive(Default)]
pub struct Visuals {
    slot: Option<Slot>,
}

impl Visuals {
    fn slot(&mut self, visual: Visual) -> &mut Slot {
        if !self.slot.as_ref().is_some_and(|s| s.holds(visual)) {
            self.slot = Some(Slot::build(visual));
        }
        self.slot.as_mut().expect("just filled")
    }

    pub fn spectrum(&mut self) -> &mut Analyzer {
        match self.slot(Visual::Spectrum) {
            Slot::Spectrum(a) => a,
            _ => unreachable!("built for Spectrum"),
        }
    }

    pub fn scopes(&mut self) -> &mut Scopes {
        match self.slot(Visual::Oscilloscope) {
            Slot::Scopes(s) => s,
            _ => unreachable!("built for a scope"),
        }
    }

    pub fn scopes_if_shown(&mut self) -> Option<&mut Scopes> {
        match self.slot.as_mut()? {
            Slot::Scopes(s) => Some(s),
            _ => None,
        }
    }

    pub fn picture(&mut self, visual: Visual) -> &mut dyn Picture {
        match self.slot(visual) {
            Slot::Picture(_, p) => p.as_mut(),
            _ => unreachable!("{visual:?} is not a picture visualizer"),
        }
    }

    pub fn is_active(&self, visual: Visual, tap: &Tap) -> bool {
        match &self.slot {
            Some(slot) if slot.holds(visual) => match slot {
                Slot::Spectrum(a) => a.is_active(),
                Slot::Scopes(s) => s.is_active(tap),
                Slot::Picture(_, p) => p.is_active(),
            },
            _ => false,
        }
    }
}
