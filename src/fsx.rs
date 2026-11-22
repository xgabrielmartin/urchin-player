use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Track,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
}

pub fn save_if_different(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(true)
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(crate::decode::is_playable)
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    natord::compare_ignore_case(a, b)
}

pub fn list(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    list_matching(dir, show_hidden, is_audio)
}

pub fn list_in(dir: &Path, show_hidden: bool, order: &crate::order::Order) -> Vec<Entry> {
    let mut entries = list(dir, show_hidden);
    order.apply(dir, &mut entries);
    entries
}

pub fn list_playlists(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    list_matching(dir, show_hidden, crate::m3u::is_playlist)
}

fn list_matching(dir: &Path, show_hidden: bool, keep: impl Fn(&Path) -> bool) -> Vec<Entry> {
    let mut folders = Vec::new();
    let mut tracks = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let path = e.path();
            let (is_dir, is_file) = match e.file_type() {
                Ok(t) if t.is_symlink() => fs::metadata(&path).map_or((false, false), |m| (m.is_dir(), m.is_file())),
                Ok(t) => (t.is_dir(), t.is_file()),
                Err(_) => (false, false),
            };
            if is_dir {
                folders.push(Entry { name, path, kind: Kind::Folder });
            } else if is_file && keep(&path) {
                tracks.push(Entry { name, path, kind: Kind::Track });
            }
        }
    }
    folders.sort_by(|a, b| natural(&a.name, &b.name));
    tracks.sort_by(|a, b| natural(&a.name, &b.name));
    folders.extend(tracks);
    folders
}

pub fn collect_tracks(dir: &Path) -> Vec<PathBuf> {
    collect_tracks_in(dir, &crate::order::Order::empty())
}

pub fn collect_tracks_in(dir: &Path, order: &crate::order::Order) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_into(dir, order, &mut seen, &mut out);
    out
}

fn collect_into(
    dir: &Path,
    order: &crate::order::Order,
    seen: &mut std::collections::HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let real = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(real) {
        return;
    }
    for e in list_in(dir, false, order) {
        match e.kind {
            Kind::Track => out.push(e.path),
            Kind::Folder => collect_into(&e.path, order, seen, out),
        }
    }
}

pub fn subfolders(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    list(dir, show_hidden).into_iter().filter(|e| e.kind == Kind::Folder).collect()
}

pub fn title_cow(p: &Path) -> Cow<'_, str> {
    p.file_stem().map_or(Cow::Borrowed(""), |s| s.to_string_lossy())
}

pub fn title_from_path(p: &Path) -> String {
    title_cow(p).into_owned()
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn default_music_dir() -> PathBuf {
    dirs::audio_dir()
        .filter(|p| p.is_dir())
        .or_else(|| Some(home().join("Music")).filter(|p| p.is_dir()))
        .unwrap_or_else(home)
}

pub fn drives() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    for e in list(Path::new("/Volumes"), false) {
        out.push(e.path);
    }
    #[cfg(target_os = "linux")]
    for base in ["/media", "/mnt", "/run/media"] {
        for e in subfolders(Path::new(base), false) {
            let inner = subfolders(&e.path, false);
            if base != "/mnt" && !inner.is_empty() && e.name == whoami() {
                out.extend(inner.into_iter().map(|i| i.path));
            } else {
                out.push(e.path);
            }
        }
    }
    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let p = PathBuf::from(format!("{}:\\", letter as char));
        if p.exists() {
            out.push(p);
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn whoami() -> String {
    std::env::var("USER").unwrap_or_default()
}

pub fn complete_path(input: &str) -> Option<String> {
    let expanded = expand_tilde(input);
    let (dir, partial) = if expanded.ends_with(std::path::MAIN_SEPARATOR) {
        (PathBuf::from(&expanded), String::new())
    } else {
        let p = PathBuf::from(&expanded);
        let parent = p.parent()?.to_path_buf();
        let partial = p.file_name()?.to_string_lossy().to_string();
        (parent, partial)
    };
    let lower = partial.to_lowercase();
    let matches: Vec<String> = subfolders(&dir, partial.starts_with('.'))
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .collect();
    if matches.is_empty() {
        return None;
    }
    let mut prefix = matches[0].clone();
    for m in &matches[1..] {
        while !m.to_lowercase().starts_with(&prefix.to_lowercase()) {
            prefix.pop();
        }
    }
    let mut result = dir.join(&prefix).to_string_lossy().to_string();
    if matches.len() == 1 {
        result.push(std::path::MAIN_SEPARATOR);
    }
    Some(contract_tilde(&result))
}

pub fn expand_tilde(s: &str) -> String {
    match s.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with(std::path::MAIN_SEPARATOR) => {
            format!("{}{}", home().to_string_lossy(), rest)
        }
        _ => s.to_string(),
    }
}

fn tilde_form(s: &str) -> Option<String> {
    let home = home();
    let h = home.to_string_lossy();
    let h = h.trim_end_matches(std::path::MAIN_SEPARATOR);
    match s.strip_prefix(h) {
        Some(rest) if !h.is_empty() && (rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR)) => Some(format!("~{rest}")),
        _ => None,
    }
}

pub fn contract_tilde(s: &str) -> String {
    tilde_form(s).unwrap_or_else(|| s.to_string())
}

pub fn contract_tilde_path(p: &Path) -> Cow<'_, str> {
    let s = p.to_string_lossy();
    match tilde_form(&s) {
        Some(t) => Cow::Owned(t),
        None => s,
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = if secs.is_finite() && secs > 0.0 { secs as u64 } else { 0 };
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
