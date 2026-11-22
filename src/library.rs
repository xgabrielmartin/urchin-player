use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use symphonia::core::meta::{StandardTag, Tag};

use crate::fsx;
use crate::search::fold;

pub const UNKNOWN_GENRE: &str = "Unknown Genre";
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    pub genre: String,
    pub disc: u16,
    pub track: u16,
}

fn leading_number(s: &str) -> u16 {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

fn take(tags: &mut Tags, t: &Tag) {
    let value = crate::decode::tag_text(t);
    let value = value.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if value.is_empty() {
        return;
    }
    let text = |slot: &mut String| {
        if slot.is_empty() {
            *slot = value.to_string();
        }
    };
    let number = |n: u64| u16::try_from(n).ok().filter(|n| *n > 0).unwrap_or_else(|| leading_number(value));
    match &t.std {
        Some(StandardTag::TrackTitle(_)) => text(&mut tags.title),
        Some(StandardTag::Artist(_)) => text(&mut tags.artist),
        Some(StandardTag::AlbumArtist(_)) => text(&mut tags.album_artist),
        Some(StandardTag::Album(_)) => text(&mut tags.album),
        Some(StandardTag::Genre(_)) => text(&mut tags.genre),
        Some(StandardTag::TrackNumber(n)) if tags.track == 0 => tags.track = number(*n),
        Some(StandardTag::DiscNumber(n)) if tags.disc == 0 => tags.disc = number(*n),
        None if tags.track == 0 && matches!(t.raw.key.to_ascii_lowercase().as_str(), "iprt" | "itrk") => {
            tags.track = leading_number(value);
        }
        _ => {}
    }
}

pub fn read_tags(path: &Path) -> Tags {
    let mut tags = Tags::default();
    let Ok(mut format) = crate::decode::probe(path) else { return tags };
    crate::decode::each_tag(format.as_mut(), path, |t| take(&mut tags, t));
    tags
}

#[derive(Debug)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub genre: u32,
    pub artist: u32,
    pub album: u32,
    pub disc: u16,
    pub no: u16,
}

#[derive(Debug)]
pub struct Album {
    pub name: String,
    pub artist: u32,
}

#[derive(Debug, Default)]
pub struct Index {
    pub tracks: Vec<Track>,
    pub genres: Vec<String>,
    pub artists: Vec<String>,
    pub albums: Vec<Album>,
}

struct Name {
    display: String,
    unknown: bool,
}

fn sorted_names(names: HashMap<String, Name>) -> (Vec<String>, HashMap<String, u32>) {
    let mut all: Vec<(String, Name)> = names.into_iter().collect();
    all.sort_by(|(fa, a), (fb, b)| a.unknown.cmp(&b.unknown).then_with(|| natord::compare(fa, fb)));
    let places = all.iter().enumerate().map(|(i, (f, _))| (f.clone(), i as u32)).collect();
    (all.into_iter().map(|(_, n)| n.display).collect(), places)
}

impl Index {
    pub fn build(rows: Vec<(PathBuf, Tags)>) -> Index {
        let pick = |text: &str, unknown: &'static str| -> (String, Name) {
            let t = text.trim();
            if t.is_empty() {
                (fold(unknown), Name { display: unknown.to_string(), unknown: true })
            } else {
                (fold(t), Name { display: t.to_string(), unknown: false })
            }
        };
        struct Raw {
            genre: String,
            artist: String,
            album: String,
        }
        let mut genres = HashMap::new();
        let mut artists = HashMap::new();
        let mut albums: HashMap<(String, String), Name> = HashMap::new();
        let mut raw = Vec::with_capacity(rows.len());
        for (_, tags) in &rows {
            let (gk, g) = pick(&tags.genre, UNKNOWN_GENRE);
            let who = if tags.album_artist.trim().is_empty() { &tags.artist } else { &tags.album_artist };
            let (ak, a) = pick(who, UNKNOWN_ARTIST);
            let (bk, b) = pick(&tags.album, UNKNOWN_ALBUM);
            genres.entry(gk.clone()).or_insert(g);
            artists.entry(ak.clone()).or_insert(a);
            albums.entry((bk.clone(), ak.clone())).or_insert(b);
            raw.push(Raw { genre: gk, artist: ak, album: bk });
        }
        let (genre_names, genre_at) = sorted_names(genres);
        let (artist_names, artist_at) = sorted_names(artists);
        let mut album_list: Vec<((String, String), Name)> = albums.into_iter().collect();
        album_list.sort_by(|((na, aa), a), ((nb, ab), b)| {
            a.unknown.cmp(&b.unknown).then_with(|| natord::compare(na, nb)).then_with(|| natord::compare(aa, ab))
        });
        let album_at: HashMap<(String, String), u32> =
            album_list.iter().enumerate().map(|(i, (k, _))| (k.clone(), i as u32)).collect();
        let albums: Vec<Album> =
            album_list.into_iter().map(|((_, artist), n)| Album { name: n.display, artist: artist_at[&artist] }).collect();

