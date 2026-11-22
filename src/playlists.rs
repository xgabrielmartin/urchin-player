use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::m3u::{self, Playlist};

pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::fsx::home().join(".config"));
    let dir = base.join("urchin").join("playlists");
    let old = crate::settings::config_dir().join("playlists");
    if !dir.exists() && old != dir && old.is_dir() {
        if let Some(parent) = dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::rename(&old, &dir);
    }
    dir
}

#[derive(Debug, PartialEq)]
pub struct Imported {
    pub path: PathBuf,
    pub songs: usize,
    pub skipped: usize,
}

pub fn list_in(library: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(library)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| m3u::is_playlist(p) && p.is_file())
        .collect();
    found.sort_by(|a, b| natord::compare_ignore_case(&name_of(a), &name_of(b)));
    found
}

pub fn name_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn import_into(src: &Path, library: &Path) -> io::Result<Imported> {
    let playlist = Playlist::open(src)?;
    if playlist.items.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "there are no songs in it"));
    }
    let tracks = playlist.tracks(crate::fsx::is_audio);
    let summary = |path: PathBuf| Imported { path, songs: tracks.paths.len(), skipped: tracks.skipped };

    fs::create_dir_all(library)?;
    let already_here = match (fs::canonicalize(src), fs::canonicalize(library)) {
        (Ok(s), Ok(l)) => s.parent() == Some(l.as_path()),
        _ => false,
    };
    if already_here {
        return Ok(summary(src.to_path_buf()));
    }

    let stem: String = name_of(src).chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    let stem = if stem.trim().is_empty() { "playlist".to_string() } else { stem };
    let mut dest = library.join(format!("{stem}.m3u8"));
    let mut n = 2;
    while dest.exists() {
        dest = library.join(format!("{stem}_{n}.m3u8"));
        n += 1;
    }
    playlist.save_absolute(&dest)?;
    Ok(summary(dest))
}

fn file_stem_for(name: &str) -> String {
    let stem: String = name.trim().chars().map(|c| if matches!(c, '/' | '\\' | '\0') || c.is_control() { '_' } else { c }).collect();
    let stem = stem.trim_start_matches('.').to_string();
    if stem.is_empty() { "playlist".to_string() } else { stem }
}

pub fn rename(path: &Path, name: &str) -> io::Result<PathBuf> {
    if name.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the playlist needs a name"));
    }
    let ext = path.extension().map_or_else(|| "m3u8".into(), |e| e.to_string_lossy().into_owned());
    let dest = path.with_file_name(format!("{}.{ext}", file_stem_for(name)));
    if dest == path {
        return Ok(dest);
    }
    let same_file = matches!((fs::canonicalize(path), fs::canonicalize(&dest)), (Ok(a), Ok(b)) if a == b);
    if dest.exists() && !same_file {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "there is already a playlist with that name"));
    }
    fs::rename(path, &dest)?;
    Ok(dest)
}

