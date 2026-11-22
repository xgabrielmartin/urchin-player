use super::*;

impl App {
    pub(super) fn open_playlists(&mut self, select: Option<PathBuf>) {
        self.solo = false;
        self.solo_for = None;
        self.key_focus = None;
        self.queue_view = None;
        self.library = None;
        self.playlists = Some(playlists::View::open(&playlists::dir(), select.as_deref()));
    }

    pub(super) fn show_playlist_menu(&mut self, songs: Vec<PathBuf>, at: Anchor) {
        let lists = playlists::list_in(&playlists::dir());
        let items = if lists.is_empty() {
            vec![MenuItem { label: "No Playlists Yet".into(), shortcut: "", action: None, checked: None, heading: false }]
        } else {
            lists
                .into_iter()
                .map(|p| {
                    let action = match songs.as_slice() {
                        [song] => Action::AddToPlaylist { playlist: p.clone(), song: song.clone() },
                        _ => Action::AddManyToPlaylist { playlist: p.clone(), songs: songs.clone() },
                    };
                    MenuItem::new(playlists::name_of(&p), "", action)
                })
                .collect()
        };
        self.menu = Some(OpenMenu::new(items, at.as_click()));
    }

    pub(super) fn add_selection_to_playlist(&mut self) {
        if let (Some(songs), Some(row)) = (self.ticked_songs(), self.active_row()) {
            let at = self.row_anchor(row.row_target());
            self.show_playlist_menu(songs, at);
            return;
        }
        let in_songs = self.library.as_ref().and_then(|v| v.highlighted_song().map(|t| (t.path.clone(), v.song_selected)));
        if let Some((song, row)) = in_songs {
            self.show_playlist_menu(vec![song], self.row_anchor(Target::LibraryRow { col: library::SONGS, row }));
            return;
        }
        if self.in_view() {
            return;
        }
        let Some(song) = self.selected_entry().filter(|e| e.kind == Kind::Track).map(|e| e.path.clone()) else {
            self.say("select a song to add to a playlist");
            return;
        };
        let row = Target::Row { col: self.focus, row: self.levels[self.focus].selected.unwrap_or(0) };
        self.show_playlist_menu(vec![song], self.row_anchor(row));
    }

    pub(super) fn play_playlist(&mut self, shuffle: bool) {
        if self.tick_count() > 0 {
            return self.play_ticked(shuffle);
        }
        let Some(v) = &self.playlists else { return };
        if v.songs.is_empty() {
            self.say("there is nothing to play in this playlist");
            return;
        }
        let mut queue = v.songs.clone();
        let mut index = if v.on_songs { v.song_selected } else { 0 };
        if shuffle {
            self.mix(&mut queue);
            index = 0;
        }
        self.follow = None;
        self.engine.play(queue, index);
    }

