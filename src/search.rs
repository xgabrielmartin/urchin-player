use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use unicode_normalization::UnicodeNormalization;
use unicode_width::UnicodeWidthStr;

use crate::fsx;
use crate::library::Index;

const SEP: &str = " › ";
const CUT: &str = "… › ";
const APOSTROPHES: [char; 4] = ['\'', '\u{2019}', '\u{2018}', '\u{2bc}'];

pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .nfc()
        .flat_map(char::to_lowercase)
        .filter(|c| !APOSTROPHES.contains(c))
        .collect()
}

pub fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(fold).filter(|w| w.chars().any(char::is_alphanumeric)).collect()
}

struct Tagged {
    title: String,
    trail: Vec<String>,
}

pub struct Song {
    pub path: PathBuf,
    key: String,
    tagged: Option<Tagged>,
}

impl Song {
    fn new(root: &Path, path: PathBuf) -> Song {
        let below = path.strip_prefix(root).unwrap_or(&path);
        let mut key = String::new();
        for folder in below.parent().into_iter().flat_map(Path::components) {
            key.push_str(&folder.as_os_str().to_string_lossy());
            key.push('/');
        }
        key.push_str(&fsx::title_cow(&path));
        Song { key: fold(&key), path, tagged: None }
    }

    pub fn title(&self) -> Cow<'_, str> {
        match &self.tagged {
            Some(t) => Cow::Borrowed(t.title.as_str()),
            None => fsx::title_cow(&self.path),
        }
    }
}

fn index(root: &Path) -> Vec<Song> {
    fsx::collect_tracks(root).into_iter().map(|p| Song::new(root, p)).collect()
}

pub struct Search {
    pub query: String,
    root: PathBuf,
    songs: Vec<Song>,
    walk: Option<Receiver<Vec<Song>>>,
    words: Vec<String>,
    found: Vec<usize>,
    pub selected: usize,
    pub offset: usize,
    by_tags: bool,
    pub marks: crate::marks::Marks<PathBuf>,
}

impl Search {
    pub fn open(root: PathBuf) -> Search {
        let (tx, rx) = mpsc::channel();
        let from = root.clone();
        let spawned = std::thread::Builder::new().name("urchin-search".into()).spawn(move || {
            let _ = tx.send(index(&from));
        });
        match spawned {
            Ok(_) => Search { walk: Some(rx), ..Search::with_songs(root, Vec::new()) },
            Err(_) => {
                let songs = index(&root);
                Search::with_songs(root, songs)
            }
        }
    }

    pub fn from_library(root: PathBuf, ix: &Index) -> Search {
        let songs = ix
            .query(None, None, None)
            .into_iter()
            .map(|i| &ix.tracks[i as usize])
            .map(|t| Song { path: t.path.clone(), key: fold(&ix.search_text(t)), tagged: Some(Tagged { title: t.title.clone(), trail: ix.trail(t) }) })
            .collect();
        Search { by_tags: true, ..Search::with_songs(root, songs) }
    }

    pub fn by_tags(&self) -> bool {
        self.by_tags
    }

    #[cfg(test)]
    pub fn from_tracks(root: PathBuf, tracks: Vec<PathBuf>) -> Search {
        let songs = tracks.into_iter().map(|p| Song::new(&root, p)).collect();
        Search::with_songs(root, songs)
    }

    fn with_songs(root: PathBuf, songs: Vec<Song>) -> Search {
        Search { query: String::new(), root, songs, walk: None, words: Vec::new(), found: Vec::new(), selected: 0, offset: 0, by_tags: false, marks: Default::default() }
    }

    pub fn poll(&mut self) {
        let Some(walk) = &self.walk else { return };
        match walk.try_recv() {
            Ok(songs) => self.songs = songs,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {}
        }
        self.walk = None;
        self.refilter();
    }

    pub fn indexing(&self) -> bool {
        self.walk.is_some()
    }

    pub fn is_blank(&self) -> bool {
        self.words.is_empty()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn total(&self) -> usize {
        self.songs.len()
    }

    pub fn count(&self) -> usize {
        self.found.len()
    }

    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    fn refilter(&mut self) {
        self.words = words(&self.query);
        let found: Vec<usize> = if self.words.is_empty() {
            Vec::new()
        } else {
            let words = &self.words;
            self.songs.iter().enumerate().filter(|(_, s)| words.iter().all(|w| s.key.contains(w.as_str()))).map(|(i, _)| i).collect()
        };
        if found != self.found {
            self.found = found;
            self.selected = 0;
            self.offset = 0;
        }
    }

    pub fn result(&self, i: usize) -> Option<&Song> {
        self.found.get(i).map(|&k| &self.songs[k])
    }

    pub fn ticked_paths(&self) -> Vec<PathBuf> {
        self.songs.iter().filter(|s| self.marks.contains(&s.path)).map(|s| s.path.clone()).collect()
    }

    pub fn highlighted(&self) -> Option<&Song> {
        self.result(self.selected)
    }

    pub fn select(&mut self, i: usize, height: usize) {
        let h = height.max(1);
        self.selected = i.min(self.found.len().saturating_sub(1));
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        self.offset = self.offset.min(self.found.len().saturating_sub(h));
    }

    pub fn fit(&mut self, height: usize) {
        self.offset = self.offset.min(self.found.len().saturating_sub(height.max(1)));
    }

    pub fn step(&mut self, delta: isize, height: usize) {
        let to = (self.selected as isize).saturating_add(delta).max(0) as usize;
        self.select(to, height);
    }

    pub fn scroll(&mut self, delta: isize, height: usize) {
        let max = self.found.len().saturating_sub(height.max(1));
        self.offset = (self.offset as isize).saturating_add(delta).clamp(0, max as isize) as usize;
    }

    pub fn playlist(&self) -> Option<(Vec<PathBuf>, usize)> {
        if self.found.is_empty() {
            return None;
        }
        Some((self.found.iter().map(|&k| self.songs[k].path.clone()).collect(), self.selected))
    }

    pub fn trail(&self, song: &Song) -> Vec<String> {
        if let Some(t) = &song.tagged {
            return t.trail.clone();
        }
        let below = song.path.strip_prefix(&self.root).ok().and_then(Path::parent);
        let folders: Vec<String> =
            below.into_iter().flat_map(Path::components).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if folders.is_empty() {
            vec![self.root.file_name().unwrap_or(self.root.as_os_str()).to_string_lossy().into_owned()]
        } else {
            folders
        }
    }
}

pub fn trail_text(folders: &[String], width: usize) -> String {
    let whole = folders.join(SEP);
    if whole.width() <= width || folders.len() < 2 {
        return whole;
    }
    for cut in 1..folders.len() {
        let tail = format!("{CUT}{}", folders[cut..].join(SEP));
        if tail.width() <= width {
            return tail;
        }
    }
    format!("{CUT}{}", folders[folders.len() - 1])
}

pub fn row_cells(title: usize, trail: usize, width: usize) -> (usize, usize) {
    const GAP: usize = 2;
    if width < 40 || trail == 0 {
        return (width, 0);
    }
    let left_over = width.saturating_sub(title + GAP);
    let trail_cells = left_over.max(width * 2 / 5).min(trail);
    (width - trail_cells - GAP, trail_cells)
}
