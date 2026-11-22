use super::*;

impl App {
    pub fn restore_queue(&mut self) {
        let (keeper, songs) = queue::Keeper::open(queue::Keeper::default_file());
        self.kept = Some(keeper);
        if !songs.is_empty() {
            self.follow = shared_folder(&songs).filter(|d| d.starts_with(&self.root) && *d != self.root).map(|d| (d, false));
            match self.settings.resume.as_ref().and_then(|r| resume_place(&songs, r)) {
                Some((index, seconds)) => self.engine.load_queue_at(songs, index, seconds),
                None => self.engine.load_queue(songs),
            }
            self.settle_engine();
        }
    }

    pub(super) fn copy_path(&mut self, path: &Path, why: &str) {
        let text = path.to_string_lossy();
        self.osc_out.push(crate::reveal::clipboard_escape(&text));
        self.say(format!("{why}: {}", fsx::contract_tilde(&text)));
    }

    pub(super) fn open_about_link(&mut self) {
        self.overlay = None;
        let url = crate::ui::ABOUT_URL;
        if crate::reveal::open_url(url) {
            self.say("opening github.com/xgabrielmartin");
        } else {
            self.osc_out.push(crate::reveal::clipboard_escape(url));
            self.say(format!("couldn't open a browser; copied {url}"));
        }
    }

    pub(super) fn reveal_results(&mut self) {
        while let Ok((path, shown)) = self.reveals.1.try_recv() {
            use crate::reveal::Shown;
            let name = short_name(&path);
            match shown {
                Shown::Selected => {
                    let place = match crate::reveal::label() {
                        "Show in Finder" => "Finder",
                        "Show in Explorer" => "Explorer",
                        _ => "your file manager",
                    };
                    self.say(format!("showing {name} in {place}"));
                }
                Shown::Folder => {
                    let folder = path.parent().map(short_name).unwrap_or_default();
                    self.say(format!("opened {folder} (your file manager can't highlight {name})"));
                }
                Shown::NoDesktop => self.copy_path(&path, "no desktop here to show it on; copied the path"),
                Shown::Failed => self.copy_path(&path, "couldn't open a file manager; copied the path"),
            }
        }
    }

    pub(crate) fn continue_into_next_folder(&mut self) {
        if !self.settings.continue_folders || self.settings.repeat != Repeat::Off || self.status.state != State::Playing {
            return;
        }
        let Some((from, shuffled)) = self.follow.clone() else { return };
        let len = self.status.queue.len();
        if self.status.playing_index() != len.checked_sub(1) {
            return;
        }
        let show_hidden = self.settings.show_hidden;
        let found = next_folder(&from, &self.root, show_hidden, |dir| fsx::collect_tracks_in(dir, &self.order));
        let Some((next, mut songs)) = found else {
            self.follow = None;
            return;
        };
        if shuffled {
            self.mix(&mut songs);
        }
        let n = songs.len();
        self.engine.enqueue(songs, false);
        self.follow = Some((next.clone(), shuffled));
        self.say(format!("next up: {} ({})", short_name(&next), plural_songs(n)));
    }

    pub(super) fn track_left_off(&mut self) {
        if self.kept.is_none() {
            return;
        }
        if let Some(song) = self.status.current() {
            let point = crate::settings::Resume { song: song.clone(), index: self.status.index, seconds: self.status.position };
            self.left_off = Some((point, self.status.duration));
        }
        let now = (self.status.current().cloned(), self.status.state);
        match &self.left_off_saved {
            None => self.left_off_saved = Some(now),
            Some(was) if *was != now => {
                self.left_off_saved = Some(now);
                self.save_left_off();
            }
            _ => {}
        }
    }

    pub(super) fn save_left_off(&mut self) {
        let point = self.left_off.clone().and_then(|(mut p, duration)| {
            if duration > 0.0 && p.seconds >= duration - (duration / 10.0).min(1.0) {
                let next = self.status.queue.get(p.index + 1)?.clone();
                p = crate::settings::Resume { song: next, index: p.index + 1, seconds: 0.0 };
            }
            p.seconds = (p.seconds * 10.0).round() / 10.0;
            Some(p)
        });
        if self.settings.resume != point {
            self.settings.resume = point;
            self.settings.save();
        }
    }

    pub fn restore_order(&mut self) {
        self.order = Order::open(Order::default_file());
        self.refresh();
    }

    pub fn use_tag_cache(&mut self) {
        self.tag_cache = Some(library::cache_file());
    }

    pub fn follow_moved_files(&mut self) {
        self.moves = Some(Moves::new());
        let store = self.eq_store.clone();
        if store.lock().unwrap().stale_prints() != (Vec::new(), Vec::new()) {
            let _ = std::thread::Builder::new().name("urchin-reprint".into()).spawn(move || refresh_stale_prints(&store));
        }
    }