        let tracks = rows
            .into_iter()
            .zip(raw)
            .map(|((path, tags), r)| Track {
                title: if tags.title.trim().is_empty() { fsx::title_from_path(&path) } else { tags.title.trim().to_string() },
                genre: genre_at[&r.genre],
                artist: artist_at[&r.artist],
                album: album_at[&(r.album, r.artist)],
                disc: tags.disc,
                no: tags.track,
                path,
            })
            .collect();
        Index { tracks, genres: genre_names, artists: artist_names, albums }
    }

    pub fn query(&self, genre: Option<u32>, artist: Option<u32>, album: Option<u32>) -> Vec<u32> {
        let mut found: Vec<u32> = (0..self.tracks.len() as u32)
            .filter(|&i| {
                let t = &self.tracks[i as usize];
                genre.is_none_or(|g| t.genre == g) && artist.is_none_or(|a| t.artist == a) && album.is_none_or(|b| t.album == b)
            })
            .collect();
        found.sort_by(|&a, &b| {
            let (a, b) = (&self.tracks[a as usize], &self.tracks[b as usize]);
            a.artist
                .cmp(&b.artist)
                .then(a.album.cmp(&b.album))
                .then(a.disc.cmp(&b.disc))
                .then((a.no == 0).cmp(&(b.no == 0)))
                .then(a.no.cmp(&b.no))
                .then_with(|| natord::compare_ignore_case(&a.title, &b.title))
                .then_with(|| a.path.cmp(&b.path))
        });
        found
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Cached {
    path: PathBuf,
    size: u64,
    mtime: u64,
    tags: Tags,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    version: u32,
    files: Vec<Cached>,
}

const CACHE_VERSION: u32 = 1;

pub fn cache_file() -> PathBuf {
    crate::settings::config_dir().join("library.json")
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let m = fs::metadata(path).ok()?;
    let secs = m.modified().ok()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Some((m.len(), secs))
}

fn load_cache(file: &Path) -> HashMap<PathBuf, Cached> {
    let Ok(bytes) = fs::read(file) else { return HashMap::new() };
    match serde_json::from_slice::<CacheFile>(&bytes) {
        Ok(c) if c.version == CACHE_VERSION => c.files.into_iter().map(|c| (c.path.clone(), c)).collect(),
        _ => HashMap::new(),
    }
}

fn save_cache(file: &Path, entries: &[Cached]) {
    let files: Vec<Cached> = entries.iter().filter(|c| c.path.to_str().is_some()).cloned().collect();
    let Ok(bytes) = serde_json::to_vec(&CacheFile { version: CACHE_VERSION, files }) else { return };
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = file.with_extension("json.tmp");
    if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

#[derive(Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
}

impl Progress {
    #[cfg(test)]
    pub fn set(&self, done: usize, total: usize) {
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn get(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

pub fn read_all(root: &Path, cache: Option<&Path>, progress: &Progress) -> Index {
    let paths = fsx::collect_tracks(root);
    progress.total.store(paths.len(), Ordering::Relaxed);
    let old = cache.map(load_cache).unwrap_or_default();

    let read = |path: &PathBuf| -> Cached {
        let (size, mtime) = stamp(path).unwrap_or((0, 0));
        let cached = old.get(path).filter(|c| c.size == size && c.mtime == mtime && size != 0);
        let entry = match cached {
            Some(c) => c.clone(),
            None => Cached { path: path.clone(), size, mtime, tags: read_tags(path) },
        };
        progress.done.fetch_add(1, Ordering::Relaxed);
        entry
    };
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
    let chunk = paths.len().div_ceil(threads).max(1);
    let entries: Vec<Cached> = std::thread::scope(|s| {
        let jobs: Vec<_> = paths.chunks(chunk).map(|part| s.spawn(|| part.iter().map(read).collect::<Vec<_>>())).collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    });

    if let Some(file) = cache {
        let same = entries.len() == old.len() && entries.iter().all(|e| old.get(&e.path).is_some_and(|o| o.size == e.size && o.mtime == e.mtime));
        if !same {
            save_cache(file, &entries);
        }
    }
    Index::build(entries.into_iter().map(|e| (e.path, e.tags)).collect())
}

pub fn tags_of(paths: &[PathBuf], cache: Option<&Path>) -> Vec<Tags> {
    let old = cache.map(load_cache).unwrap_or_default();
    let read = |path: &PathBuf| -> Tags {
        let hit = old.get(path).and_then(|c| stamp(path).filter(|&(size, mtime)| c.size == size && c.mtime == mtime && size != 0).map(|_| c.tags.clone()));
        hit.unwrap_or_else(|| read_tags(path))
    };
    if paths.len() < 32 {
        return paths.iter().map(read).collect();
    }
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let jobs: Vec<_> = paths.chunks(chunk).map(|part| s.spawn(|| part.iter().map(read).collect::<Vec<_>>())).collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    })
}

pub struct Row {
    pub id: Option<u32>,
    pub name: String,
    pub by: String,
    pub count: usize,
}

#[derive(Default)]
pub struct Column {
    pub rows: Vec<Row>,
    pub selected: usize,
    pub offset: usize,
}

impl Column {
    fn id(&self) -> Option<u32> {
        self.rows.get(self.selected).and_then(|r| r.id)
    }

    pub fn id_is_all(&self) -> bool {
        self.id().is_none()
    }

    fn select(&mut self, i: usize, height: usize) {
        self.selected = i.min(self.rows.len().saturating_sub(1));
        keep_visible(self.selected, &mut self.offset, self.rows.len(), height);
    }
}

fn keep_visible(selected: usize, offset: &mut usize, len: usize, height: usize) {
    let h = height.max(1);
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + h {
        *offset = selected + 1 - h;
    }
    *offset = (*offset).min(len.saturating_sub(h));
}

pub type Place = Option<(u32, u32, u32)>;

pub const SONGS: usize = 3;

pub struct View {
    index: Option<Arc<Index>>,
    walk: Option<Receiver<Index>>,
    pub progress: Arc<Progress>,
    root: PathBuf,
    pub focus: usize,
    pub cols: [Column; 3],
    pub songs: Vec<u32>,
    pub song_selected: usize,
    pub song_offset: usize,
    pub song_marks: crate::marks::Marks,
    located: std::cell::RefCell<Option<(PathBuf, Place)>>,
}

impl View {
    pub fn open(root: PathBuf) -> View {
        View::open_with(root, Some(cache_file()))
    }

    pub fn open_with(root: PathBuf, cache: Option<PathBuf>) -> View {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let (from, p, c) = (root.clone(), progress.clone(), cache.clone());
        let spawned = std::thread::Builder::new().name("urchin-library".into()).spawn(move || {
            let _ = tx.send(read_all(&from, c.as_deref(), &p));
        });
        let mut view = View::empty(root.clone(), progress.clone());
        match spawned {
            Ok(_) => view.walk = Some(rx),
            Err(_) => view.set_index(read_all(&root, cache.as_deref(), &progress)),
        }
        view
    }

    #[cfg(test)]
    pub fn reading_at(done: usize, total: usize) -> View {
        let progress = Arc::new(Progress::default());
        progress.set(done, total);
        View::empty(PathBuf::new(), progress)
    }

    #[cfg(test)]
    pub fn with_index(index: Index) -> View {
        let mut v = View::empty(PathBuf::new(), Arc::new(Progress::default()));
        v.set_index(index);
        v
    }

    fn empty(root: PathBuf, progress: Arc<Progress>) -> View {
        View {
            index: None,
            walk: None,
            progress,
            root,
            focus: 0,
            cols: Default::default(),
            songs: Vec::new(),
            song_selected: 0,
            song_offset: 0,
            song_marks: Default::default(),
            located: Default::default(),
        }
    }

    pub fn locate(&self, path: &Path) -> Place {
        let mut cache = self.located.borrow_mut();
        if let Some((p, found)) = cache.as_ref() {
            if p == path {
                return *found;
            }
        }
        let found = self.index.as_ref().and_then(|ix| ix.tracks.iter().find(|t| t.path == path)).map(|t| (t.genre, t.artist, t.album));
        *cache = Some((path.to_path_buf(), found));
        found
    }

    pub fn poll(&mut self) {
        let Some(walk) = &self.walk else { return };
        match walk.try_recv() {
            Ok(index) => self.set_index(index),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => self.set_index(Index::default()),
        }
        self.walk = None;
    }

    fn set_index(&mut self, index: Index) {
        self.index = Some(Arc::new(index));
        *self.located.borrow_mut() = None;
        self.rebuild_genres();
    }

    pub fn reading(&self) -> bool {
        self.index.is_none()
    }

    pub fn index(&self) -> Option<&Arc<Index>> {
        self.index.as_ref()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn rebuild_genres(&mut self) {
        let Some(ix) = &self.index else { return };
        let mut counts = vec![0usize; ix.genres.len()];
        for t in &ix.tracks {
            counts[t.genre as usize] += 1;
        }
        let rows = with_all(ix.tracks.len(), counts.iter().enumerate().filter(|(_, &n)| n > 0).map(|(i, &n)| Row {
            id: Some(i as u32),
            name: ix.genres[i].clone(),
            by: String::new(),
            count: n,
        }));
        self.cols[0] = Column { rows, ..Column::default() };
        self.rebuild_artists();
    }

    fn rebuild_artists(&mut self) {
        let Some(ix) = &self.index else { return };
        let genre = self.cols[0].id();
        let mut counts = vec![0usize; ix.artists.len()];
        let mut total = 0;
        for t in ix.tracks.iter().filter(|t| genre.is_none_or(|g| t.genre == g)) {
            counts[t.artist as usize] += 1;
            total += 1;
        }
        let rows = with_all(total, counts.iter().enumerate().filter(|(_, &n)| n > 0).map(|(i, &n)| Row {
            id: Some(i as u32),
            name: ix.artists[i].clone(),
            by: String::new(),
            count: n,
        }));
        self.cols[1] = Column { rows, ..Column::default() };
        self.rebuild_albums();
    }

    fn rebuild_albums(&mut self) {
        let Some(ix) = &self.index else { return };
        let (genre, artist) = (self.cols[0].id(), self.cols[1].id());
        let mut counts = vec![0usize; ix.albums.len()];
        let mut total = 0;
        for t in ix.tracks.iter().filter(|t| genre.is_none_or(|g| t.genre == g) && artist.is_none_or(|a| t.artist == a)) {
            counts[t.album as usize] += 1;
            total += 1;
        }
        let rows = with_all(total, counts.iter().enumerate().filter(|(_, &n)| n > 0).map(|(i, &n)| Row {
            id: Some(i as u32),
            name: ix.albums[i].name.clone(),
            by: ix.artists[ix.albums[i].artist as usize].clone(),
            count: n,
        }));
        self.cols[2] = Column { rows, ..Column::default() };
    }

    pub fn select(&mut self, col: usize, row: usize, height: usize) {
        if self.index.is_none() {
            return;
        }
        if col == SONGS {
            self.focus = SONGS;
            self.select_song(row, height);
            return;
        }
        let was = self.cols[col].id();
        self.cols[col].select(row, height);
        self.focus = col;
        if self.cols[col].id() != was {
            match col {
                0 => self.rebuild_artists(),
                1 => self.rebuild_albums(),
                _ => {}
            }
        }
    }

    pub fn select_song(&mut self, i: usize, height: usize) {
        self.song_selected = i.min(self.songs.len().saturating_sub(1));
        keep_visible(self.song_selected, &mut self.song_offset, self.songs.len(), height);
    }

    pub fn step(&mut self, delta: isize, height: usize) {
        if self.focus == SONGS {
            let to = (self.song_selected as isize).saturating_add(delta).max(0) as usize;
            self.select_song(to, height);
        } else {
            let to = (self.cols[self.focus].selected as isize).saturating_add(delta).max(0) as usize;
            self.select(self.focus, to, height);
        }
    }

    pub fn scroll(&mut self, col: usize, delta: isize, height: usize) {
        let (len, offset) = if col == SONGS { (self.songs.len(), &mut self.song_offset) } else { (self.cols[col].rows.len(), &mut self.cols[col].offset) };
        let max = len.saturating_sub(height.max(1));
        *offset = (*offset as isize).saturating_add(delta).clamp(0, max as isize) as usize;
    }

    pub fn fit(&mut self, height: usize) {
        for c in &mut self.cols {
            c.offset = c.offset.min(c.rows.len().saturating_sub(height.max(1)));
        }
        self.song_offset = self.song_offset.min(self.songs.len().saturating_sub(height.max(1)));
    }

    pub fn right(&mut self, height: usize) {
        if self.index.is_none() {
            return;
        }
        match self.focus {
            0 | 1 => self.focus += 1,
            2 => self.show_songs(height),
            _ => {}
        }
    }

    pub fn left(&mut self) -> bool {
        match self.focus {
            0 => false,
            SONGS => {
                self.leave_songs();
                true
            }
            _ => {
                self.focus -= 1;
                true
            }
        }
    }

    pub fn show_songs(&mut self, height: usize) {
        let Some(ix) = &self.index else { return };
        let (g, a, b) = self.filter(2);
        self.songs = ix.query(g, a, b);
        self.song_marks.clear();
        self.song_selected = 0;
        self.song_offset = 0;
        self.focus = SONGS;
        self.fit(height);
    }

    pub fn leave_songs(&mut self) {
        self.songs = Vec::new();
        self.song_marks.clear();
        self.song_selected = 0;
        self.song_offset = 0;
        self.focus = 2;
    }

    fn filter(&self, depth: usize) -> (Option<u32>, Option<u32>, Option<u32>) {
        let at = |c: usize| if depth >= c { self.cols[c].id() } else { None };
        (at(0), at(1), at(2))
    }

    pub fn songs_of(&self, col: usize) -> Vec<PathBuf> {
        let Some(ix) = &self.index else { return Vec::new() };
        let (g, a, b) = self.filter(col.min(2));
        ix.query(g, a, b).into_iter().map(|i| ix.tracks[i as usize].path.clone()).collect()
    }

    pub fn playlist(&self) -> Option<(Vec<PathBuf>, usize)> {
        let ix = self.index.as_ref()?;
        let (paths, from) = if self.focus == SONGS {
            (self.songs.iter().map(|&i| ix.tracks[i as usize].path.clone()).collect::<Vec<_>>(), self.song_selected)
        } else {
            (self.songs_of(self.focus), 0)
        };
        if paths.is_empty() {
            None
        } else {
            Some((paths, from))
        }
    }

    pub fn highlighted_song(&self) -> Option<&Track> {
        if self.focus != SONGS {
            return None;
        }
        let ix = self.index.as_ref()?;
        self.songs.get(self.song_selected).map(|&i| &ix.tracks[i as usize])
    }

    pub fn songs_title(&self) -> String {
        let picked: Vec<&str> = (0..3)
            .filter(|&c| self.cols[c].id().is_some())
            .map(|c| self.cols[c].rows[self.cols[c].selected].name.as_str())
            .collect();
        if picked.is_empty() {
            "All Songs".to_string()
        } else {
            picked.join(" › ")
        }
    }

    pub fn picked_name(&self, col: usize) -> String {
        match col {
            SONGS => self.highlighted_song().map(|t| t.title.clone()).unwrap_or_default(),
            c => self.cols[c].rows.get(self.cols[c].selected).map(|r| r.name.clone()).unwrap_or_default(),
        }
    }
}

fn with_all(total: usize, rows: impl Iterator<Item = Row>) -> Vec<Row> {
    std::iter::once(Row { id: None, name: "All".into(), by: String::new(), count: total }).chain(rows).collect()
}

impl Index {
    pub fn search_text(&self, t: &Track) -> String {
        format!("{} {} {} {}", t.title, self.artists[t.artist as usize], self.albums[t.album as usize].name, self.genres[t.genre as usize])
    }

    pub fn trail(&self, t: &Track) -> Vec<String> {
        vec![self.genres[t.genre as usize].clone(), self.artists[t.artist as usize].clone(), self.albums[t.album as usize].name.clone()]
    }
}

#[cfg(test)]
pub fn wav_with_tags(info: &[(&str, &str)]) -> Vec<u8> {
    let mut list = b"INFO".to_vec();
    for (id, text) in info {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        list.extend_from_slice(id.as_bytes());
        list.extend_from_slice(&(value.len() as u32).to_le_bytes());
        list.extend_from_slice(&value);
        if value.len() % 2 == 1 {
            list.push(0);
        }
    }
    let samples = vec![0u8; 800];
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&8000u32.to_le_bytes());
    fmt.extend_from_slice(&16000u32.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"LIST");
    body.extend_from_slice(&(list.len() as u32).to_le_bytes());
    body.extend_from_slice(&list);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    body.extend_from_slice(&samples);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(title: &str, artist: &str, album: &str, genre: &str, disc: u16, track: u16) -> Tags {
        Tags { title: title.into(), artist: artist.into(), album: album.into(), genre: genre.into(), disc, track, ..Tags::default() }
    }

    fn row(path: &str, t: Tags) -> (PathBuf, Tags) {
        (PathBuf::from(path), t)
    }

    fn names(ix: &Index, ids: &[u32]) -> Vec<String> {
        ids.iter().map(|&i| ix.tracks[i as usize].title.clone()).collect()
    }

    fn sample() -> Index {
        Index::build(vec![
            row("/m/1.flac", tags("Time", "Pink Floyd", "Dark Side", "Rock", 1, 4)),
            row("/m/2.flac", tags("Speak to Me", "Pink Floyd", "Dark Side", "Rock", 1, 1)),
            row("/m/3.flac", tags("Money", "pink floyd", "dark side", "rock", 1, 6)),
            row("/m/4.flac", tags("Roygbiv", "Boards of Canada", "Music Has the Right", "Electronic", 0, 0)),
            row("/m/5.flac", tags("Hello", "Adele", "Greatest Hits", "Pop", 0, 1)),
            row("/m/6.flac", tags("Hey", "Pixies", "Greatest Hits", "Rock", 0, 1)),
            row("/m/untagged.flac", Tags::default()),
        ])
    }

    #[test]
    fn leading_numbers_read_track_and_disc_tags() {
        assert_eq!(leading_number("3"), 3);
        assert_eq!(leading_number(" 03/12"), 3);
        assert_eq!(leading_number("x"), 0);
        assert_eq!(leading_number(""), 0);
        assert_eq!(leading_number("99999999"), 0, "too big for a track number is no number");
    }

    #[test]
    fn names_are_sorted_ignoring_case_with_unknown_last_and_spellings_merged() {
        let ix = sample();
        assert_eq!(ix.genres, ["Electronic", "Pop", "Rock", UNKNOWN_GENRE]);
        assert_eq!(ix.artists, ["Adele", "Boards of Canada", "Pink Floyd", "Pixies", UNKNOWN_ARTIST]);
        let hits: Vec<&Album> = ix.albums.iter().filter(|a| a.name == "Greatest Hits").collect();
        assert_eq!(hits.len(), 2);
        assert_eq!(ix.albums.last().unwrap().name, UNKNOWN_ALBUM);
        assert_eq!(ix.albums.iter().filter(|a| a.name == "Dark Side").count(), 1);
    }

    #[test]
    fn an_untagged_song_is_named_by_its_file_and_filed_under_unknown() {
        let ix = sample();
        let t = ix.tracks.iter().find(|t| t.path.ends_with("untagged.flac")).unwrap();
        assert_eq!(t.title, "untagged");
        assert_eq!(ix.genres[t.genre as usize], UNKNOWN_GENRE);
        assert_eq!(ix.artists[t.artist as usize], UNKNOWN_ARTIST);
        assert_eq!(ix.albums[t.album as usize].name, UNKNOWN_ALBUM);
    }

    #[test]
    fn the_album_artist_wins_over_the_artist_so_a_compilation_is_one_album() {
        let mut a = tags("One", "Band A", "Mix", "Pop", 1, 1);
        let mut b = tags("Two", "Band B", "Mix", "Pop", 1, 2);
        a.album_artist = "Various Artists".into();
        b.album_artist = "Various Artists".into();
        let ix = Index::build(vec![row("/m/a.mp3", a), row("/m/b.mp3", b)]);
        assert_eq!(ix.artists, ["Various Artists"]);
        assert_eq!(ix.albums.len(), 1);
    }

    #[test]
    fn songs_come_out_by_disc_then_track_number_with_unnumbered_ones_last() {
        let ix = Index::build(vec![
            row("/m/d2t1.mp3", tags("D2 T1", "A", "X", "G", 2, 1)),
            row("/m/b.mp3", tags("Bonus", "A", "X", "G", 1, 0)),
            row("/m/d1t10.mp3", tags("D1 T10", "A", "X", "G", 1, 10)),
            row("/m/d1t2.mp3", tags("D1 T2", "A", "X", "G", 1, 2)),
            row("/m/a.mp3", tags("Another", "A", "X", "G", 1, 0)),
        ]);
        assert_eq!(names(&ix, &ix.query(None, None, None)), ["D1 T2", "D1 T10", "Another", "Bonus", "D2 T1"]);
    }

    #[test]
    fn a_query_narrows_by_genre_artist_and_album() {
        let ix = sample();
        let rock = ix.genres.iter().position(|g| g == "Rock").unwrap() as u32;
        let floyd = ix.artists.iter().position(|g| g == "Pink Floyd").unwrap() as u32;
        assert_eq!(ix.query(Some(rock), None, None).len(), 4, "Dark Side x3 and the Pixies");
        assert_eq!(names(&ix, &ix.query(Some(rock), Some(floyd), None)), ["Speak to Me", "Time", "Money"]);
        assert!(ix.query(Some(rock + 1), Some(floyd), None).is_empty());
        assert_eq!(ix.query(None, None, None).len(), 7);
    }

    #[test]
    fn the_columns_narrow_left_to_right_and_all_comes_first() {
        let mut v = View::with_index(sample());
        let col = |v: &View, c: usize| v.cols[c].rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(col(&v, 0), ["All", "Electronic", "Pop", "Rock", UNKNOWN_GENRE]);
        assert_eq!(v.cols[0].rows[0].count, 7);
        assert_eq!(col(&v, 1).len(), 6, "All and five artists");

        let rock = v.cols[0].rows.iter().position(|r| r.name == "Rock").unwrap();
        v.select(0, rock, 10);
        assert_eq!(col(&v, 1), ["All", "Pink Floyd", "Pixies"]);
        assert_eq!(v.cols[1].rows[0].count, 4);
        assert_eq!(col(&v, 2), ["All", "Dark Side", "Greatest Hits"]);

        v.select(1, 2, 10);
        assert_eq!(col(&v, 2), ["All", "Greatest Hits"]);
        assert_eq!(v.cols[2].rows[1].by, "Pixies");

        v.select(0, 0, 10);
        assert_eq!(v.cols[1].selected, 0);
        assert_eq!(v.cols[2].selected, 0);
        assert_eq!(col(&v, 1).len(), 6);
    }

    #[test]
    fn keys_walk_right_into_the_songs_and_left_out_again() {
        let mut v = View::with_index(sample());
        let rock = v.cols[0].rows.iter().position(|r| r.name == "Rock").unwrap();
        v.select(0, rock, 10);
        v.right(10);
        assert_eq!(v.focus, 1);
        v.step(1, 10);
        v.right(10);
        assert_eq!(v.focus, 2);
        v.step(1, 10);
        v.right(10);
        assert_eq!(v.focus, SONGS);
        assert_eq!(v.songs.len(), 3);
        assert_eq!(v.songs_title(), "Rock › Pink Floyd › Dark Side");
        assert_eq!(v.highlighted_song().unwrap().title, "Speak to Me");
        v.step(1, 10);
        assert_eq!(v.highlighted_song().unwrap().title, "Time");
        assert!(v.left());
        assert_eq!((v.focus, v.songs.len()), (2, 0));
        assert!(v.left());
        assert!(v.left());
        assert_eq!(v.focus, 0);
        assert!(!v.left(), "nothing is left of the first column");
    }

    #[test]
    fn enter_plays_what_is_under_the_row_not_what_is_picked_further_right() {
        let mut v = View::with_index(sample());
        let rock = v.cols[0].rows.iter().position(|r| r.name == "Rock").unwrap();
        v.select(0, rock, 10);
        v.select(1, 2, 10);
        v.focus = 0;
        let (songs, from) = v.playlist().unwrap();
        assert_eq!((songs.len(), from), (4, 0));
        v.focus = 1;
        assert_eq!(v.playlist().unwrap().0.len(), 1);
        v.select(0, rock, 10);
        v.select(1, 1, 10);
        v.right(10);
        v.right(10);
        v.step(2, 10);
        let (songs, from) = v.playlist().unwrap();
        assert_eq!((songs.len(), from), (3, 2));
        assert!(songs[2].ends_with("3.flac"), "Money, the third of Dark Side");
    }

    #[test]
    fn an_empty_library_is_fine() {
        let mut v = View::with_index(Index::build(Vec::new()));
        assert_eq!(v.cols[0].rows.len(), 1);
        v.step(5, 10);
        v.right(10);
        v.right(10);
        v.right(10);
        assert!(v.playlist().is_none());
        assert_eq!(v.songs_title(), "All Songs");
    }

    #[test]
    fn scrolling_and_fitting_stay_inside_the_lists() {
        let rows: Vec<_> = (0..50).map(|i| row(&format!("/m/{i}.mp3"), tags(&format!("S{i}"), &format!("A{i:02}"), "X", "G", 0, 0))).collect();
        let mut v = View::with_index(Index::build(rows));
        v.scroll(1, 1000, 10);
        assert_eq!(v.cols[1].offset, 51 - 10);
        v.scroll(1, -1000, 10);
        assert_eq!(v.cols[1].offset, 0);
        v.select(1, 40, 10);
        assert_eq!(v.cols[1].selected, 40);
        assert_eq!(v.cols[1].offset, 31);
        v.fit(30);
        assert_eq!(v.cols[1].offset, 21);
    }

    #[test]
    fn search_text_and_trail_use_the_tags() {
        let ix = sample();
        let t = &ix.tracks[0];
        let text = fold(&ix.search_text(t));
        assert!(text.contains("pink floyd") && text.contains("dark side") && text.contains("rock") && text.contains("time"));
        assert_eq!(ix.trail(t), ["Rock", "Pink Floyd", "Dark Side"]);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("urchin_library_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tags_are_read_from_a_real_file_and_a_file_with_none_has_none() {
        let dir = scratch("tags");
        let a = dir.join("a.wav");
        fs::write(&a, wav_with_tags(&[("INAM", "Time"), ("IART", "Pink Floyd"), ("IPRD", "Dark Side"), ("IGNR", "Rock"), ("IPRT", "4")])).unwrap();
        let t = read_tags(&a);
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.album.as_str(), t.genre.as_str()), ("Time", "Pink Floyd", "Dark Side", "Rock"));
        assert_eq!(t.track, 4);
        let b = dir.join("b.wav");
        fs::write(&b, wav_with_tags(&[])).unwrap();
        assert_eq!(read_tags(&b), Tags::default());
        assert_eq!(read_tags(&dir.join("missing.wav")), Tags::default());
        let junk = dir.join("junk.mp3");
        fs::write(&junk, b"not audio at all").unwrap();
        assert_eq!(read_tags(&junk), Tags::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cache_keeps_tags_and_only_changed_files_are_read_again() {
        let dir = scratch("cache");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        let cache = dir.join("library.json");
        fs::write(music.join("a.wav"), wav_with_tags(&[("INAM", "One"), ("IART", "Ann"), ("IGNR", "Pop")])).unwrap();
        fs::write(music.join("b.wav"), wav_with_tags(&[("INAM", "Two"), ("IART", "Bob"), ("IGNR", "Pop")])).unwrap();

        let progress = Progress::default();
        let ix = read_all(&music, Some(&cache), &progress);
        assert_eq!(progress.get(), (2, 2));
        assert_eq!(ix.artists, ["Ann", "Bob"]);
        assert!(cache.is_file());

        let mut text = fs::read_to_string(&cache).unwrap();
        text = text.replace("\"Ann\"", "\"Anna\"");
        fs::write(&cache, &text).unwrap();
        let ix = read_all(&music, Some(&cache), &Progress::default());
        assert_eq!(ix.artists, ["Anna", "Bob"]);

        fs::write(music.join("a.wav"), wav_with_tags(&[("INAM", "One"), ("IART", "Ann"), ("IGNR", "Pop"), ("IPRD", "Longer")])).unwrap();
        fs::write(music.join("c.wav"), wav_with_tags(&[("INAM", "Three"), ("IART", "Cy"), ("IGNR", "Pop")])).unwrap();
        let ix = read_all(&music, Some(&cache), &Progress::default());
        assert_eq!(ix.artists, ["Ann", "Bob", "Cy"]);

        fs::write(&cache, b"{ not json").unwrap();
        assert_eq!(read_all(&music, Some(&cache), &Progress::default()).tracks.len(), 3);
        fs::write(&cache, br#"{"version":99,"files":[]}"#).unwrap();
        assert_eq!(read_all(&music, Some(&cache), &Progress::default()).tracks.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_view_reads_in_the_background_and_shows_it_when_polled() {
        let dir = scratch("view");
        fs::write(dir.join("a.wav"), wav_with_tags(&[("INAM", "One"), ("IART", "Ann"), ("IGNR", "Pop")])).unwrap();
        let mut v = View::open_with(dir.clone(), Some(dir.join("c").join("library.json")));
        for _ in 0..500 {
            v.poll();
            if !v.reading() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!v.reading());
        assert_eq!(v.cols[0].rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["All", "Pop"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
