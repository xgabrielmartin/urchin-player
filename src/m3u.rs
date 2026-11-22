use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Path(PathBuf),
    Url(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub target: Target,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

impl Item {
    pub fn path(path: impl Into<PathBuf>) -> Item {
        Item { target: Target::Path(path.into()), title: None, duration: None }
    }

    pub fn with_info(mut self, title: impl Into<String>, duration: Option<f64>) -> Item {
        let title = title.into();
        self.title = if title.is_empty() { None } else { Some(title) };
        self.duration = duration.filter(|d| d.is_finite() && *d >= 0.0);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub items: Vec<Item>,
}

impl Playlist {
    pub fn from_paths(paths: &[PathBuf]) -> Playlist {
        Playlist { items: paths.iter().map(Item::path).collect() }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Tracks {
    pub paths: Vec<PathBuf>,
    pub skipped: usize,
}

impl Playlist {
    pub fn open(path: &Path) -> io::Result<Playlist> {
        let bytes = fs::read(path)?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        Ok(Playlist::parse(&bytes, base))
    }

    pub fn parse(bytes: &[u8], base: &Path) -> Playlist {
        let text = decode(bytes);
        let mut items = Vec::new();
        let mut pending: Option<(Option<f64>, String)> = None;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                if let Some(info) = strip_prefix_ci(rest, "EXTINF:") {
                    pending = Some(parse_extinf(info));
                }
                continue;
            }
            let mut item = Item { target: target_of(line, base), title: None, duration: None };
            if let Some((duration, title)) = pending.take() {
                item = item.with_info(title, duration);
            }
            items.push(item);
        }
        Playlist { items }
    }

    pub fn tracks(&self, is_audio: impl Fn(&Path) -> bool) -> Tracks {
        let mut out = Tracks::default();
        for item in &self.items {
            match &item.target {
                Target::Path(p) if is_audio(p) && p.is_file() => out.paths.push(p.clone()),
                _ => out.skipped += 1,
            }
        }
        out
    }

    #[allow(dead_code)]
    pub fn to_string(&self, playlist_path: &Path) -> String {
        let base = playlist_path.parent().unwrap_or_else(|| Path::new(""));
        self.render(Some(base))
    }

    pub fn to_string_absolute(&self) -> String {
        self.render(None)
    }

    fn render(&self, relative_to_dir: Option<&Path>) -> String {
        let mut out = String::from("#EXTM3U\n");
        for item in &self.items {
            let line = match &item.target {
                Target::Path(p) => match relative_to_dir {
                    Some(base) => path_text(&relative_to(p, base)),
                    None => path_text(p),
                },
                Target::Url(u) => u.clone(),
            };
            if item.title.is_some() || item.duration.is_some() {
                let secs = item.duration.map_or(-1, |d| d.round() as i64);
                let title = item.title.as_deref().unwrap_or("");
                out.push_str(&format!("#EXTINF:{secs},{}\n", one_line(title)));
            }
            out.push_str(&one_line(&line));
            out.push('\n');
        }
        out
    }

    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, &self.to_string(path))
    }

    pub fn save_absolute(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, &self.to_string_absolute())
    }
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn is_playlist(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8"))
}

fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn utf16(bytes: &[u8], pair: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| pair([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

fn parse_extinf(info: &str) -> (Option<f64>, String) {
    let (head, title) = info.split_once(',').unwrap_or((info, ""));
    let duration = head
        .split_whitespace()
        .next()
        .and_then(|w| w.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d >= 0.0);
    (duration, title.trim().to_string())
}

fn target_of(line: &str, base: &Path) -> Target {
    let path = match strip_prefix_ci(line, "file://") {
        Some(rest) if !local_file_url(rest) => return Target::Url(line.to_string()),
        Some(rest) => PathBuf::from(file_url_path(rest)),
        None if is_url(line) => return Target::Url(line.to_string()),
        None => native_path(line),
    };
    if cfg!(windows) && is_network_path(&path.to_string_lossy()) {
        return Target::Url(line.to_string());
    }
    Target::Path(clean(&base.join(path)))
}

fn local_file_url(rest: &str) -> bool {
    let host = rest.split('/').next().unwrap_or("");
    let b = host.as_bytes();
    host.is_empty() || host.eq_ignore_ascii_case("localhost") || (b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

fn is_network_path(s: &str) -> bool {
    let mut c = s.chars();
    matches!((c.next(), c.next()), (Some('/' | '\\'), Some('/' | '\\')))
}

fn is_url(s: &str) -> bool {
    match s.split_once("://") {
        Some((scheme, _)) => {
            scheme.len() >= 2
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

fn file_url_path(rest: &str) -> String {
    let rest = strip_prefix_ci(rest, "localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    let b = decoded.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return decoded[1..].to_string();
    }
    decoded
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn native_path(line: &str) -> PathBuf {
    if cfg!(windows) || !line.contains('\\') || line.contains('/') {
        PathBuf::from(line)
    } else {
        PathBuf::from(line.replace('\\', "/"))
    }
}

fn clean(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out.iter().map(|c| c.as_os_str()).collect()
}

fn relative_to(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() != base.is_absolute() {
        return path.to_path_buf();
    }
    let p: Vec<Component> = path.components().collect();
    let b: Vec<Component> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 || (path.is_absolute() && common == 1) {
        return path.to_path_buf();
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() { PathBuf::from(".") } else { out }
}

fn path_text(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { s.replace('\\', "/") } else { s.into_owned() }
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[Item]) -> Vec<&Target> {
        items.iter().map(|i| &i.target).collect()
    }

    #[test]
    fn reads_extended_and_plain() {
        let text = b"#EXTM3U\n#EXTINF:215,Artist - Title\nmusic/a.mp3\nb.flac\n";
        let pl = Playlist::parse(text, Path::new("/pl"));
        assert_eq!(pl.items.len(), 2);
        assert_eq!(pl.items[0].target, Target::Path("/pl/music/a.mp3".into()));
        assert_eq!(pl.items[0].title.as_deref(), Some("Artist - Title"));
        assert_eq!(pl.items[0].duration, Some(215.0));
        assert_eq!(pl.items[1].title, None);
    }

    #[test]
    fn bom_crlf_and_blank_lines() {
        let text = b"\xEF\xBB\xBF#EXTM3U\r\n\r\n#EXTINF:1,x\r\na.mp3\r\n";
        let pl = Playlist::parse(text, Path::new("/"));
        assert_eq!(pl.items.len(), 1);
        assert_eq!(pl.items[0].title.as_deref(), Some("x"));
    }

    #[test]
    fn utf16_and_legacy_codepage() {
        let mut u16le = vec![0xFF, 0xFE];
        for u in "#EXTM3U\nsøng.mp3\n".encode_utf16() {
            u16le.extend(u.to_le_bytes());
        }
        let pl = Playlist::parse(&u16le, Path::new("/"));
        assert_eq!(pl.items[0].target, Target::Path("/søng.mp3".into()));
        let pl = Playlist::parse(b"caf\xE9.mp3\n", Path::new("/"));
        assert_eq!(pl.items[0].target, Target::Path("/café.mp3".into()));
    }

    #[test]
    fn windows_paths_urls_and_file_urls() {
        let pl = Playlist::parse(
            b"..\\Other Music\\Bar.mp3\nhttp://x.example/live\nfile:///Users/me/My%20Song.mp3\nC:\\Music\\a.mp3\n",
            Path::new("/pl"),
        );
        let t = p(&pl.items);
        if !cfg!(windows) {
            assert_eq!(t[0], &Target::Path("/Other Music/Bar.mp3".into()));
        }
        assert_eq!(t[1], &Target::Url("http://x.example/live".into()));
        assert_eq!(t[2], &Target::Path("/Users/me/My Song.mp3".into()));
        assert!(matches!(t[3], Target::Path(_)));
    }

    #[test]
    fn extinf_variants() {
        assert_eq!(parse_extinf("-1 tvg-id=\"x\" group-title=\"y\",Name"), (None, "Name".into()));
        assert_eq!(parse_extinf("215.5,A, B"), (Some(215.5), "A, B".into()));
        assert_eq!(parse_extinf("abc"), (None, String::new()));
        let pl = Playlist::parse(b"#extinf:3,t\na.mp3\n", Path::new("/"));
        assert_eq!(pl.items[0].duration, Some(3.0));
    }

    #[test]
    fn other_machines_are_never_paths() {
        let text = b"file://evil.example/s/a.mp3\nfile:///Music/b.mp3\nfile://LOCALHOST/Music/c.mp3\n";
        let pl = Playlist::parse(text, Path::new("/pl"));
        assert_eq!(pl.items[0].target, Target::Url("file://evil.example/s/a.mp3".into()));
        assert_eq!(pl.items[1].target, Target::Path("/Music/b.mp3".into()));
        assert_eq!(pl.items[2].target, Target::Path("/Music/c.mp3".into()));
        assert!(local_file_url("C:/Music/a.mp3"));
        for s in [r"\\evil\s\a.mp3", "//evil/s/a.mp3", r"\/evil\s", r"\\?\UNC\evil\s", r"\\.\pipe\x"] {
            assert!(is_network_path(s), "{s}");
        }
        for s in [r"C:\Music\a.mp3", "/Music/a.mp3", "Music/a.mp3", r"\Music\a.mp3"] {
            assert!(!is_network_path(s), "{s}");
        }
    }

    #[test]
    fn percent_decode_edges() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%41"), "A");
    }

    #[test]
    fn writes_relative_paths_and_round_trips() {
        let pl = Playlist {
            items: vec![
                Item::path("/m/Rock/a.flac").with_info("A\nB", Some(200.4)),
                Item::path("/m/lists/b.mp3"),
                Item::path("/other/c.mp3"),
                Item { target: Target::Url("http://s/x".into()), title: None, duration: None },
            ],
        };
        let text = pl.to_string(Path::new("/m/lists/mix.m3u8"));
        assert_eq!(
            text,
            "#EXTM3U\n#EXTINF:200,A B\n../Rock/a.flac\nb.mp3\n/other/c.mp3\nhttp://s/x\n"
        );
        let back = Playlist::parse(text.as_bytes(), Path::new("/m/lists"));
        assert_eq!(back.items.len(), 4);
        assert_eq!(back.items[0].title.as_deref(), Some("A B"));
        assert_eq!(back.items[0].duration, Some(200.0));
    }

    #[test]
    fn tracks_skip_missing_streams_and_playlists() {
        let dir = std::env::temp_dir().join(format!("urchin_m3u_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.mp3"), b"x").unwrap();
        fs::write(dir.join("other.m3u"), b"a.mp3\n").unwrap();
        fs::write(
            dir.join("mix.m3u8"),
            b"a.mp3\ngone.mp3\nother.m3u\nhttp://s/x\n",
        )
        .unwrap();
        let pl = Playlist::open(&dir.join("mix.m3u8")).unwrap();
        let t = pl.tracks(|p| p.extension().is_some_and(|e| e == "mp3"));
        assert_eq!(t.paths, vec![dir.join("a.mp3")]);
        assert_eq!(t.skipped, 3);
        pl.save(&dir.join("copy.m3u8")).unwrap();
        assert!(!dir.join("copy.m3u8.tmp").exists());
        let full = pl.to_string_absolute();
        assert!(full.contains(&format!("{}", dir.join("a.mp3").display())), "{full}");
        assert!(is_playlist(&dir.join("copy.M3U8")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
