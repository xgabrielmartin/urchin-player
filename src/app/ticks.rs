use super::*;

impl App {
    pub fn active_row(&self) -> Option<TickRow> {
        if let Some(Overlay::Search(s)) = &self.overlay {
            return (s.count() > 0).then_some(TickRow::Search(s.selected));
        }
        if self.overlay.is_some() {
            return None;
        }
        if let Some(v) = &self.queue_view {
            return (!self.status.queue.is_empty()).then_some(TickRow::Queue(v.selected));
        }
        if let Some(v) = &self.playlists {
            return (v.on_songs && !v.songs.is_empty()).then_some(TickRow::Playlist(v.song_selected));
        }
        if let Some(v) = &self.library {
            return (v.focus == library::SONGS && !v.songs.is_empty()).then_some(TickRow::Library(v.song_selected));
        }
        let c = self.levels.get(self.focus)?;
        c.selected.filter(|&i| i < c.entries.len()).map(|row| TickRow::Folder { col: self.focus, row })
    }

    pub(super) fn marks_of(&mut self, t: TickRow) -> Option<&mut crate::marks::Marks> {
        match t {
            TickRow::Folder { col, .. } => self.levels.get_mut(col).map(|c| &mut c.marks),
            TickRow::Queue(_) => self.queue_view.as_mut().map(|v| &mut v.marks),
            TickRow::Playlist(_) => self.playlists.as_mut().map(|v| &mut v.song_marks),
            TickRow::Library(_) => self.library.as_mut().map(|v| &mut v.song_marks),
            TickRow::Search(_) => None,
        }
    }

    pub(super) fn search_path(&self, i: usize) -> Option<PathBuf> {
        match &self.overlay {
            Some(Overlay::Search(s)) => s.result(i).map(|x| x.path.clone()),
            _ => None,
        }
    }

    pub fn is_ticked(&self, t: TickRow) -> bool {
        match t {
            TickRow::Folder { col, row } => self.levels.get(col).is_some_and(|c| c.marks.contains(&row)),
            TickRow::Queue(i) => self.queue_view.as_ref().is_some_and(|v| v.marks.contains(&i)),
            TickRow::Playlist(i) => self.playlists.as_ref().is_some_and(|v| v.song_marks.contains(&i)),
            TickRow::Library(i) => self.library.as_ref().is_some_and(|v| v.song_marks.contains(&i)),
            TickRow::Search(i) => match (&self.overlay, self.search_path(i)) {
                (Some(Overlay::Search(s)), Some(p)) => s.marks.contains(&p),
                _ => false,
            },
        }
    }

    pub(super) fn set_tick(&mut self, t: TickRow, on: Option<bool>) -> bool {
        let on = on.unwrap_or(!self.is_ticked(t));
        if let TickRow::Search(i) = t {
            if let (Some(p), Some(s)) = (self.search_path(i), self.search_mut()) {
                s.marks.set(p, on);
            }
        } else if let Some(m) = self.marks_of(t) {
            m.set(t.row(), on);
        }
        on
    }

    pub(super) fn tick_range(&mut self, a: TickRow, b: TickRow, on: bool) {
        if let TickRow::Search(_) = a {
            for i in a.row().min(b.row())..=a.row().max(b.row()) {
                self.set_tick(a.at(i), Some(on));
            }
            if let (Some(p), Some(s)) = (self.search_path(b.row()), self.search_mut()) {
                s.marks.anchor = Some(p);
            }
        } else if let Some(m) = self.marks_of(a) {
            m.set_range(a.row(), b.row(), on);
        }
    }

    pub(super) fn extend_ticks(&mut self, t: TickRow) {
        if let TickRow::Search(i) = t {
            let from = match &self.overlay {
                Some(Overlay::Search(s)) => s.marks.anchor.as_ref().and_then(|p| (0..s.count()).find(|&k| s.result(k).is_some_and(|x| x.path == *p))),
                _ => None,
            };
            self.tick_range(t.at(from.unwrap_or(i)), t, true);
        } else if let Some(m) = self.marks_of(t) {
            m.extend_to(t.row());
        }
    }

    pub fn tick_count(&self) -> usize {
        if let Some(Overlay::Search(s)) = &self.overlay {
            return s.marks.len();
        }
        if self.overlay.is_some() {
            return 0;
        }
        if let Some(v) = &self.queue_view {
            return v.marks.len();
        }
        if let Some(v) = &self.playlists {
            return if v.on_songs { v.song_marks.len() } else { 0 };
        }
        if let Some(v) = &self.library {
            return if v.focus == library::SONGS { v.song_marks.len() } else { 0 };
        }
        self.levels.get(self.focus).map_or(0, |c| c.marks.len())
    }

    pub(super) fn clear_ticks(&mut self) -> bool {
        if self.tick_count() == 0 {
            return false;
        }
        if let Some(s) = self.search_mut() {
            s.marks.clear();
        } else if let Some(v) = self.queue_view.as_mut() {
            v.marks.clear();
        } else if let Some(v) = self.playlists.as_mut() {
            v.song_marks.clear();
        } else if let Some(v) = self.library.as_mut() {
            v.song_marks.clear();
        } else if let Some(c) = self.levels.get_mut(self.focus) {
            c.marks.clear();
        }
        true
    }

