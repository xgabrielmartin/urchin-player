use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fsx::{Entry, Kind};

pub const SETTLE: Duration = Duration::from_millis(1000);

#[derive(Default)]
pub struct Order {
    file: Option<PathBuf>,
    folders: BTreeMap<String, Vec<String>>,
    unsaved: Option<Instant>,
}

fn key(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

impl Order {
    pub fn empty() -> Order {
        Order::default()
    }

    pub fn default_file() -> PathBuf {
        crate::settings::config_dir().join("folder_order.json")
    }

    pub fn open(file: PathBuf) -> Order {
        let folders = fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Order { file: Some(file), folders, unsaved: None }
    }

    pub fn is_custom(&self, dir: &Path) -> bool {
        self.folders.contains_key(&key(dir))
    }

    pub fn apply(&self, dir: &Path, entries: &mut [Entry]) {
        let Some(names) = self.folders.get(&key(dir)) else { return };
        let at: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let first = entries.iter().position(|e| e.kind == Kind::Track).unwrap_or(entries.len());
        entries[first..].sort_by_key(|e| at.get(e.name.as_str()).copied().unwrap_or(usize::MAX));
    }

    pub fn set(&mut self, dir: &Path, names: Vec<String>) {
        if names.windows(2).all(|w| natord::compare_ignore_case(&w[0], &w[1]).is_le()) {
            self.reset(dir);
            return;
        }
        if self.folders.get(&key(dir)) != Some(&names) {
            self.folders.insert(key(dir), names);
            self.changed();
        }
    }

    pub fn reset(&mut self, dir: &Path) -> bool {
        let had = self.folders.remove(&key(dir)).is_some();
        if had {
            self.changed();
        }
        had
    }

    fn changed(&mut self) {
        if self.file.is_some() {
            self.unsaved = Some(Instant::now());
        }
    }

    pub fn save_settled(&mut self, now: Instant) {
        if self.unsaved.is_some_and(|t| now.duration_since(t) >= SETTLE) {
            self.save_now();
        }
    }

    pub fn save_now(&mut self) {
        if self.unsaved.take().is_some() {
            self.save();
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let Ok(text) = serde_json::to_string_pretty(&self.folders) else { return };
        let _ = crate::fsx::save_if_different(file, text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(name: &str) -> Entry {
        Entry { name: name.to_string(), path: PathBuf::from("/m").join(name), kind: Kind::Track }
    }

    fn folder(name: &str) -> Entry {
        Entry { name: name.to_string(), path: PathBuf::from("/m").join(name), kind: Kind::Folder }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn a_saved_order_moves_only_the_songs_and_unlisted_songs_go_last() {
        let dir = Path::new("/m");
        let mut order = Order::empty();
        order.set(dir, vec!["c.mp3".into(), "a.mp3".into(), "gone.mp3".into()]);
        let mut e = vec![folder("x"), folder("y"), song("a.mp3"), song("b.mp3"), song("c.mp3"), song("d.mp3")];
        order.apply(dir, &mut e);
        assert_eq!(names(&e), ["x", "y", "c.mp3", "a.mp3", "b.mp3", "d.mp3"]);
    }

    #[test]
    fn a_folder_without_an_order_is_left_alone() {
        let mut order = Order::empty();
        order.set(Path::new("/other"), vec!["b.mp3".into(), "a.mp3".into()]);
        let mut e = vec![song("a.mp3"), song("b.mp3")];
        order.apply(Path::new("/m"), &mut e);
        assert_eq!(names(&e), ["a.mp3", "b.mp3"]);
        assert!(!order.is_custom(Path::new("/m")));
        assert!(order.is_custom(Path::new("/other")));
    }

    #[test]
    fn orders_survive_a_restart_and_reset_forgets_them() {
        let dir = std::env::temp_dir().join(format!("urchin_order_{}", std::process::id()));
        let file = dir.join("folder_order.json");
        let _ = fs::remove_dir_all(&dir);
        let mut order = Order::open(file.clone());
        order.set(Path::new("/m/rock"), vec!["b.mp3".into(), "a.mp3".into()]);
        order.set(Path::new("/m/jazz"), vec!["z.mp3".into(), "y.mp3".into()]);
        assert!(!file.exists(), "nothing is written while the order is still moving");
        order.save_now();

        let mut again = Order::open(file.clone());
        let mut e = vec![song("a.mp3"), song("b.mp3")];
        again.apply(Path::new("/m/rock"), &mut e);
        assert_eq!(names(&e), ["b.mp3", "a.mp3"]);

        assert!(again.reset(Path::new("/m/rock")));
        assert!(!again.reset(Path::new("/m/rock")));
        again.save_now();
        let last = Order::open(file);
        assert!(!last.is_custom(Path::new("/m/rock")));
        assert!(last.is_custom(Path::new("/m/jazz")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_order_that_is_just_name_order_is_not_kept() {
        let dir = Path::new("/m");
        let mut order = Order::empty();
        order.set(dir, vec!["2 - b.mp3".into(), "1 - a.mp3".into()]);
        assert!(order.is_custom(dir));
        order.set(dir, vec!["2 - b.mp3".into(), "10 - a.mp3".into()]);
        assert!(!order.is_custom(dir));
    }

    #[test]
    fn a_burst_of_moves_is_written_once_after_it_settles() {
        let dir = std::env::temp_dir().join(format!("urchin_order_settle_{}", std::process::id()));
        let file = dir.join("folder_order.json");
        let _ = fs::remove_dir_all(&dir);
        let mut order = Order::open(file.clone());
        let folder = Path::new("/m/rock");
        let start = Instant::now();
        for _ in 0..5 {
            order.set(folder, vec!["c.mp3".into(), "a.mp3".into(), "b.mp3".into()]);
            order.set(folder, vec!["c.mp3".into(), "b.mp3".into(), "a.mp3".into()]);
            order.save_settled(start);
        }
        assert!(!file.exists(), "still moving: nothing written yet");
        order.save_settled(Instant::now() + SETTLE);
        let again = Order::open(file.clone());
        let mut e = vec![song("a.mp3"), song("b.mp3"), song("c.mp3")];
        again.apply(folder, &mut e);
        assert_eq!(names(&e), ["c.mp3", "b.mp3", "a.mp3"], "the last order is the one written");
        let written = fs::metadata(&file).unwrap().modified().unwrap();
        order.save_settled(Instant::now() + SETTLE * 3);
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), written);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_broken_file_is_no_orders() {
        let dir = std::env::temp_dir().join(format!("urchin_order_bad_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("folder_order.json");
        fs::write(&file, "not json {").unwrap();
        assert!(!Order::open(file).is_custom(Path::new("/m")));
        assert!(!Order::open(dir.join("nope.json")).is_custom(Path::new("/m")));
        let _ = fs::remove_dir_all(&dir);
    }
}
