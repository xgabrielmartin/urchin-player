use super::*;

impl App {
    pub(super) fn show_theme_picker(&mut self) {
        let list: Vec<usize> = (0..THEMES.len()).collect();
        let selected = self.theme;
        let mut p = ThemePicker { filter: String::new(), list, selected, offset: 0, original: self.theme };
        p.offset = selected.saturating_sub(self.list_height / 2);
        self.overlay = Some(Overlay::Theme(p));
    }

    pub(super) fn theme_filter(&mut self) {
        let Some(Overlay::Theme(p)) = self.overlay.as_mut() else { return };
        let f = p.filter.to_lowercase();
        let current = self.theme;
        p.list = (0..THEMES.len()).filter(|&i| THEMES[i].name.to_lowercase().contains(&f)).collect();
        p.selected = p.list.iter().position(|&i| i == current).unwrap_or(0);
        p.offset = 0;
        self.theme_preview();
    }

    pub(super) fn theme_select(&mut self, row: usize) {
        let h = self.list_height.max(1);
        if let Some(Overlay::Theme(p)) = self.overlay.as_mut() {
            if p.list.is_empty() {
                return;
            }
            p.selected = row.min(p.list.len() - 1);
            if p.selected < p.offset {
                p.offset = p.selected;
            } else if p.selected >= p.offset + h {
                p.offset = p.selected + 1 - h;
            }
        }
        self.theme_preview();
    }

    pub(super) fn theme_preview(&mut self) {
        if let Some(Overlay::Theme(p)) = &self.overlay {
            if let Some(&i) = p.list.get(p.selected) {
                self.theme = i;
            }
        }
    }

    pub(super) fn theme_finish(&mut self, keep: bool) {
        if let Some(Overlay::Theme(p)) = self.overlay.take() {
            if keep {
                self.settings.theme = THEMES[self.theme].name.to_string();
                self.settings.save();
                self.say(format!("theme: {}", THEMES[self.theme].name));
            } else {
                self.theme = p.original;
            }
        }
    }

    pub(super) fn show_search(&mut self) {
        self.key_focus = None;
        let tags = self.library.as_ref().and_then(|v| v.index().map(|ix| (v.root().to_path_buf(), ix.clone())));
        self.overlay = Some(Overlay::Search(match tags {
            Some((root, ix)) => Search::from_library(root, &ix),
            None => Search::open(self.root.clone()),
        }));
    }

    pub(super) fn search_mut(&mut self) -> Option<&mut Search> {
        match self.overlay.as_mut() {
            Some(Overlay::Search(s)) => Some(s),
            _ => None,
        }
    }

    pub(super) fn search_select(&mut self, row: usize) {
        let h = self.list_height;
        if let Some(s) = self.search_mut() {
            s.select(row, h);
        }
    }

    pub(super) fn search_step(&mut self, delta: isize) {
        let h = self.list_height;
        if let Some(s) = self.search_mut() {
            s.step(delta, h);
        }
    }