    pub(super) fn ticked_songs(&self) -> Option<Vec<PathBuf>> {
        if self.tick_count() == 0 {
            return None;
        }
        if let Some(Overlay::Search(s)) = &self.overlay {
            return Some(s.ticked_paths());
        }
        if let Some(v) = &self.queue_view {
            return Some(v.marks.iter().filter_map(|&i| self.status.queue.get(i).cloned()).collect());
        }
        if let Some(v) = &self.playlists {
            return Some(v.song_marks.iter().filter_map(|&i| v.songs.get(i).cloned()).collect());
        }
        if let Some(v) = &self.library {
            let ix = v.index()?;
            return Some(v.song_marks.iter().filter_map(|&i| v.songs.get(i)).map(|&t| ix.tracks[t as usize].path.clone()).collect());
        }
        let c = self.levels.get(self.focus)?;
        let mut songs = Vec::new();
        for e in c.marks.iter().filter_map(|&i| c.entries.get(i)) {
            match e.kind {
                Kind::Track => songs.push(e.path.clone()),
                Kind::Folder => songs.extend(fsx::collect_tracks_in(&e.path, &self.order)),
            }
        }
        Some(songs)
    }

    pub(super) fn ticked_menu(&self) -> Option<Vec<MenuItem>> {
        let n = self.tick_count();
        if n == 0 {
            return None;
        }
        let any = PathBuf::new();
        let mut items = vec![MenuItem::heading(&format!("{n} ticked"))];
        if self.queue_view.is_some() && !matches!(self.overlay, Some(Overlay::Search(_))) {
            items.push(MenuItem::new("Move Up", "Shift+K", Action::QueueMoveUp));
            items.push(MenuItem::new("Move Down", "Shift+J", Action::QueueMoveDown));
            items.push(MenuItem::new("Remove", "Del", Action::QueueRemove));
            items.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(any)));
        } else if matches!(self.overlay, Some(Overlay::Search(_))) {
            items.push(MenuItem::new("Play", "Enter", Action::SearchPlay));
            items.push(MenuItem::new("Play Next", "", Action::SearchPlayNext));
            items.push(MenuItem::new("Add to Queue", "", Action::SearchAddToQueue));
            items.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(any)));
        } else {
            items.push(MenuItem::new("Play", "Enter", Action::PlaySelection));
            items.push(MenuItem::new("Shuffle", "", Action::ShuffleSelection));
            items.push(MenuItem::new("Play Next", "Shift+N", Action::PlayNext));
            items.push(MenuItem::new("Add to Queue", "Shift+Q", Action::AddToQueue));
            items.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(any)));
            let in_browser = !self.in_view();
            let c = &self.levels[self.focus];
            if in_browser && c.marks.iter().all(|&i| c.entries.get(i).is_some_and(|e| e.kind == Kind::Track)) {
                items.push(MenuItem::sep());
                items.push(MenuItem::new("Move Up", "Shift+K", Action::MoveSongUp));
                items.push(MenuItem::new("Move Down", "Shift+J", Action::MoveSongDown));
            }
        }
        items.push(MenuItem::sep());
        items.push(MenuItem::new("Clear Ticks", "Esc", Action::ClearTicks));
        Some(items)
    }

    pub(super) fn play_ticked(&mut self, shuffle: bool) {
        let Some(mut songs) = self.ticked_songs() else { return };
        if songs.is_empty() {
            self.say("there is nothing to play in what's ticked");
            return;
        }
        if shuffle {
            self.mix(&mut songs);
        }
        self.solo_for = None;
        self.follow = None;
        self.engine.play(songs, 0);
    }

    pub(super) fn step_active(&mut self) {
        let h = self.column_height.max(1);
        match self.active_row() {
            Some(TickRow::Search(_)) => self.search_step(1),
            Some(TickRow::Queue(_)) => self.queue_step(1),
            Some(TickRow::Playlist(_)) => {
                if let Some(v) = self.playlists.as_mut() {
                    v.step(1, h);
                }
            }
            Some(TickRow::Library(_)) => {
                if let Some(v) = self.library.as_mut() {
                    v.step(1, h);
                }
            }
            Some(TickRow::Folder { .. }) => self.move_selection(1),
            None => {}
        }
    }

    pub(super) fn use_list_of(&mut self, t: TickRow) {
        match t {
            TickRow::Folder { col, row } if col != self.focus => self.select(col, row),
            TickRow::Playlist(_) => {
                if let Some(v) = self.playlists.as_mut() {
                    v.on_songs = true;
                }
            }
            _ => {}
        }
    }

    pub(super) fn tick_mouse(&mut self, target: Option<&Target>, m: MouseEvent) -> bool {
        let Some(t) = target else { return false };
        if let Target::Tick(row) = *t {
            self.use_list_of(row);
            let on = self.set_tick(row, None);
            self.drag = Some(Target::Tick(row));
            self.tick_drag = Some((row, on));
            return true;
        }
        let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
        let shift = m.modifiers.contains(KeyModifiers::SHIFT);
        let Some(row) = TickRow::of(t).filter(|_| ctrl || shift) else { return false };
        self.use_list_of(row);
        if shift {
            self.extend_ticks(row);
        } else {
            self.set_tick(row, None);
        }
        true
    }

}
