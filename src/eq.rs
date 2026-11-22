use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const BANDS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const LABELS: [&str; 10] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
pub const MAX_DB: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub pre: f32,
    pub gains: [f32; 10],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<u8>,
}

impl Curve {
    pub fn is_flat(&self) -> bool {
        self.pre.abs() < 0.05 && self.gains.iter().all(|g| g.abs() < 0.05)
    }

    pub fn get(&self, band: usize) -> f32 {
        if band == 0 { self.pre } else { self.gains[band - 1] }
    }

    pub fn set(&mut self, band: usize, db: f32) {
        let db = (db * 2.0).round() / 2.0;
        let db = db.clamp(-MAX_DB, MAX_DB);
        if band == 0 { self.pre = db } else { self.gains[band - 1] = db }
    }
}

const VLC_FREQS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];
#[rustfmt::skip]
const VLC_PRESETS: [(&str, f32, [f32; 10]); 18] = [
    ("Flat",                 12.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Classical",            12.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -7.2, -7.2, -7.2, -9.6]),
    ("Club",                  6.0, [0.0, 0.0, 8.0, 5.6, 5.6, 5.6, 3.2, 0.0, 0.0, 0.0]),
    ("Dance",                 5.0, [9.6, 7.2, 2.4, 0.0, 0.0, -5.6, -7.2, -7.2, 0.0, 0.0]),
    ("Full bass",             5.0, [-8.0, 9.6, 9.6, 5.6, 1.6, -4.0, -8.0, -10.4, -11.2, -11.2]),
    ("Full bass and treble",  4.0, [7.2, 5.6, 0.0, -7.2, -4.8, 1.6, 8.0, 11.2, 12.0, 12.0]),
    ("Full treble",           3.0, [-9.6, -9.6, -9.6, -4.0, 2.4, 11.2, 16.0, 16.0, 16.0, 16.8]),
    ("Headphones",            4.0, [4.8, 11.2, 5.6, -3.2, -2.4, 1.6, 4.8, 9.6, 12.8, 14.4]),
    ("Large hall",            5.0, [10.4, 10.4, 5.6, 5.6, 0.0, -4.8, -4.8, -4.8, 0.0, 0.0]),
    ("Live",                  7.0, [-4.8, 0.0, 4.0, 5.6, 5.6, 5.6, 4.0, 2.4, 2.4, 2.4]),
    ("Party",                 6.0, [7.2, 7.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7.2, 7.2]),
    ("Pop",                   6.0, [-1.6, 4.8, 7.2, 8.0, 5.6, 0.0, -2.4, -2.4, -1.6, -1.6]),
    ("Reggae",                8.0, [0.0, 0.0, 0.0, -5.6, 0.0, 6.4, 6.4, 0.0, 0.0, 0.0]),
    ("Rock",                  5.0, [8.0, 4.8, -5.6, -8.0, -3.2, 4.0, 8.8, 11.2, 11.2, 11.2]),
    ("Ska",                   6.0, [-2.4, -4.8, -4.0, 0.0, 4.0, 5.6, 8.8, 9.6, 11.2, 9.6]),
    ("Soft",                  5.0, [4.8, 1.6, 0.0, -2.4, 0.0, 4.0, 8.0, 9.6, 11.2, 12.0]),
    ("Soft rock",             7.0, [4.0, 4.0, 2.4, 0.0, -4.0, -5.6, -3.2, 0.0, 2.4, 8.8]),
    ("Techno",                5.0, [8.0, 5.6, 0.0, -5.6, -4.8, 0.0, 8.0, 9.6, 9.6, 8.8]),
];

#[rustfmt::skip]
const RAP_PRESETS: [(&str, [f32; 10]); 15] = [
    ("Alternative",   [2.0, 1.0, 0.0, -1.0, 0.0, 2.0, 4.0, 6.0, 4.0, 3.0]),
    ("Bass booster",  [15.0, 12.0, 10.0, 5.0, 0.0, -5.0, -8.0, -10.0, -12.0, -14.0]),
    ("Bass reducer",  [-10.0, -8.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Deep",          [10.0, 9.0, 7.0, 3.0, 0.0, -3.0, -2.0, 0.0, 1.0, 2.0]),
    ("Drum & bass",   [14.0, 12.0, 6.0, 2.0, -2.0, 0.0, 4.0, 8.0, 10.0, 8.0]),
    ("Electronic",    [10.0, 8.0, 5.0, 3.0, 1.0, 3.0, 6.0, 9.0, 12.0, 10.0]),
    ("Hip hop",       [8.0, 7.0, 5.0, 0.0, -2.0, -2.0, -2.0, 0.0, 2.0, 3.0]),
    ("Indie",         [3.0, 2.0, 1.0, 0.0, 1.0, 3.0, 5.0, 7.0, 5.0, 4.0]),
    ("Jazz",          [4.0, 3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
    ("Loudness",      [12.0, 10.0, 6.0, 3.0, 0.0, 2.0, 5.0, 8.0, 10.0, 12.0]),
    ("Metal",         [-3.0, -1.0, 3.0, 6.0, 8.0, 10.0, 12.0, 14.0, 10.0, 7.0]),
    ("Piano",         [2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 3.0, 4.0, 5.0, 4.0]),
    ("R&B",           [6.0, 6.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0, 3.0, 4.0]),
    ("Speech",        [-3.0, -2.0, 0.0, 2.0, 4.0, 6.0, 5.0, 3.0, 0.0, -2.0]),
    ("Vocal",         [-2.0, -1.0, 1.0, 3.0, 5.0, 7.0, 6.0, 4.0, 2.0, 0.0]),
];

pub const PRESET_COUNT: usize = VLC_PRESETS.len() + RAP_PRESETS.len();

pub const FIRST_MINE: u8 = 64;

pub const NAME_MAX: usize = 40;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MyPreset {
    pub id: u8,
    pub name: String,
    pub pre: f32,
    pub gains: [f32; 10],
}

impl MyPreset {
    fn curve(&self) -> Curve {
        Curve { pre: self.pre, gains: self.gains, based_on: Some(self.id) }
    }
}

fn same_values(a: &Curve, b: &Curve) -> bool {
    (a.pre - b.pre).abs() < 0.05 && a.gains.iter().zip(b.gains.iter()).all(|(x, y)| (x - y).abs() < 0.05)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    New(u8),
    Updated(u8),
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    BuiltIn(String),
    Taken(String),
    Full,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            NameError::Empty => write!(f, "type a name for the preset"),
            NameError::TooLong => write!(f, "that name is too long (up to {NAME_MAX} characters)"),
            NameError::BuiltIn(n) => write!(f, "{n} is a built-in preset; pick another name"),
            NameError::Taken(n) => write!(f, "you already have a preset called {n}"),
            NameError::Full => write!(f, "there's no room for more presets; delete one first"),
        }
    }
}

pub fn preset_name(i: usize) -> &'static str {
    if i < VLC_PRESETS.len() {
        VLC_PRESETS[i].0
    } else {
        RAP_PRESETS.get(i - VLC_PRESETS.len()).map_or("Custom", |p| p.0)
    }
}

pub fn preset_order() -> Vec<usize> {
    let mut ids: Vec<usize> = (0..PRESET_COUNT).collect();
    ids.sort_by_key(|&i| preset_name(i).to_lowercase());
    ids
}

pub fn preset_curve(i: usize) -> Curve {
    if i >= VLC_PRESETS.len() {
        let (_, gains) = RAP_PRESETS[i - VLC_PRESETS.len()];
        return Curve { pre: 0.0, gains, based_on: Some(i as u8) };
    }
    let (_, _, amps) = VLC_PRESETS[i];
    let mut gains = [0.0; 10];
    for (g, &f) in gains.iter_mut().zip(BANDS.iter()) {
        *g = interpolate(&amps, f);
    }
    Curve { pre: 0.0, gains, based_on: Some(i as u8) }
}

fn interpolate(amps: &[f32; 10], f: f32) -> f32 {
    if f <= VLC_FREQS[0] {
        return amps[0];
    }
    for k in 0..9 {
        let (f0, f1) = (VLC_FREQS[k], VLC_FREQS[k + 1]);
        if f <= f1 {
            let t = (f.ln() - f0.ln()) / (f1.ln() - f0.ln());
            let v = amps[k] + (amps[k + 1] - amps[k]) * t;
            return (v * 10.0).round() / 10.0;
        }
    }
    amps[9]
}

impl Curve {
    #[cfg(test)]
    pub fn label(&self) -> String {
        if let Some(i) = self.based_on.map(|i| i as usize).filter(|&i| i < PRESET_COUNT) {
            let p = preset_curve(i);
            let same = (p.pre - self.pre).abs() < 0.05 && p.gains.iter().zip(self.gains.iter()).all(|(a, b)| (a - b).abs() < 0.05);
            return if same { preset_name(i).to_string() } else { format!("{} (customized)", preset_name(i)) };
        }
        if self.is_flat() { "Flat".into() } else { "Custom".into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Own,
    Folder(u8),
    None,
}

#[derive(Default)]
pub struct Store {
    map: HashMap<String, Curve>,
    folders: HashMap<String, u8>,
    mine: Vec<MyPreset>,
    prints: HashMap<String, String>,
    folder_prints: HashMap<String, Vec<String>>,
    old_prints: HashMap<String, u32>,
    old_folder_prints: HashMap<String, u32>,
    dirty: bool,
}

#[derive(Serialize, Deserialize)]
struct SongEntry {
    #[serde(flatten)]
    curve: Curve,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    print: Option<String>,
    #[serde(default = "first_version", skip_serializing_if = "Option::is_none")]
    print_version: Option<u32>,
}

fn first_version() -> Option<u32> {
    Some(1)
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum FolderEntry {
    Full {
        preset: u8,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prints: Vec<String>,
        #[serde(default = "first_version", skip_serializing_if = "Option::is_none")]
        prints_version: Option<u32>,
    },
    Plain(u8),
}

pub const FOLDER_PRINTS: usize = 3;

fn gone(old: &str) -> bool {
    let old = Path::new(old);
    if old.exists() {
        return false;
    }
    let Some(above) = old.ancestors().skip(1).find(|a| a.exists()) else { return false };
    let mounts = ["/", "/Volumes", "/media", "/mnt", "/run/media"];
    if above.parent().is_none() || mounts.iter().any(|m| above == Path::new(m)) {
        return false;
    }
    std::fs::read_dir(above).is_ok_and(|mut d| d.next().is_some())
}

fn store_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("equalizer.json")
}

fn folders_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("equalizer_folders.json")
}

fn presets_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("equalizer_presets.json")
}

impl Store {
    pub fn load() -> Store {
        let songs: HashMap<String, SongEntry> = crate::settings::read_json(&store_path());
        let folders: HashMap<String, FolderEntry> = crate::settings::read_json(&folders_path());
        let mut store = Store { mine: crate::settings::read_json(&presets_path()), ..Store::default() };
        for (path, e) in songs {
            if let Some(p) = e.print {
                let made_by = e.print_version.unwrap_or(1);
                if made_by != crate::fingerprint::VERSION {
                    store.old_prints.insert(path.clone(), made_by);
                }
                store.prints.insert(path.clone(), p);
            }
            store.map.insert(path, e.curve);
        }
        for (path, e) in folders {
            match e {
                FolderEntry::Plain(id) => {
                    store.folders.insert(path, id);
                }
                FolderEntry::Full { preset, prints, prints_version } => {
                    if !prints.is_empty() {
                        let made_by = prints_version.unwrap_or(1);
                        if made_by != crate::fingerprint::VERSION {
                            store.old_folder_prints.insert(path.clone(), made_by);
                        }
                        store.folder_prints.insert(path.clone(), prints);
                    }
                    store.folders.insert(path, preset);
                }
            }
        }
        store
    }

    pub fn wants_print(&self, song: &Path) -> bool {
        let k = key(song);
        self.map.contains_key(&*k) && !self.prints.contains_key(&*k)
    }

    pub fn set_print(&mut self, song: &Path, print: String) {
        let k = key(song);
        let old = self.old_prints.remove(&*k).is_some();
        if self.map.contains_key(&*k) && (old || self.prints.get(&*k) != Some(&print)) {
            self.prints.insert(k.into_owned(), print);
            self.dirty = true;
        }
    }

    pub fn stale_prints(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let here = |k: &&String| Path::new(k.as_str()).exists();
        let mut songs: Vec<PathBuf> =
            self.old_prints.keys().filter(|k| self.map.contains_key(*k)).filter(here).map(PathBuf::from).collect();
        let mut folders: Vec<PathBuf> =
            self.old_folder_prints.keys().filter(|k| self.folders.contains_key(*k)).filter(here).map(PathBuf::from).collect();
        songs.sort();
        folders.sort();
        (songs, folders)
    }

    pub fn refresh_print(&mut self, song: &Path, print: Option<String>) {
        match print {
            Some(p) => self.set_print(song, p),
            None => {
                self.old_prints.remove(&*key(song));
            }
        }
    }

    pub fn refresh_folder_prints(&mut self, dir: &Path, prints: Vec<String>) {
        if prints.is_empty() {
            self.old_folder_prints.remove(&*key(dir));
        } else {
            self.set_folder_prints(dir, prints);
        }
    }

    pub fn has_lost_songs(&self) -> bool {
        self.prints.keys().any(|k| self.map.contains_key(k) && gone(k))
    }

    pub fn relink_song(&mut self, song: &Path, print: &str) -> bool {
        let k = key(song).into_owned();
        if self.map.contains_key(&k) {
            return false;
        }
        let mut lost: Vec<&String> = self
            .prints
            .iter()
            .filter(|(old, p)| p.as_str() == print && self.map.contains_key(*old) && gone(old))
            .map(|(old, _)| old)
            .collect();
        lost.sort();
        let Some(old) = lost.first().map(|o| (*o).clone()) else { return false };
        let curve = self.map.remove(&old).unwrap();
        self.prints.remove(&old);
        self.old_prints.remove(&old);
        self.map.insert(k.clone(), curve);
        self.prints.insert(k, print.to_string());
        self.dirty = true;
        true
    }

    pub fn wants_folder_prints(&self, dir: &Path) -> bool {
        let k = key(dir);
        self.folders.contains_key(&*k) && !self.folder_prints.contains_key(&*k)
    }

    pub fn set_folder_prints(&mut self, dir: &Path, prints: Vec<String>) {
        let k = key(dir);
        let old = self.old_folder_prints.remove(&*k).is_some();
        if self.folders.contains_key(&*k) && !prints.is_empty() && (old || self.folder_prints.get(&*k) != Some(&prints)) {
            self.folder_prints.insert(k.into_owned(), prints);
            self.dirty = true;
        }
    }

    pub fn has_lost_folders(&self) -> bool {
        self.folder_prints.keys().any(|k| self.folders.contains_key(k) && gone(k))
    }

    pub fn relink_folder(&mut self, dir: &Path, prints: &[String]) -> Option<u8> {
        let k = key(dir).into_owned();
        if self.folders.contains_key(&k) || prints.is_empty() {
            return None;
        }
        let mut best: Option<(usize, String)> = None;
        for (old, theirs) in &self.folder_prints {
            if !self.folders.contains_key(old) || !gone(old) {
                continue;
            }
            let common = theirs.iter().filter(|p| prints.contains(p)).count();
            let need = theirs.len().min(prints.len()).min(2);
            if common >= need && common > 0 && best.as_ref().is_none_or(|(c, o)| common > *c || (common == *c && old < o)) {
                best = Some((common, old.clone()));
            }
        }
        let (_, old) = best?;
        let preset = self.folders.remove(&old)?;
        self.folder_prints.remove(&old);
        self.old_folder_prints.remove(&old);
        self.folders.insert(k.clone(), preset);
        self.folder_prints.insert(k, prints.to_vec());
        self.dirty = true;
        Some(preset)
    }

    pub fn preset_curve(&self, id: u8) -> Option<Curve> {
        if (id as usize) < PRESET_COUNT {
            Some(preset_curve(id as usize))
        } else {
            self.mine.iter().find(|p| p.id == id).map(MyPreset::curve)
        }
    }

    pub fn preset_name(&self, id: u8) -> String {
        if (id as usize) < PRESET_COUNT {
            preset_name(id as usize).to_string()
        } else {
            self.mine.iter().find(|p| p.id == id).map_or_else(|| "Custom".to_string(), |p| p.name.clone())
        }
    }

    pub fn is_mine(&self, id: u8) -> bool {
        self.mine.iter().any(|p| p.id == id)
    }

    pub fn my_presets(&self) -> Vec<u8> {
        let mut v: Vec<&MyPreset> = self.mine.iter().collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        v.into_iter().map(|p| p.id).collect()
    }

    pub fn preset_order(&self) -> Vec<u8> {
        let mut ids = self.my_presets();
        ids.extend(preset_order().into_iter().map(|i| i as u8));
        ids
    }

    pub fn label(&self, c: &Curve) -> String {
        if let Some((id, p)) = c.based_on.and_then(|id| self.preset_curve(id).map(|p| (id, p))) {
            let name = self.preset_name(id);
            return if same_values(&p, c) { name } else { format!("{name} (customized)") };
        }
        if c.is_flat() { "Flat".into() } else { "Custom".into() }
    }

    pub fn find_mine(&self, name: &str) -> Option<u8> {
        let name = name.trim().to_lowercase();
        self.mine.iter().find(|p| p.name.to_lowercase() == name).map(|p| p.id)
    }

    fn check_name(&self, name: &str) -> Result<String, NameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if name.chars().count() > NAME_MAX {
            return Err(NameError::TooLong);
        }
        if let Some(i) = (0..PRESET_COUNT).find(|&i| preset_name(i).eq_ignore_ascii_case(name)) {
            return Err(NameError::BuiltIn(preset_name(i).to_string()));
        }
        Ok(name.to_string())
    }

    pub fn save_mine(&mut self, name: &str, curve: &Curve) -> Result<Saved, NameError> {
        let name = self.check_name(name)?;
        if let Some(id) = self.find_mine(&name) {
            let old = self.preset_curve(id).unwrap();
            let new = Curve { pre: curve.pre, gains: curve.gains, based_on: Some(id) };
            for c in self.map.values_mut().filter(|c| c.based_on == Some(id) && same_values(c, &old)) {
                *c = new;
            }
            let p = self.mine.iter_mut().find(|p| p.id == id).unwrap();
            (p.name, p.pre, p.gains) = (name, curve.pre, curve.gains);
            self.dirty = true;
            return Ok(Saved::Updated(id));
        }
        let id = (FIRST_MINE..=u8::MAX).find(|id| !self.is_mine(*id)).ok_or(NameError::Full)?;
        self.mine.push(MyPreset { id, name, pre: curve.pre, gains: curve.gains });
        self.dirty = true;
        Ok(Saved::New(id))
    }

    pub fn rename_mine(&mut self, id: u8, name: &str) -> Result<(), NameError> {
        let name = self.check_name(name)?;
        if let Some(other) = self.find_mine(&name).filter(|&o| o != id) {
            return Err(NameError::Taken(self.preset_name(other)));
        }
        if let Some(p) = self.mine.iter_mut().find(|p| p.id == id) {
            p.name = name;
            self.dirty = true;
        }
        Ok(())
    }

    pub fn delete_mine(&mut self, id: u8) -> usize {
        let Some(old) = self.preset_curve(id).filter(|_| self.is_mine(id)) else { return 0 };
        self.mine.retain(|p| p.id != id);
        let before = self.folders.len();
        self.folders.retain(|_, f| *f != id);
        let folders = before - self.folders.len();
        self.map.retain(|_, c| !(c.based_on == Some(id) && same_values(c, &old)));
        for c in self.map.values_mut().filter(|c| c.based_on == Some(id)) {
            c.based_on = None;
        }
        self.dirty = true;
        folders
    }

    pub fn get(&self, song: &Path) -> Curve {
        match self.source(song) {
            Source::Own => self.map[&*key(song)],
            Source::Folder(id) => self.preset_curve(id).unwrap_or_default(),
            Source::None => Curve::default(),
        }
    }

    pub fn source(&self, song: &Path) -> Source {
        if self.map.contains_key(&*key(song)) {
            Source::Own
        } else if let Some(id) = self.folder_preset_for(song) {
            Source::Folder(id)
        } else {
            Source::None
        }
    }

    pub fn has(&self, song: &Path) -> bool {
        self.map.contains_key(&*key(song))
    }

    pub fn folder_preset_for(&self, song: &Path) -> Option<u8> {
        let dir = song.parent()?;
        self.folders.get(&*key(dir)).copied().filter(|&id| self.preset_curve(id).is_some())
    }

    pub fn set_folder_preset(&mut self, dir: &Path, preset: Option<u8>) {
        if let Some(id) = preset.filter(|&id| self.preset_curve(id).is_some_and(|c| !c.is_flat())) { self.folders.insert(key(dir).into_owned(), id) } else {
            self.folder_prints.remove(&*key(dir));
            self.old_folder_prints.remove(&*key(dir));
            self.folders.remove(&*key(dir))
        };
        self.dirty = true;
    }

    pub fn set(&mut self, song: &Path, curve: Curve) {
        if curve.is_flat() && curve.based_on.is_none() {
            self.map.remove(&*key(song));
            self.prints.remove(&*key(song));
            self.old_prints.remove(&*key(song));
        } else {
            self.map.insert(key(song).into_owned(), curve);
        }
        self.dirty = true;
    }

    pub fn save_if_changed(&mut self) {
        if !self.dirty {
            return;
        }
        fn write(path: std::path::PathBuf, json: serde_json::Result<String>) -> bool {
            let Ok(json) = json else { return false };
            crate::fsx::save_if_different(&path, json.as_bytes()).is_ok()
        }
        let songs: std::collections::BTreeMap<&String, SongEntry> =
            self.map
                .iter()
                .map(|(k, c)| {
                    let print = self.prints.get(k).cloned();
                    let print_version = print.as_ref().map(|_| self.old_prints.get(k).copied().unwrap_or(crate::fingerprint::VERSION));
                    (k, SongEntry { curve: *c, print, print_version })
                })
                .collect();
        let folders: std::collections::BTreeMap<&String, FolderEntry> = self
            .folders
            .iter()
            .map(|(k, &preset)| {
                let prints = self.folder_prints.get(k).cloned().unwrap_or_default();
                let prints_version = (!prints.is_empty())
                    .then(|| self.old_folder_prints.get(k).copied().unwrap_or(crate::fingerprint::VERSION));
                (k, FolderEntry::Full { preset, prints, prints_version })
            })
            .collect();
        let songs = write(store_path(), serde_json::to_string(&songs));
        let folders = write(folders_path(), serde_json::to_string(&folders));
        let presets = (self.mine.is_empty() && !presets_path().exists()) || write(presets_path(), serde_json::to_string_pretty(&self.mine));
        if songs && folders && presets {
            self.dirty = false;
        }
    }
}

fn key(song: &Path) -> Cow<'_, str> {
    song.to_string_lossy()
}

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coef {
    fn peaking(freq: f32, gain_db: f32, rate: f32) -> Coef {
        let q = std::f32::consts::SQRT_2;
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq.min(rate * 0.45) / rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Coef {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    fn response_db(&self, w: f32) -> f32 {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (nr, ni) = (self.b0 + self.b1 * c1 + self.b2 * c2, -(self.b1 * s1 + self.b2 * s2));
        let (dr, di) = (1.0 + self.a1 * c1 + self.a2 * c2, -(self.a1 * s1 + self.a2 * s2));
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
    }

    #[inline]
    fn run(&self, x: f32, z: &mut [f32; 2]) -> f32 {
        let y = self.b0 * x + z[0];
        z[0] = self.b1 * x - self.a1 * y + z[1];
        z[1] = self.b2 * x - self.a2 * y;
        y
    }
}

#[derive(Clone, Copy)]
pub struct Filter {
    bands: [Coef; 10],
    active: [bool; 10],
    gain: f32,
    bypass: bool,
}

impl Filter {
    pub fn flat() -> Filter {
        Filter { bands: [Coef::default(); 10], active: [false; 10], gain: 1.0, bypass: true }
    }

    pub fn compile(c: &Curve, rate: u32) -> Filter {
        let rate = rate.max(8_000) as f32;
        let mut f = Filter::flat();
        f.bypass = c.is_flat();
        for (i, (&gain, &freq)) in c.gains.iter().zip(BANDS.iter()).enumerate() {
            f.active[i] = gain.abs() >= 0.05;
            if f.active[i] {
                f.bands[i] = Coef::peaking(freq, gain, rate);
            }
        }
        f.gain = 10f32.powf((c.pre - f.peak_db(rate)) / 20.0);
        f
    }

    fn peak_db(&self, rate: f32) -> f32 {
        let mut peak = 0.0f32;
        let top = 20_000f32.min(rate * 0.49);
        let grid = (0..240).map(|k| 20.0 * (top / 20.0).powf(k as f32 / 239.0));
        let centers = BANDS.iter().map(|&f| f.min(rate * 0.45));
        for f in grid.chain(centers) {
            let w = 2.0 * std::f32::consts::PI * f / rate;
            let mut db = 0.0;
            for i in 0..10 {
                if self.active[i] {
                    db += self.bands[i].response_db(w);
                }
            }
            peak = peak.max(db);
        }
        peak
    }
}

pub struct Processor {
    #[cfg_attr(not(test), allow(dead_code))]
    rate: f32,
    filter: Filter,
    z: [[[f32; 2]; 2]; 10],
}

impl Processor {
    pub fn new(rate: u32) -> Processor {
        Processor { rate: rate as f32, filter: Filter::flat(), z: [[[0.0; 2]; 2]; 10] }
    }

    pub fn set_filter(&mut self, f: &Filter) {
        self.filter = *f;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_curve(&mut self, c: &Curve) {
        self.filter = Filter::compile(c, self.rate as u32);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn peak_db(&self) -> f32 {
        self.filter.peak_db(self.rate)
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let f = &self.filter;
        if f.bypass {
            return (l, r);
        }
        let (mut l, mut r) = (l * f.gain, r * f.gain);
        for i in 0..10 {
            if f.active[i] {
                l = f.bands[i].run(l, &mut self.z[i][0]);
                r = f.bands[i].run(r, &mut self.z[i][1]);
            }
        }
        (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
    }
}
