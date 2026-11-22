use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const MAX_RECENT: usize = 100;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub music_root: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub pinned: Vec<PathBuf>,
    pub volume: f32,
    pub show_hidden: bool,
    pub eq_enabled: bool,
    pub flipped: bool,
    pub analyzer: bool,
    pub visual: Visual,
    pub repeat: crate::audio::Repeat,
    pub crossfade: u32,
    pub replaygain: crate::audio::ReplayGain,
    pub shuffle_albums: bool,
    pub continue_folders: bool,
    pub folder_split: u16,
    pub playlist_split: u16,
    pub library_split: [u16; 2],
    pub visual_rows: u16,
    pub compact: bool,
    pub resume: Option<Resume>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Resume {
    pub song: PathBuf,
    pub index: usize,
    pub seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visual {
    #[default]
    Spectrum,
    Trace,
    Bokeh,
    BokehLight,
    Lines,
    Oscilloscope,
    Vectorscope,
    Spectroscope,
    Synthwave,
    SynthMoonWave,
    Galaxy,
    ClubLights,
}

impl Visual {
    const NAMES: [(Visual, &'static str, &'static [&'static str]); 12] = [
        (Visual::Spectrum, "spectrum", &[]),
        (Visual::Trace, "trace", &[]),
        (Visual::Bokeh, "bokeh", &[]),
        (Visual::BokehLight, "bokeh_light", &["bokeh light"]),
        (Visual::Lines, "lines", &["squares"]),
        (Visual::Oscilloscope, "oscilloscope", &[]),
        (Visual::Vectorscope, "vectorscope", &[]),
        (Visual::Spectroscope, "spectroscope", &[]),
        (Visual::Synthwave, "synthwave", &["synthwave_urchin"]),
        (Visual::SynthMoonWave, "synthmoonwave", &["synth_moon_wave"]),
        (Visual::Galaxy, "galaxy", &["galaxy_urchin"]),
        (Visual::ClubLights, "club_lights", &["club lights"]),
    ];

    pub fn name(self) -> &'static str {
        Self::NAMES.iter().find(|(v, ..)| *v == self).map_or("spectrum", |(_, name, _)| name)
    }

    pub fn from_name(name: &str) -> Visual {
        Self::NAMES
            .iter()
            .find(|(_, saved, old)| saved.eq_ignore_ascii_case(name) || old.iter().any(|o| o.eq_ignore_ascii_case(name)))
            .map_or(Visual::Spectrum, |(v, ..)| *v)
    }

    pub fn slot_shift(self) -> i16 {
        self.scope_mode().map_or(0, crate::scopes::Mode::slot_shift)
    }

    pub fn scope_mode(self) -> Option<crate::scopes::Mode> {
        use crate::scopes::Mode;
        match self {
            Visual::Oscilloscope => Some(Mode::Oscilloscope),
            Visual::Vectorscope => Some(Mode::Vectorscope),
            Visual::Spectroscope => Some(Mode::Spectroscope),
            _ => None,
        }
    }
}

impl From<crate::scopes::Mode> for Visual {
    fn from(mode: crate::scopes::Mode) -> Visual {
        use crate::scopes::Mode;
        match mode {
            Mode::Oscilloscope => Visual::Oscilloscope,
            Mode::Vectorscope => Visual::Vectorscope,
            Mode::Spectroscope => Visual::Spectroscope,
        }
    }
}

impl Serialize for Visual {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Visual {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Visual, D::Error> {
        let name = std::borrow::Cow::<str>::deserialize(d)?;
        Ok(Visual::from_name(&name))
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: crate::theme::DEFAULT_THEME.to_string(),
            music_root: None,
            recent: Vec::new(),
            pinned: Vec::new(),
            volume: 0.8,
            show_hidden: false,
            eq_enabled: true,
            flipped: false,
            analyzer: true,
            visual: Visual::Spectrum,
            repeat: crate::audio::Repeat::Off,
            crossfade: 0,
            replaygain: crate::audio::ReplayGain::Off,
            shuffle_albums: false,
            continue_folders: false,
            folder_split: 500,
            playlist_split: 500,
            library_split: [333, 667],
            visual_rows: 0,
            compact: false,
            resume: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let base = xdg.or_else(dirs::config_dir).unwrap_or_else(crate::fsx::home);
    let dir = base.join("urchin");
    let old = base.join("batplayer");
    if !dir.exists() && old.is_dir() {
        let _ = fs::rename(&old, &dir);
    }
    dir
}

fn path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    let Ok(text) = fs::read_to_string(path) else { return T::default() };
    if let Ok(v) = serde_json::from_str(&text) { v } else {
        let mut bad = path.as_os_str().to_owned();
        bad.push(".bad");
        let _ = fs::rename(path, bad);
        T::default()
    }
}

impl Settings {
    pub fn load() -> Settings {
        read_json(&path())
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = crate::fsx::save_if_different(&path(), json.as_bytes());
        }
    }

    pub fn touch_recent(&mut self, dir: &PathBuf) {
        self.recent.retain(|p| p != dir);
        self.recent.insert(0, dir.clone());
        self.recent.truncate(MAX_RECENT);
    }
}