    pub(super) fn search_key(&mut self, k: KeyEvent) {
        const FAR: isize = 1 << 30;
        let h = self.list_height.max(1) as isize;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char('x' | 'X') if ctrl && !alt => {
                if let Some(row) = self.active_row() {
                    self.set_tick(row, None);
                    self.step_active();
                }
            }
            KeyCode::Esc if self.clear_ticks() => {}
            KeyCode::Esc => self.overlay = None,
            KeyCode::Enter => self.run(Action::SearchPlay),
            KeyCode::Right | KeyCode::Menu => self.search_menu(None),
            KeyCode::F(10) if k.modifiers.contains(KeyModifiers::SHIFT) => self.search_menu(None),
            KeyCode::Up => self.search_step(-1),
            KeyCode::Down => self.search_step(1),
            KeyCode::PageUp => self.search_step(-h),
            KeyCode::PageDown => self.search_step(h),
            KeyCode::Home => self.search_step(-FAR),
            KeyCode::End => self.search_step(FAR),
            KeyCode::Backspace => {
                if let Some(s) = self.search_mut() {
                    s.backspace();
                }
            }
            KeyCode::Char(c) if (!ctrl || alt) && !c.is_control() => {
                if let Some(s) = self.search_mut() {
                    s.type_char(c);
                }
            }
            _ => {}
        }
    }

    pub(super) fn search_menu(&mut self, at: Option<Anchor>) {
        let ticked = self.ticked_menu();
        let Some(Overlay::Search(s)) = &self.overlay else { return };
        let Some(song) = s.highlighted() else { return };
        let items = if let Some(items) = ticked { items } else { vec![
            MenuItem::new("Play", "Enter", Action::SearchPlay),
            MenuItem::new("Play Next", "", Action::SearchPlayNext),
            MenuItem::new("Add to Queue", "", Action::SearchAddToQueue),
            MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(song.path.clone())),
            MenuItem::new("Show in Folder", "", Action::SearchShow),
            file_items(&song.path)[0].clone(),
            file_items(&song.path)[1].clone(),
        ] };
        let row = Target::SearchRow(s.selected);
        let (anchor, hover) = match at {
            Some(anchor) => (anchor, None),
            None => (self.row_anchor(row), Some(0)),
        };
        self.menu = Some(OpenMenu { hover, ..OpenMenu::new(items, anchor) });
    }

    pub(super) fn search_play(&mut self) {
        let Some(Overlay::Search(s)) = &self.overlay else { return };
        let ticked = s.ticked_paths();
        let Some((songs, from)) = (if ticked.is_empty() { s.playlist() } else { Some((ticked, 0)) }) else {
            self.say("there is nothing to play yet");
            return;
        };
        self.overlay = None;
        self.solo_for = None;
        self.follow = None;
        self.engine.play(songs, from);
    }

    pub(super) fn search_enqueue(&mut self, next: bool) {
        let Some(Overlay::Search(s)) = &self.overlay else { return };
        let ticked = s.ticked_paths();
        if !ticked.is_empty() {
            let name = fsx::title_from_path(&ticked[0]);
            self.enqueue_paths(ticked, name, next);
            return;
        }
        let Some(song) = s.highlighted() else { return };
        let (path, name) = (song.path.clone(), song.title().into_owned());
        self.enqueue_paths(vec![path], name, next);
    }

    pub(super) fn search_show(&mut self) {
        let Some(Overlay::Search(s)) = &self.overlay else { return };
        let Some(song) = s.highlighted().map(|x| x.path.clone()) else { return };
        self.overlay = None;
        self.show_in_folder(song);
    }

    pub(super) fn show_in_folder(&mut self, song: PathBuf) {
        let Some(folder) = song.parent().map(Path::to_path_buf) else { return };
        self.playlists = None;
        self.queue_view = None;
        self.library = None;
        self.solo = false;
        self.key_focus = None;
        self.back.push(self.current_dir());
        self.forward.clear();
        self.navigate_to(&folder);
        if let Some(i) = self.levels[self.focus].entries.iter().position(|e| e.path == song) {
            self.select(self.focus, i);
        }
    }

    pub(super) fn show_open(&mut self, mode: OpenMode) {
        let start = if mode == OpenMode::MusicFolder { self.root.clone() } else { self.current_dir() };
        let mut places: Vec<(String, PathBuf)> = Vec::new();
        for p in &self.settings.pinned {
            places.push((format!("☆ {}", short_name(p)), p.clone()));
        }
        for p in self.settings.recent.iter().take(8) {
            places.push((format!("⟲ {}", short_name(p)), p.clone()));
        }
        places.push(("⌂ Home".into(), fsx::home()));
        places.push(("♪ Music".into(), fsx::default_music_dir()));
        for d in fsx::drives() {
            places.push((format!("⏏ {}", short_name(&d)), d));
        }
        let mut d = OpenDialog {
            mode,
            path: String::new(),
            dir: start.clone(),
            entries: Vec::new(),
            selected: 0,
            offset: 0,
            places,
            editing_path: false,
            checked: Vec::new(),
        };
        load_dialog_dir(&mut d, &start, self.settings.show_hidden);
        self.overlay = Some(Overlay::Open(d));
    }

    pub(super) fn open_dialog_go(&mut self, dir: PathBuf) {
        let show_hidden = self.settings.show_hidden;
        if let Some(Overlay::Open(d)) = self.overlay.as_mut() {
            if dir.is_dir() {
                load_dialog_dir(d, &dir, show_hidden);
            }
        }
    }

    pub(super) fn open_dialog_select(&mut self, row: usize) {
        let h = self.list_height.max(1);
        if let Some(Overlay::Open(d)) = self.overlay.as_mut() {
            let len = d.entries.len() + 1;
            d.selected = row.min(len.saturating_sub(1));
            if d.selected < d.offset {
                d.offset = d.selected;
            } else if d.selected >= d.offset + h {
                d.offset = d.selected + 1 - h;
            }
        }
    }

    pub(super) fn open_dialog_enter(&mut self) {
        let target = match &self.overlay {
            Some(Overlay::Open(d)) => {
                if d.selected == 0 {
                    d.dir.parent().map(|p| p.to_path_buf())
                } else {
                    d.entries.get(d.selected - 1).map(|e| e.path.clone())
                }
            }
            _ => None,
        };
        if let Some(t) = target {
            self.open_dialog_go(t);
        }
    }

    pub(super) fn import_toggle(&mut self, row: Option<usize>) {
        let Some(Overlay::Open(d)) = self.overlay.as_mut() else { return };
        let row = row.unwrap_or(d.selected);
        let Some(e) = row.checked_sub(1).and_then(|k| d.entries.get(k)) else { return };
        if e.kind != Kind::Track {
            return;
        }
        let path = e.path.clone();
        match d.checked.iter().position(|p| *p == path) {
            Some(at) => {
                d.checked.remove(at);
            }
            None => d.checked.push(path),
        }
    }

    pub(super) fn import_dialog_enter(&mut self) {
        let Some(Overlay::Open(d)) = &self.overlay else { return };
        if !d.checked.is_empty() {
            let files = d.checked.clone();
            self.import_playlists(files);
            return;
        }
        match d.selected.checked_sub(1).and_then(|k| d.entries.get(k)).cloned() {
            Some(e) if e.kind == Kind::Track => self.import_playlists(vec![e.path]),
            _ => self.open_dialog_enter(),
        }
    }

    pub(super) fn import_playlists(&mut self, files: Vec<PathBuf>) {
        let library = playlists::dir();
        let mut done = Vec::new();
        let mut failed = Vec::new();
        for f in &files {
            match playlists::import_into(f, &library) {
                Ok(i) => done.push(i),
                Err(e) => failed.push(format!("{}: {e}", short_name(f))),
            }
        }
        self.overlay = None;
        let Some(first) = done.first() else {
            self.say(format!("couldn't import {}", failed.join("; ")));
            return;
        };
        let mut msg = if done.len() == 1 {
            let i = &done[0];
            let mut m = format!("imported {} ({} song{}", playlists::name_of(&i.path), i.songs, if i.songs == 1 { "" } else { "s" });
            if i.skipped > 0 {
                m.push_str(&format!(", {} can't be played", i.skipped));
            }
            m.push(')');
            m
        } else {
            format!("imported {} playlists", done.len())
        };
        if !failed.is_empty() {
            msg.push_str(&format!(" · couldn't import {}", failed.join("; ")));
        }
        let select = first.path.clone();
        self.open_playlists(Some(select));
        self.say(msg);
    }

    pub(super) fn submit_playlist_name(&mut self) {
        let Some(Overlay::PlaylistName(name, what)) = &self.overlay else { return };
        if what.is_eq() {
            let (name, what) = (name.clone(), what.clone());
            return self.submit_preset_name(name, what);
        }
        if name.trim().is_empty() {
            self.say("type a name for the playlist");
            return;
        }
        let (name, what) = (name.clone(), what.clone());
        match what {
            NameFor::Queue => {
                let songs = self.status.queue.clone();
                match playlists::save_songs_as(&name, &playlists::dir(), &songs) {
                    Ok(path) => {
                        self.overlay = None;
                        let made = playlists::name_of(&path);
                        self.say(format!("saved the queue ({}) as playlist {made}", plural_songs(songs.len())));
                    }
                    Err(e) => self.say(format!("couldn't save the queue: {e}")),
                }
            }
            NameFor::New => match playlists::create_in(&name, &playlists::dir()) {
                Ok(path) => {
                    self.overlay = None;
                    let made = playlists::name_of(&path);
                    self.open_playlists(Some(path));
                    self.say(format!("made playlist {made}"));
                }
                Err(e) => self.say(format!("couldn't make the playlist: {e}")),
            },
            NameFor::EqPreset | NameFor::EqRename(_) => {}
            NameFor::Rename(old) => match playlists::rename(&old, &name) {
                Ok(path) => {
                    self.overlay = None;
                    let now = playlists::name_of(&path);
                    self.open_playlists(Some(path));
                    self.say(format!("renamed {} to {now}", playlists::name_of(&old)));
                }
                Err(e) => self.say(format!("couldn't rename: {e}")),
            },
        }
    }

    pub(super) fn open_dialog_button(&mut self, b: OpenButton) {
        let Some(Overlay::Open(d)) = &self.overlay else { return };
        let (mode, dir) = (d.mode, d.dir.clone());
        let chosen = if d.selected > 0 { d.entries.get(d.selected - 1).map_or(dir, |e| e.path.clone()) } else { dir };
        match b {
            OpenButton::Cancel => self.overlay = None,
            OpenButton::Import => self.import_dialog_enter(),
            OpenButton::Open => {
                self.overlay = None;
                if mode == OpenMode::MusicFolder {
                    self.settings.music_root = Some(chosen.clone());
                    self.settings.save();
                    self.set_root(chosen);
                    self.say("music folder set");
                } else {
                    self.open_folder(&chosen);
                }
            }
            OpenButton::Play | OpenButton::Shuffle => {
                self.overlay = None;
                self.open_folder(&chosen);
                let e = Entry {
                    name: short_name(&chosen),
                    path: chosen,
                    kind: Kind::Folder,
                };
                self.play_entry(&e, self.focus, b == OpenButton::Shuffle);
            }
        }
    }

}