    pub(crate) fn follow_moves(&mut self) {
        let Some(m) = self.moves.as_mut() else { return };
        m.collect();
        let Some(song) = self.status.current().cloned() else { return };
        let Some(dir) = song.parent().map(Path::to_path_buf) else { return };
        let store = self.eq_store.clone();

        if m.song.as_ref() != Some(&song) {
            m.song = Some(song.clone());
            m.found_song = false;
            let s = store.lock().unwrap();
            let no_preset = s.folder_preset_for(&song).is_none();
            m.find_song = !s.has(&song) && s.has_lost_songs();
            let find_folder = no_preset && s.has_lost_folders();
            if find_folder || s.wants_folder_prints(&dir) {
                m.folder = Some((dir.clone(), folder_print_songs(&dir), find_folder));
            }
        }

        let (wants, find) = {
            let s = store.lock().unwrap();
            (s.wants_print(&song), m.find_song && !s.has(&song))
        };
        if wants || find {
            match m.get(&song).map(|p| p.cloned()) {
                None => m.ask(std::slice::from_ref(&song)),
                Some(None) => m.find_song = false,
                Some(Some(print)) => {
                    m.find_song = false;
                    let mut s = store.lock().unwrap();
                    if wants {
                        s.set_print(&song, print);
                    } else if s.relink_song(&song, &print) {
                        s.save_if_changed();
                        drop(s);
                        m.found_song = true;
                        self.engine.refresh_curves();
                        self.say("found this song's EQ again: the file was moved or renamed");
                    }
                }
            }
        }

        let Some(m) = self.moves.as_mut() else { return };
        let Some((dir, songs, find)) = m.folder.clone() else { return };
        let found_song = m.found_song;
        let missing: Vec<PathBuf> = songs.iter().filter(|p| m.get(p).is_none()).cloned().collect();
        if !missing.is_empty() {
            m.ask(&missing);
            return;
        }
        m.folder = None;
        let prints: Vec<String> = songs.iter().filter_map(|p| m.get(p).flatten().cloned()).collect();
        let mut s = store.lock().unwrap();
        if s.wants_folder_prints(&dir) {
            s.set_folder_prints(&dir, prints);
        } else if find {
            if let Some(id) = s.relink_folder(&dir, &prints) {
                s.save_if_changed();
                let name = s.preset_name(id);
                drop(s);
                self.engine.refresh_curves();
                self.say(if found_song {
                    format!("found this song's EQ and {}'s preset ({name}) again: they were moved or renamed", short_name(&dir))
                } else {
                    format!("found {}'s preset ({name}) again: the folder was moved or renamed", short_name(&dir))
                });
            }
        }
    }

    pub(super) fn album_keys(&self, songs: &[PathBuf]) -> Vec<albums::Key> {
        albums::keys(songs, self.tag_cache.as_deref())
    }

    pub(super) fn mix(&self, songs: &mut Vec<PathBuf>) {
        if self.settings.shuffle_albums {
            let keys = self.album_keys(songs);
            albums::shuffle(songs, &keys, queue::clock_seed());
        } else {
            queue::shuffle(songs, queue::clock_seed());
        }
    }

    pub fn next_album_index(&self) -> Result<usize, &'static str> {
        let Some(at) = self.status.playing_index() else { return Err("nothing is playing") };
        let queue = &self.status.queue;
        let Some(here) = queue.get(at) else { return Err("nothing is playing") };
        let here = self.album_keys(std::slice::from_ref(here)).remove(0);
        let next = (at + 1..queue.len()).find(|&i| self.album_keys(std::slice::from_ref(&queue[i]))[0] != here);
        match next {
            Some(i) => Ok(i),
            None if self.settings.repeat == Repeat::All => Ok(0),
            None => Err("this is the last album in the queue"),
        }
    }

    pub(super) fn next_album(&mut self) {
        match self.next_album_index() {
            Ok(to) => {
                let song = self.status.queue[to].clone();
                self.engine.jump(to, song);
                self.settle_engine();
            }
            Err(why) => self.say(why),
        }
    }

    pub(super) fn save_settled(&mut self) {
        let now = Instant::now();
        if let Some(kept) = self.kept.as_mut() {
            kept.keep_settled(&self.status.queue, now);
        }
        self.order.save_settled(now);
    }

    pub fn save_waiting(&mut self) {
        if let Some(kept) = self.kept.as_mut() {
            kept.keep(&self.status.queue);
        }
        self.order.save_now();
    }

    pub fn enable_media_controls(&mut self) {
        self.media = crate::mediakeys::Media::start();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn poll_media(&mut self) {
        use crate::mediakeys::{MediaControlEvent as E, SeekDirection};
        let Some(media) = self.media.as_mut() else { return };
        let events = media.events();
        media.update(&self.status);
        for e in events {
            let playing = self.status.state == State::Playing;
            crate::trace::tlog!("media key: {:?} (player {:?})", e, self.status.state);
            match e {
                E::Toggle => self.media_play_pause(),
                E::Play if !playing => self.media_play_pause(),
                E::Pause if playing => self.media_play_pause(),
                E::Next => self.run(Action::Next),
                E::Previous => self.run(Action::Previous),
                E::Stop => self.run(Action::Stop),
                E::SeekBy(dir, d) => {
                    let s = d.as_secs_f64();
                    self.run(Action::SeekBy(if matches!(dir, SeekDirection::Forward) { s } else { -s }));
                }
                E::SetPosition(pos) => {
                    let to = pos.0.as_secs_f64();
                    self.run(Action::SeekBy(to - self.status.position));
                }
                _ => {}
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn media_play_pause(&mut self) {
        let now = self.engine.status();
        if now.state == State::Stopped && self.engine.caught_up(&now) {
            self.run(Action::PlayPause);
        } else {
            self.engine.media_toggle_pause();
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn poll_media(&mut self) {}
}