    pub(super) fn playlist_menu_items(&self, on_songs: bool) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem::new("Shuffle", "", Action::ShuffleSelection),
            MenuItem::new("Play Next", "Shift+N", Action::PlayNext),
            MenuItem::new("Add to Queue", "Shift+Q", Action::AddToQueue),
        ];
        if let (false, Some(file)) = (on_songs, self.playlists.as_ref().and_then(|v| v.files.get(v.selected))) {
            items.push(MenuItem::sep());
            items.push(MenuItem::new("Rename", "F2", Action::RenamePlaylist(file.clone())));
            items.push(MenuItem::new("Delete", "Del", Action::DeletePlaylist(file.clone())));
            items.push(MenuItem::sep());
            items.extend(file_items(file));
        }
        if let (true, Some(song)) = (on_songs, self.playlists.as_ref().and_then(|v| v.songs.get(v.song_selected))) {
            items.push(MenuItem::sep());
            items.extend(file_items(song));
        }
        items
    }

    pub(super) fn row_anchor(&self, row: Target) -> Anchor {
        let (col, row) = self.hits.iter().find(|h| h.target == row).map_or((2, 2), |h| (h.rect.x + 2, h.rect.y));
        Anchor::Click { col, row }
    }

    pub(super) fn open_context_menu_by_key(&mut self) {
        if let (Some(items), Some(row)) = (self.ticked_menu(), self.active_row()) {
            let hover = items.iter().position(|i| i.action.is_some());
            self.menu = Some(OpenMenu { hover, ..OpenMenu::new(items, self.row_anchor(row.row_target())) });
            return;
        }
        let (items, row) = if let Some(v) = &self.queue_view {
            (self.queue_menu_items(), Target::QueueRow(v.selected))
        } else if let Some(v) = &self.playlists {
            let songs = v.on_songs;
            let row = if songs { Target::PlaylistSong(v.song_selected) } else { Target::PlaylistRow(v.selected) };
            (self.playlist_menu_items(songs), row)
        } else if let Some(v) = &self.library {
            let row = if v.focus == library::SONGS { v.song_selected } else { v.cols[v.focus].selected };
            (self.library_menu_items(), Target::LibraryRow { col: v.focus, row })
        } else {
            let Some(e) = self.selected_entry().cloned() else { return };
            let col = self.focus;
            (self.context_items(&e), Target::Row { col, row: self.levels[col].selected.unwrap_or(0) })
        };
        let hover = items.iter().position(|i| i.action.is_some());
        self.menu = Some(OpenMenu { hover, ..OpenMenu::new(items, self.row_anchor(row)) });
    }

    pub(super) fn playlist_key_action(&mut self, delete: bool) {
        let Some(v) = &self.playlists else { return };
        if v.on_songs {
            self.say("press ← to pick a playlist first");
            return;
        }
        let Some(file) = v.files.get(v.selected).cloned() else { return };
        self.run(if delete { Action::DeletePlaylist(file) } else { Action::RenamePlaylist(file) });
    }

    pub(super) fn playlists_key(&mut self, k: KeyEvent) -> bool {
        const FAR: isize = 1 << 30;
        let h = self.column_height.max(1);
        let Some(v) = self.playlists.as_mut() else { return false };
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => v.step(-1, h),
            KeyCode::Down | KeyCode::Char('j') => v.step(1, h),
            KeyCode::PageUp => v.step(-(h as isize), h),
            KeyCode::PageDown => v.step(h as isize, h),
            KeyCode::Home => v.step(-FAR, h),
            KeyCode::End => v.step(FAR, h),
            KeyCode::Left | KeyCode::Char('h') => v.on_songs = false,
            KeyCode::Right | KeyCode::Char('l') => v.on_songs = !v.songs.is_empty(),
            KeyCode::Enter => self.play_playlist(false),
            KeyCode::F(2) => self.playlist_key_action(false),
            KeyCode::Delete => self.playlist_key_action(true),
            KeyCode::Char('m') | KeyCode::Menu if !k.modifiers.contains(KeyModifiers::CONTROL) => self.open_context_menu_by_key(),
            KeyCode::Esc | KeyCode::Backspace => self.playlists = None,
            _ => return false,
        }
        true
    }

    pub(super) fn open_library(&mut self) {
        self.solo = false;
        self.solo_for = None;
        self.key_focus = None;
        self.playlists = None;
        self.queue_view = None;
        self.library = Some(library::View::open(self.root.clone()));
    }

    pub(super) fn play_library(&mut self, shuffle: bool) {
        if self.tick_count() > 0 {
            return self.play_ticked(shuffle);
        }
        let Some(v) = &self.library else { return };
        if v.reading() {
            self.say("still reading the tags of your music");
            return;
        }
        let Some((mut queue, mut index)) = v.playlist() else {
            self.say("there is nothing to play here");
            return;
        };
        if shuffle {
            self.mix(&mut queue);
            index = 0;
        }
        self.follow = None;
        self.engine.play(queue, index);
    }

    pub(super) fn library_menu_items(&self) -> Vec<MenuItem> {
        let Some(v) = &self.library else { return Vec::new() };
        let mut items = Vec::new();
        if v.focus == 2 {
            items.push(MenuItem::new("Show Songs", "→", Action::LibraryShowSongs));
        }
        items.push(MenuItem::new("Shuffle", "", Action::ShuffleSelection));
        items.push(MenuItem::new("Play Next", "Shift+N", Action::PlayNext));
        items.push(MenuItem::new("Add to Queue", "Shift+Q", Action::AddToQueue));
        if let Some(t) = v.highlighted_song() {
            items.push(MenuItem::sep());
            items.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(t.path.clone())));
            items.push(MenuItem::new("Show in Folder", "", Action::LibraryShow));
            items.extend(file_items(&t.path));
        }
        items
    }

    pub(super) fn library_show(&mut self) {
        let Some(song) = self.library.as_ref().and_then(|v| v.highlighted_song()).map(|t| t.path.clone()) else { return };
        self.show_in_folder(song);
    }

    pub(super) fn library_key(&mut self, k: KeyEvent) -> bool {
        const FAR: isize = 1 << 30;
        let h = self.column_height.max(1);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(v) = self.library.as_mut() else { return false };
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => v.step(-1, h),
            KeyCode::Down | KeyCode::Char('j') => v.step(1, h),
            KeyCode::PageUp => v.step(-(h as isize), h),
            KeyCode::PageDown => v.step(h as isize, h),
            KeyCode::Home => v.step(-FAR, h),
            KeyCode::End => v.step(FAR, h),
            KeyCode::Left | KeyCode::Char('h') => {
                v.left();
            }
            KeyCode::Right | KeyCode::Char('l') => v.right(h),
            KeyCode::Enter => self.play_library(false),
            KeyCode::Char('m') | KeyCode::Menu if !ctrl => self.open_context_menu_by_key(),
            KeyCode::Backspace => self.leave_view(),
            KeyCode::Esc => self.library = None,
            _ => return false,
        }
        true
    }

    pub(super) fn open_queue(&mut self) {
        self.solo = false;
        self.solo_for = None;
        self.key_focus = None;
        self.playlists = None;
        self.library = None;
        let playing = self.status.playing_index();
        self.queue_view = Some(queue::View::open(playing, self.status.queue.len(), self.column_height));
    }

    pub(super) fn queue_selected(&self) -> Option<(usize, PathBuf)> {
        let i = self.queue_view.as_ref()?.selected;
        self.status.queue.get(i).map(|s| (i, s.clone()))
    }

    pub(super) fn settle_engine(&mut self) {
        let end = Instant::now() + Duration::from_millis(60);
        loop {
            self.status = self.engine.status();
            if self.engine.caught_up(&self.status) || Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn enqueue_selection(&mut self, next: bool) {
        if self.queue_view.is_some() {
            return;
        }
        if let Some(songs) = self.ticked_songs() {
            let name = songs.first().map(|s| fsx::title_from_path(s)).unwrap_or_default();
            self.enqueue_paths(songs, name, next);
            return;
        }
        let (paths, name) = if let Some(v) = &self.library {
            if v.focus == library::SONGS {
                let song = v.highlighted_song();
                (song.iter().map(|t| t.path.clone()).collect::<Vec<_>>(), song.map(|t| t.title.clone()).unwrap_or_default())
            } else {
                (v.songs_of(v.focus), v.picked_name(v.focus))
            }
        } else if let Some(v) = &self.playlists {
            if v.on_songs {
                let song = v.songs.get(v.song_selected).cloned();
                (song.iter().cloned().collect::<Vec<_>>(), song.map(|s| fsx::title_from_path(&s)).unwrap_or_default())
            } else {
                (v.songs.clone(), v.names.get(v.selected).cloned().unwrap_or_default())
            }
        } else if let Some((e, _)) = self.selection_or_current() {
            match e.kind {
                Kind::Folder => (fsx::collect_tracks_in(&e.path, &self.order), e.name.clone()),
                Kind::Track => (vec![e.path.clone()], fsx::title_from_path(&e.path)),
            }
        } else {
            return;
        };
        self.enqueue_paths(paths, name, next);
    }

    pub(super) fn enqueue_paths(&mut self, paths: Vec<PathBuf>, name: String, next: bool) {
        if paths.is_empty() {
            self.say("there is nothing to add here");
            return;
        }
        let n = paths.len();
        let starts = self.status.state == State::Stopped;
        if starts {
            self.follow = None;
        }
        self.engine.enqueue(paths, next);
        self.settle_engine();
        if starts {
            return;
        }
        self.say(match (n, next) {
            (1, true) => format!("{name} plays next"),
            (1, false) => format!("added {name} to the queue"),
            (n, true) => format!("{n} songs play next"),
            (n, false) => format!("added {n} songs to the queue"),
        });
    }

    pub(super) fn queue_step(&mut self, delta: isize) {
        let (len, h) = (self.status.queue.len(), self.column_height.max(1));
        if let Some(v) = self.queue_view.as_mut() {
            v.step(delta, len, h);
        }
    }

    pub(super) fn queue_play(&mut self) {
        if let Some((i, song)) = self.queue_selected() {
            self.engine.jump(i, song);
            self.settle_engine();
        }
    }

    pub(super) fn queue_remove(&mut self) {
        let ticked = self.queue_view.as_ref().map(|v| v.marks.sorted()).unwrap_or_default();
        if !ticked.is_empty() {
            for &i in ticked.iter().rev() {
                if let Some(song) = self.status.queue.get(i).cloned() {
                    self.engine.remove_from_queue(i, song);
                }
            }
            self.settle_engine();
            let (len, h) = (self.status.queue.len(), self.column_height.max(1));
            if let Some(v) = self.queue_view.as_mut() {
                v.marks.clear();
                v.clamp(len, h);
            }
            self.say(format!("took {} out of the queue", plural_songs(ticked.len())));
            return;
        }
        let Some((i, song)) = self.queue_selected() else { return };
        self.engine.remove_from_queue(i, song);
        self.settle_engine();
        let (len, h) = (self.status.queue.len(), self.column_height.max(1));
        if let Some(v) = self.queue_view.as_mut() {
            v.clamp(len, h);
        }
    }

    pub(super) fn queue_move(&mut self, delta: isize) {
        let ticked = self.queue_view.as_ref().map(|v| v.marks.sorted()).unwrap_or_default();
        if !ticked.is_empty() {
            let mut queue: Vec<PathBuf> = self.status.queue.to_vec();
            let moves = crate::marks::block_moves(&ticked, delta, queue.len(), |_| true);
            if moves.is_empty() {
                return;
            }
            let selected = self.queue_view.as_ref().map_or(0, |v| v.selected);
            let mut follow = selected;
            for &(from, to) in &moves {
                let song = queue.remove(from);
                queue.insert(to, song.clone());
                self.engine.move_in_queue(from, to, song);
                if follow == from {
                    follow = to;
                } else if follow == to {
                    follow = from;
                }
            }
            self.settle_engine();
            let (len, h) = (self.status.queue.len(), self.column_height.max(1));
            if let Some(v) = self.queue_view.as_mut() {
                for &(from, to) in &moves {
                    v.marks.moved(from, to);
                }
                v.select(follow, len, h);
            }
            return;
        }
        let Some((from, song)) = self.queue_selected() else { return };
        let len = self.status.queue.len();
        let to = (from as isize + delta).clamp(0, len as isize - 1) as usize;
        if to == from {
            return;
        }
        self.engine.move_in_queue(from, to, song);
        self.settle_engine();
        let (len, h) = (self.status.queue.len(), self.column_height.max(1));
        if let Some(v) = self.queue_view.as_mut() {
            v.select(to, len, h);
        }
    }

    pub(super) fn clear_queue(&mut self) {
        let playing = self.status.playing_index().is_some();
        let n = self.status.queue.len();
        if n == 0 || (playing && n == 1) {
            self.say("the queue is already empty");
            return;
        }
        self.engine.clear_queue();
        self.settle_engine();
        if let Some(v) = self.queue_view.as_mut() {
            v.marks.clear();
        }
        let (len, h) = (self.status.queue.len(), self.column_height.max(1));
        if let Some(v) = self.queue_view.as_mut() {
            v.select(0, len, h);
        }
        self.say(if playing { "cleared the queue (the song playing stays)" } else { "cleared the queue" });
    }

    pub(super) fn shuffle_queue(&mut self) {
        if let Some(v) = self.queue_view.as_mut() {
            v.marks.clear();
        }
        let from = self.status.playing_index().map_or(0, |i| i + 1);
        let upcoming = self.status.queue.len().saturating_sub(from);
        if upcoming < 2 {
            self.say(if upcoming == 0 { "no songs are left to shuffle" } else { "only one song is left to shuffle" });
            return;
        }
        if self.settings.shuffle_albums {
            let songs: Vec<PathBuf> = self.status.queue[from..].to_vec();
            let keys = self.album_keys(&songs);
            let (order, count) = albums::shuffled_order(&keys, queue::clock_seed());
            if count < 2 {
                self.say("only one album is left to shuffle");
                return;
            }
            self.engine.reorder_upcoming(songs, order);
            self.settle_engine();
            self.say(format!("shuffled {count} albums"));
            return;
        }
        self.engine.shuffle_upcoming(queue::clock_seed());
        self.settle_engine();
        self.say(format!("shuffled {}", plural_songs(upcoming)));
    }

    pub(super) fn save_queue(&mut self) {
        if self.status.queue.is_empty() {
            self.say("the queue is empty");
        } else {
            self.overlay = Some(Overlay::PlaylistName(String::new(), NameFor::Queue));
        }
    }

    pub(super) fn queue_menu_items(&self) -> Vec<MenuItem> {
        let Some((i, song)) = self.queue_selected() else { return Vec::new() };
        let mut items = vec![MenuItem::new("Play", "Enter", Action::QueuePlay), MenuItem::sep()];
        if i > 0 {
            items.push(MenuItem::new("Move Up", "Shift+K", Action::QueueMoveUp));
        }
        if i + 1 < self.status.queue.len() {
            items.push(MenuItem::new("Move Down", "Shift+J", Action::QueueMoveDown));
        }
        items.push(MenuItem::new("Remove", "Del", Action::QueueRemove));
        items.push(MenuItem::sep());
        items.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(song.clone())));
        items.extend(file_items(&song));
        items.push(MenuItem::sep());
        items.push(MenuItem::new("Shuffle Queue", "Z", Action::ShuffleQueue));
        items.push(MenuItem::new("Clear Queue", "Shift+C", Action::ClearQueue));
        items.push(MenuItem::new("Save Queue as Playlist…", "Shift+S", Action::SaveQueue));
        items
    }

    pub(super) fn queue_key(&mut self, k: KeyEvent) -> bool {
        const FAR: isize = 1 << 30;
        let h = self.column_height.max(1) as isize;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Up if shift => self.queue_move(-1),
            KeyCode::Down if shift => self.queue_move(1),
            KeyCode::Char('K') => self.queue_move(-1),
            KeyCode::Char('J') => self.queue_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.queue_step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.queue_step(1),
            KeyCode::PageUp => self.queue_step(-h),
            KeyCode::PageDown => self.queue_step(h),
            KeyCode::Home => self.queue_step(-FAR),
            KeyCode::End => self.queue_step(FAR),
            KeyCode::Enter => self.queue_play(),
            KeyCode::Delete => self.queue_remove(),
            KeyCode::Char('z' | 'Z') if !ctrl => self.run(Action::ShuffleQueue),
            KeyCode::Char('C') if !ctrl => self.run(Action::ClearQueue),
            KeyCode::Char('S') if !ctrl => self.run(Action::SaveQueue),
            KeyCode::Char('a') if !ctrl => {
                if let Some((i, song)) = self.queue_selected() {
                    let row = Target::QueueRow(i);
                    let songs = self.ticked_songs().unwrap_or_else(|| vec![song]);
                    self.show_playlist_menu(songs, self.row_anchor(row));
                }
            }
            KeyCode::Char('m') | KeyCode::Menu if !ctrl => self.open_context_menu_by_key(),
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l' | 'q' | 'Q') if !ctrl => {}
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('W') => self.queue_view = None,
            _ => return false,
        }
        true
    }

    pub(super) fn selection_or_current(&self) -> Option<(Entry, usize)> {
        if let Some(e) = self.selected_entry() {
            return Some((e.clone(), self.focus));
        }
        let dir = self.current_dir();
        Some((
            Entry { name: dir.file_name()?.to_string_lossy().to_string(), path: dir, kind: Kind::Folder },
            self.focus,
        ))
    }

}