pub fn delete(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

pub fn create_in(name: &str, library: &Path) -> io::Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the playlist needs a name"));
    }
    let stem = file_stem_for(name);
    fs::create_dir_all(library)?;
    let mut n = 1;
    loop {
        let file = if n == 1 { format!("{stem}.m3u8") } else { format!("{stem}_{n}.m3u8") };
        let dest = library.join(file);
        match fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(b"#EXTM3U\n")?;
                return Ok(dest);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

pub fn save_songs_as(name: &str, library: &Path, songs: &[PathBuf]) -> io::Result<PathBuf> {
    let path = create_in(name, library)?;
    let list = Playlist::from_paths(songs);
    if let Err(e) = list.save_absolute(&path) {
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

pub fn add_song(playlist: &Path, song: &Path) -> io::Result<bool> {
    let bytes = fs::read(playlist)?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the playlist isn't UTF-8 text"));
    }
    let base = playlist.parent().unwrap_or_else(|| Path::new(""));
    let already = Playlist::parse(&bytes, base).items.iter().any(|i| matches!(&i.target, m3u::Target::Path(p) if p == song));
    if already {
        return Ok(false);
    }
    let line = song
        .to_str()
        .filter(|s| !s.contains(['\r', '\n']))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the file name can't be written to a playlist"))?;
    let mut add = String::new();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        add.push('\n');
    }
    add.push_str(line);
    add.push('\n');
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(playlist)?.write_all(add.as_bytes())?;
    Ok(true)
}

pub struct View {
    pub files: Vec<PathBuf>,
    pub names: Vec<String>,
    pub selected: usize,
    pub offset: usize,
    pub on_songs: bool,
    pub songs: Vec<PathBuf>,
    pub skipped: usize,
    pub song_selected: usize,
    pub song_offset: usize,
    pub song_marks: crate::marks::Marks,
}

impl View {
    pub fn open(library: &Path, select: Option<&Path>) -> View {
        let files = list_in(library);
        let names = files.iter().map(|p| name_of(p)).collect();
        let selected = select.and_then(|s| files.iter().position(|f| f == s)).unwrap_or(0);
        let mut v = View {
            files,
            names,
            selected,
            offset: 0,
            on_songs: false,
            songs: Vec::new(),
            skipped: 0,
            song_selected: 0,
            song_offset: 0,
            song_marks: Default::default(),
        };
        v.load_songs();
        v
    }

    fn load_songs(&mut self) {
        self.songs.clear();
        self.song_marks.clear();
        self.skipped = 0;
        self.song_selected = 0;
        self.song_offset = 0;
        if let Some(file) = self.files.get(self.selected) {
            if let Ok(pl) = Playlist::open(file) {
                let t = pl.tracks(crate::fsx::is_audio);
                self.songs = t.paths;
                self.skipped = t.skipped;
            }
        }
    }

    pub fn select_list(&mut self, i: usize, height: usize) {
        if self.files.is_empty() {
            return;
        }
        let i = i.min(self.files.len() - 1);
        if i != self.selected {
            self.selected = i;
            self.load_songs();
        }
        keep_visible(self.selected, &mut self.offset, height);
    }

    pub fn select_song(&mut self, i: usize, height: usize) {
        if self.songs.is_empty() {
            return;
        }
        self.song_selected = i.min(self.songs.len() - 1);
        keep_visible(self.song_selected, &mut self.song_offset, height);
    }

    pub fn step(&mut self, delta: isize, height: usize) {
        if self.on_songs {
            let to = (self.song_selected as isize + delta).max(0) as usize;
            self.select_song(to, height);
        } else {
            let to = (self.selected as isize + delta).max(0) as usize;
            self.select_list(to, height);
        }
    }
}

fn keep_visible(selected: usize, offset: &mut usize, height: usize) {
    let h = height.max(1);
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + h {
        *offset = selected + 1 - h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("urchin_playlists_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn create_makes_an_empty_playlist_and_never_overwrites() {
        let root = scratch("create");
        let lib = root.join("playlists");
        let a = create_in("  Road Trip ", &lib).unwrap();
        assert_eq!(a, lib.join("Road Trip.m3u8"));
        assert_eq!(fs::read_to_string(&a).unwrap(), "#EXTM3U\n");
        let b = create_in("Road Trip", &lib).unwrap();
        assert_eq!(b, lib.join("Road Trip_2.m3u8"));
        assert_eq!(create_in("a/b\\c", &lib).unwrap(), lib.join("a_b_c.m3u8"));
        assert_eq!(create_in("..x", &lib).unwrap(), lib.join("x.m3u8"));
        assert!(create_in("   ", &lib).is_err());
        assert_eq!(list_in(&lib).len(), 4);
        assert!(Playlist::open(&a).unwrap().items.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_and_delete() {
        let root = scratch("rename");
        let lib = root.join("playlists");
        let a = create_in("one", &lib).unwrap();
        let b = create_in("two", &lib).unwrap();
        fs::write(&a, "#EXTM3U\n/x/song.mp3\n").unwrap();
        let c = rename(&a, " uno ").unwrap();
        assert_eq!(c, lib.join("uno.m3u8"));
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(&c).unwrap(), "#EXTM3U\n/x/song.mp3\n");
        assert_eq!(rename(&c, "two").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(rename(&c, "  ").is_err());
        assert_eq!(rename(&c, "uno").unwrap(), c);
        let d = rename(&c, "UNO").unwrap();
        assert_eq!(d, lib.join("UNO.m3u8"));
        let m = lib.join("old.m3u");
        fs::write(&m, "#EXTM3U\n").unwrap();
        assert_eq!(rename(&m, "new").unwrap(), lib.join("new.m3u"));
        delete(&b).unwrap();
        assert!(!b.exists());
        assert!(delete(&b).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn import_copies_with_full_paths_and_never_overwrites() {
        let root = scratch("import");
        let music = root.join("music");
        let lists = root.join("lists");
        fs::create_dir_all(&music).unwrap();
        fs::create_dir_all(&lists).unwrap();
        fs::write(music.join("a.wav"), b"x").unwrap();
        fs::write(lists.join("mix.m3u"), "#EXTM3U\n#EXTINF:5,A\n../music/a.wav\nmissing.wav\nhttp://s/x\n").unwrap();
        let library = root.join("library");

        let first = import_into(&lists.join("mix.m3u"), &library).unwrap();
        assert_eq!(first.path, library.join("mix.m3u8"));
        assert_eq!((first.songs, first.skipped), (1, 2));
        let text = fs::read_to_string(&first.path).unwrap();
        assert!(text.contains(&music.join("a.wav").display().to_string()), "{text}");
        assert!(!text.contains("../music"), "{text}");

        let second = import_into(&lists.join("mix.m3u"), &library).unwrap();
        assert_eq!(second.path, library.join("mix_2.m3u8"));
        assert_eq!(list_in(&library), vec![first.path.clone(), second.path]);

        let again = import_into(&first.path, &library).unwrap();
        assert_eq!(again.path, first.path);
        assert_eq!(list_in(&library).len(), 2);

        fs::write(lists.join("empty.m3u"), "#EXTM3U\n").unwrap();
        assert!(import_into(&lists.join("empty.m3u"), &library).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn add_song_appends_once_and_leaves_the_rest_alone() {
        let root = scratch("add");
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        fs::write(&a, b"x").unwrap();
        fs::write(&b, b"x").unwrap();
        let list = root.join("mix.m3u8");
        fs::write(&list, format!("#EXTM3U\n#EXTINF:5,Keep Me\n{}", a.display())).unwrap();
        assert!(add_song(&list, &b).unwrap());
        assert!(!add_song(&list, &b).unwrap(), "a second add changes nothing");
        assert!(!add_song(&list, &a).unwrap(), "already in the file");
        assert_eq!(fs::read_to_string(&list).unwrap(), format!("#EXTM3U\n#EXTINF:5,Keep Me\n{}\n{}\n", a.display(), b.display()));
        let t = Playlist::open(&list).unwrap().tracks(crate::fsx::is_audio);
        assert_eq!(t.paths, vec![a, b]);
        assert!(add_song(&root.join("missing.m3u8"), &root.join("c.wav")).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn view_shows_the_selected_playlists_songs() {
        let root = scratch("view");
        fs::write(root.join("a.wav"), b"x").unwrap();
        fs::write(root.join("b.wav"), b"x").unwrap();
        fs::write(root.join("mix 2.m3u8"), format!("{}\n", root.join("a.wav").display())).unwrap();
        fs::write(root.join("mix 10.m3u8"), format!("{}\n{}\ngone.wav\n", root.join("a.wav").display(), root.join("b.wav").display())).unwrap();

        let mut v = View::open(&root, None);
        assert_eq!(v.names, vec!["mix 2", "mix 10"]);
        assert_eq!(v.songs.len(), 1);
        v.step(1, 10);
        assert_eq!((v.songs.len(), v.skipped), (2, 1));
        v.on_songs = true;
        v.step(5, 10);
        assert_eq!(v.song_selected, 1);
        fs::remove_dir_all(&root).unwrap();
    }
}
