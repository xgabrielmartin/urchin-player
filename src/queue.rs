use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::m3u::Playlist;

pub struct View {
    pub selected: usize,
    pub offset: usize,
    pub marks: crate::marks::Marks,
}

impl View {
    pub fn open(playing: Option<usize>, len: usize, height: usize) -> View {
        let selected = playing.unwrap_or(0).min(len.saturating_sub(1));
        let offset = selected.saturating_sub(height.max(1) / 3);
        let mut v = View { selected, offset, marks: Default::default() };
        v.clamp(len, height);
        v
    }

    pub fn select(&mut self, i: usize, len: usize, height: usize) {
        self.selected = i.min(len.saturating_sub(1));
        self.clamp(len, height);
    }

    pub fn step(&mut self, delta: isize, len: usize, height: usize) {
        let to = (self.selected as isize).saturating_add(delta).max(0) as usize;
        self.select(to, len, height);
    }

    pub fn clamp(&mut self, len: usize, height: usize) {
        let h = height.max(1);
        self.selected = self.selected.min(len.saturating_sub(1));
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        self.offset = self.offset.min(len.saturating_sub(h));
    }
}

pub fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut s = seed;
    for i in (1..items.len()).rev() {
        let j = ((splitmix(&mut s) >> 32) * (i as u64 + 1)) >> 32;
        items.swap(i, j as usize);
    }
}

fn splitmix(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn clock_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(nanos);
    h.finish()
}

pub struct Keeper {
    file: PathBuf,
    saved: Arc<Vec<PathBuf>>,
    seen: Arc<Vec<PathBuf>>,
    seen_at: Instant,
}

impl Keeper {
    pub fn default_file() -> PathBuf {
        crate::settings::config_dir().join("queue.m3u8")
    }

    pub fn open(file: PathBuf) -> (Keeper, Vec<PathBuf>) {
        let songs = Playlist::open(&file).map(|p| p.tracks(crate::fsx::is_audio).paths).unwrap_or_default();
        let saved = Arc::new(songs.clone());
        (Keeper { file, seen: saved.clone(), saved, seen_at: Instant::now() }, songs)
    }

    pub fn keep_settled(&mut self, queue: &Arc<Vec<PathBuf>>, now: Instant) {
        if !Arc::ptr_eq(&self.seen, queue) {
            if *self.seen != **queue {
                self.seen_at = now;
            }
            self.seen = queue.clone();
        }
        if now.saturating_duration_since(self.seen_at) >= crate::order::SETTLE {
            self.keep(queue);
        }
    }

    pub fn keep(&mut self, queue: &Arc<Vec<PathBuf>>) {
        if Arc::ptr_eq(&self.saved, queue) || *self.saved == **queue {
            return;
        }
        self.saved = queue.clone();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = Playlist::from_paths(queue).save_absolute(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_keeps_every_item_once_and_depends_on_the_seed() {
        let mix = |n: usize, seed| {
            let mut v: Vec<usize> = (0..n).collect();
            shuffle(&mut v, seed);
            v
        };
        let a = mix(20, 7);
        assert_eq!(a, mix(20, 7));
        assert_ne!(a, mix(20, 8));
        assert_ne!(a, (0..20).collect::<Vec<_>>());
        let mut sorted = a;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert!(mix(0, 1).is_empty());
        assert_eq!(mix(1, 1), [0]);
    }

    #[test]
    fn the_keeper_writes_only_changes_and_reads_back_what_can_still_be_played() {
        let dir = std::env::temp_dir().join(format!("urchin_keeper_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("deeper").join("queue.m3u8");
        let song = dir.join("a.wav");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&song, b"RIFF").unwrap();

        let (mut keeper, songs) = Keeper::open(file.clone());
        assert!(songs.is_empty());
        keeper.keep(&Arc::new(Vec::new()));
        assert!(!file.exists(), "nothing changed, nothing written");
        keeper.keep(&Arc::new(vec![song.clone(), dir.join("gone.wav")]));
        assert!(file.is_file());
        assert_eq!(Keeper::open(file.clone()).1, std::slice::from_ref(&song), "a song that is gone is left out");
        keeper.keep(&Arc::new(Vec::new()));
        assert!(Keeper::open(file).1.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_burst_of_queue_changes_is_written_once_it_settles() {
        let dir = std::env::temp_dir().join(format!("urchin_keeper_settle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("queue.m3u8");
        let songs: Vec<PathBuf> = (0..5).map(|i| dir.join(format!("{i}.wav"))).collect();
        for s in &songs {
            std::fs::write(s, b"RIFF").unwrap();
        }
        let (mut keeper, _) = Keeper::open(file.clone());
        let t0 = Instant::now();
        for n in 1..=songs.len() {
            let q = Arc::new(songs[..n].to_vec());
            keeper.keep_settled(&q, t0 + std::time::Duration::from_millis(30 * n as u64));
        }
        assert!(!file.exists(), "still changing: nothing written yet");
        let all = Arc::new(songs.clone());
        keeper.keep_settled(&all, t0 + crate::order::SETTLE);
        assert!(!file.exists(), "the last change was under a second ago");
        keeper.keep_settled(&all, t0 + crate::order::SETTLE * 2);
        assert_eq!(Keeper::open(file).1, songs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opens_on_the_song_playing_with_some_of_the_past_above_it() {
        let v = View::open(Some(50), 200, 12);
        assert_eq!(v.selected, 50);
        assert_eq!(v.offset, 46);
        let v = View::open(None, 200, 12);
        assert_eq!((v.selected, v.offset), (0, 0));
        let v = View::open(Some(3), 5, 12);
        assert_eq!((v.selected, v.offset), (3, 0));
        let v = View::open(Some(3), 0, 12);
        assert_eq!((v.selected, v.offset), (0, 0));
    }

    #[test]
    fn stepping_stays_inside_the_list_and_on_screen() {
        let mut v = View::open(Some(0), 30, 10);
        v.step(-5, 30, 10);
        assert_eq!(v.selected, 0);
        v.step(12, 30, 10);
        assert_eq!((v.selected, v.offset), (12, 3));
        v.step(1 << 30, 30, 10);
        assert_eq!((v.selected, v.offset), (29, 20));
        v.step(-(1 << 30), 30, 10);
        assert_eq!((v.selected, v.offset), (0, 0));
    }

    #[test]
    fn shrinking_the_list_pulls_the_highlight_back() {
        let mut v = View::open(Some(29), 30, 10);
        assert_eq!(v.offset, 20);
        v.clamp(12, 10);
        assert_eq!((v.selected, v.offset), (11, 2));
        v.clamp(0, 10);
        assert_eq!((v.selected, v.offset), (0, 0));
    }
}
