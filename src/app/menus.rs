use super::*;

impl App {
    pub fn file_menu_items(&self) -> Vec<MenuItem> {
        let mut v = vec![
            MenuItem::new("Open Folder…", "Ctrl+O", Action::OpenFolder),
            MenuItem::new("New Playlist…", "", Action::NewPlaylist),
            MenuItem::new("Add New Playlist…", "", Action::ImportPlaylist),
        ];
        if !self.settings.pinned.is_empty() {
            v.push(MenuItem::sep());
            for p in self.settings.pinned.iter().take(10) {
                v.push(MenuItem::new(format!("☆ {}", fsx::contract_tilde(&p.to_string_lossy())), "", Action::OpenPinned(p.clone())));
            }
        }
        v.push(MenuItem::sep());
        if self.settings.recent.is_empty() {
            v.push(MenuItem { label: "No Recent Folders".into(), shortcut: "", action: None, checked: None, heading: false });
        } else {
            for p in self.settings.recent.iter().take(10) {
                v.push(MenuItem::new(fsx::contract_tilde(&p.to_string_lossy()), "", Action::OpenRecent(p.clone())));
            }
            v.push(MenuItem::new("Clear Recent Folders", "", Action::ClearRecent));
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new("Quit", "Q", Action::Quit));
        v
    }

    pub fn menu_items(&self, index: usize) -> Vec<MenuItem> {
        match index {
            0 => self.file_menu_items(),
            1 => vec![
                MenuItem { checked: Some(self.playlists.is_some()), ..MenuItem::new("Playlists", "", Action::Playlists) },
                MenuItem { checked: Some(self.queue_view.is_some()), ..MenuItem::new("Queue", "Shift+W", Action::Queue) },
                MenuItem::new("Search…", "/", Action::Search),
                MenuItem { checked: Some(self.library.is_some()), ..MenuItem::new("Library", "I", Action::Library) },
                MenuItem::sep(),
                MenuItem::new(if self.status.state == State::Playing { "Pause" } else { "Play" }, "Space", Action::PlayPause),
                MenuItem::new("Next", "N", Action::Next),
                MenuItem::new("Next Album", "Shift+A", Action::NextAlbum),
                MenuItem::new("Previous", "B", Action::Previous),
                MenuItem::new("Stop", "S", Action::Stop),
                MenuItem::sep(),
                MenuItem::new("Forward 10 Seconds", ".", Action::SeekBy(10.0)),
                MenuItem::new("Back 10 Seconds", ",", Action::SeekBy(-10.0)),
                MenuItem::new("Volume Up", "+", Action::VolumeBy(0.01)),
                MenuItem::new("Volume Down", "-", Action::VolumeBy(-0.01)),
                MenuItem::sep(),
                MenuItem { checked: Some(self.settings.repeat == Repeat::One), ..MenuItem::new("Repeat", "", Action::ToggleRepeat(Repeat::One)) },
                MenuItem { checked: Some(self.settings.repeat == Repeat::All), ..MenuItem::new("Repeat All", "", Action::ToggleRepeat(Repeat::All)) },
                MenuItem { checked: Some(self.settings.shuffle_albums), ..MenuItem::new("Shuffle Albums", "", Action::ToggleShuffleAlbums) },
                MenuItem { checked: Some(self.settings.continue_folders), ..MenuItem::new("Continue Into Next Folder", "", Action::ToggleContinue) },
                MenuItem::sep(),
                MenuItem::new("Shuffle Queue", "", Action::ShuffleQueue),
                MenuItem::new("Clear Queue", "", Action::ClearQueue),
                MenuItem::new("Save Queue as Playlist…", "", Action::SaveQueue),
            ],
            2 => vec![
                MenuItem::new("Equalizer…", "Ctrl+E", Action::Equalizer),
                MenuItem { checked: Some(self.settings.eq_enabled), ..MenuItem::new("Equalizer On", "", Action::ToggleEq) },
                MenuItem::new(crossfade_label(self.settings.crossfade), "›", Action::CrossfadeMenu),
                MenuItem::new(format!("ReplayGain: {}", replaygain_name(self.settings.replaygain)), "›", Action::ReplayGainMenu),
                MenuItem::sep(),
                MenuItem::new("Theme…", "Ctrl+T", Action::Theme),
                MenuItem { checked: Some(!self.settings.flipped), ..MenuItem::new("Standard View", "", Action::SetFlipped(false)) },
                MenuItem { checked: Some(self.settings.flipped), ..MenuItem::new("Flipped View", "", Action::SetFlipped(true)) },
                MenuItem { checked: Some(self.settings.compact), ..MenuItem::new("Minimalist View", "", Action::ToggleCompact) },
                MenuItem::sep(),
                MenuItem::heading("[ Visualizers ]"),
            ]
            .into_iter()
            .chain(VISUALIZER_MENU.iter().map(|&(label, visual)| MenuItem {
                checked: Some(self.settings.analyzer && self.settings.visual == visual),
                ..MenuItem::new(label, "", Action::ToggleVisual(visual))
            }))
            .chain([
                MenuItem::sep(),
                MenuItem::new("Music Folder…", "", Action::MusicFolder),
                MenuItem { checked: Some(self.settings.show_hidden), ..MenuItem::new("Show Hidden Folders", "", Action::ToggleHidden) },
            ])
            .collect(),
            _ => vec![
                MenuItem::new("Keyboard & Mouse", "F1", Action::Help),
                MenuItem::new("About Urchin Player", "", Action::About),
            ],
        }
    }

    pub(super) fn order_items(&self) -> Vec<MenuItem> {
        let c = &self.levels[self.focus];
        let Some(i) = c.selected else { return Vec::new() };
        let song = |i: usize| c.entries.get(i).is_some_and(|e| e.kind == Kind::Track);
        let mut v = Vec::new();
        if i > 0 && song(i - 1) {
            v.push(MenuItem::new("Move Up", "Shift+K", Action::MoveSongUp));
        }
        if song(i + 1) {
            v.push(MenuItem::new("Move Down", "Shift+J", Action::MoveSongDown));
        }
        if self.order.is_custom(&c.dir) {
            v.push(MenuItem::new("Reset Order", "", Action::ResetOrder));
        }
        if !v.is_empty() {
            v.insert(0, MenuItem::sep());
        }
        v
    }

    pub(super) fn context_items(&self, e: &Entry) -> Vec<MenuItem> {
        let mut v = vec![
            MenuItem::new("Shuffle", "", Action::ShuffleSelection),
            MenuItem::new("Play Next", "Shift+N", Action::PlayNext),
            MenuItem::new("Add to Queue", "Shift+Q", Action::AddToQueue),
        ];
        if e.kind == Kind::Track {
            v.push(MenuItem::new("Add to Playlist", "›", Action::AddToPlaylistMenu(e.path.clone())));
            v.extend(self.order_items());
        }
        if e.kind == Kind::Folder {
            v.push(MenuItem::new("Open", "→", Action::OpenSelection));
            let pinned = self.settings.pinned.contains(&e.path);
            v.push(MenuItem::new(if pinned { "Unpin" } else { "Pin" }, "", Action::TogglePinSelection));
        }
        v.push(MenuItem::sep());
        v.extend(file_items(&e.path));
        v
    }

    pub fn open_menu(&mut self, index: usize, x: u16) {
        let items = self.menu_items(index);
        self.menu = Some(OpenMenu { title_index: Some(index), ..OpenMenu::new(items, Anchor::At { x, y: 1 }) });
    }

    pub(super) fn put_away_menu_bar(&mut self) {
        self.menu = None;
        if self.settings.compact {
            self.menu_peek = false;
            self.menu_peek_held = true;
        }
    }

    pub(super) fn click_menu(&mut self, index: usize) {
        self.open_menu(index, menu_title_x(index));
        if let Some(m) = self.menu.as_mut() {
            m.clicked = true;
        }
    }

    pub fn run(&mut self, action: Action) {
        let menu_at = self.menu.as_ref().map(|m| m.anchor.as_click());
        self.menu = None;
        match action {
            Action::AddToPlaylistMenu(song) => {
                let songs = self.ticked_songs().unwrap_or_else(|| vec![song]);
                self.show_playlist_menu(songs, menu_at.unwrap_or(Anchor::Click { col: 2, row: 2 }));
            }
            Action::AddManyToPlaylist { playlist, songs } => {
                let name = playlists::name_of(&playlist);
                let (mut added, mut had) = (0, 0);
                for song in &songs {
                    match playlists::add_song(&playlist, song) {
                        Ok(true) => added += 1,
                        Ok(false) => had += 1,
                        Err(e) => {
                            self.say(format!("couldn't add to {name}: {e}"));
                            return;
                        }
                    }
                }
                self.say(match (added, had) {
                    (0, _) => format!("they are all in {name} already"),
                    (n, 0) => format!("added {} to {name}", plural_songs(n)),
                    (n, h) => format!("added {} to {name} ({h} were in it already)", plural_songs(n)),
                });
            }
            Action::ClearTicks => {
                self.clear_ticks();
            }
            Action::AddToPlaylist { playlist, song } => {
                let name = playlists::name_of(&playlist);
                let title = fsx::title_from_path(&song);
                match playlists::add_song(&playlist, &song) {
                    Ok(true) => self.say(format!("added {title} to {name}")),
                    Ok(false) => self.say(format!("{title} is already in {name}")),
                    Err(e) => self.say(format!("couldn't add to {name}: {e}")),
                }
            }
            Action::OpenFolder => self.show_open(OpenMode::Open),
            Action::ImportPlaylist => self.show_open(OpenMode::ImportPlaylists),
            Action::NewPlaylist => self.overlay = Some(Overlay::PlaylistName(String::new(), NameFor::New)),
            Action::RenamePlaylist(path) => {
                self.overlay = Some(Overlay::PlaylistName(playlists::name_of(&path), NameFor::Rename(path)));
            }
            Action::DeletePlaylist(path) => {
                let name = playlists::name_of(&path);
                let keep = self.playlists.as_ref().map(|v| (v.selected, v.files.clone()));
                match playlists::delete(&path) {
                    Ok(()) => {
                        let select = keep.and_then(|(at, files)| {
                            files.get(at + 1).or_else(|| at.checked_sub(1).and_then(|k| files.get(k))).cloned()
                        });
                        if self.playlists.is_some() {
                            self.open_playlists(select);
                        }
                        self.say(format!("deleted {name}"));
                    }
                    Err(e) => self.say(format!("couldn't delete {name}: {e}")),
                }
            }
            Action::Playlists => {
                if self.playlists.take().is_none() {
                    self.open_playlists(None);
                }
            }
            Action::Queue => {
                if self.queue_view.take().is_none() {
                    self.open_queue();
                }
            }
            Action::Library => {
                if self.library.take().is_none() {
                    self.open_library();
                }
            }
            Action::LibraryShowSongs => {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.show_songs(h);
                }
            }
            Action::LibraryShow => self.library_show(),
            Action::Search => self.show_search(),
            Action::SearchPlay => self.search_play(),
            Action::SearchPlayNext => self.search_enqueue(true),
            Action::SearchAddToQueue => self.search_enqueue(false),
            Action::SearchShow => self.search_show(),
            Action::PlayNext => self.enqueue_selection(true),
            Action::AddToQueue => self.enqueue_selection(false),
            Action::QueuePlay => self.queue_play(),
            Action::QueueMoveUp => self.queue_move(-1),
            Action::QueueMoveDown => self.queue_move(1),
            Action::MoveSongUp => self.move_song(-1),
            Action::MoveSongDown => self.move_song(1),
            Action::ResetOrder => self.reset_order(),
            Action::QueueRemove => self.queue_remove(),
            Action::ClearQueue => self.clear_queue(),
            Action::ShuffleQueue => self.shuffle_queue(),
            Action::SaveQueue => self.save_queue(),
            Action::MusicFolder => self.show_open(OpenMode::MusicFolder),
            Action::OpenRecent(p) | Action::OpenPinned(p) => {
                if p.is_dir() {
                    self.open_folder(&p);
                } else {
                    self.say(format!("{} isn't available right now", fsx::contract_tilde(&p.to_string_lossy())));
                }
            }
            Action::ClearRecent => {
                self.settings.recent.clear();
                self.settings.save();
            }
            Action::Quit => self.quit = true,
            Action::PlayPause => {
                let now = self.engine.status();
                if now.state == State::Stopped && self.engine.caught_up(&now) {
                    if self.queue_view.is_some() {
                        self.queue_play();
                    } else if self.library.is_some() {
                        self.play_library(false);
                    } else if self.playlists.is_some() {
                        self.play_playlist(false);
                    } else if let Some((e, col)) = self.selection_or_current() {
                        self.play_entry(&e, col, false);
                    }
                } else {
                    self.engine.toggle_pause();
                }
            }
            Action::Next => self.engine.next(),
            Action::NextAlbum => self.next_album(),
            Action::Previous => self.engine.previous(),
            Action::Stop => self.engine.stop(),
            Action::SeekBy(d) => self.engine.seek_by(d),
            Action::VolumeBy(d) => self.set_volume(self.engine.volume() + d),
            Action::Reveal(path) => {
                self.watch_until = Some(Instant::now() + WATCH_FOR);
                self.check_folders();
                let tx = self.reveals.0.clone();
                let _ = std::thread::Builder::new().name("urchin-reveal".into()).spawn(move || {
                    let shown = crate::reveal::show(&path);
                    let _ = tx.send((path, shown));
                });
            }
            Action::CopyPath(path) => self.copy_path(&path, "copied the path"),
            Action::ToggleContinue => {
                self.settings.continue_folders = !self.settings.continue_folders;
                self.settings.save();
                self.say(if self.settings.continue_folders {
                    "when a folder ends, the next folder plays"
                } else {
                    "playback stops when the folder ends"
                });
            }
            Action::ToggleShuffleAlbums => {
                self.settings.shuffle_albums = !self.settings.shuffle_albums;
                self.settings.save();
                self.say(if self.settings.shuffle_albums { "shuffle now mixes whole albums" } else { "shuffle now mixes single songs" });
            }
            Action::ToggleRepeat(mode) => {
                self.settings.repeat = if self.settings.repeat == mode { Repeat::Off } else { mode };
                self.engine.set_repeat(self.settings.repeat);
                self.settings.save();
            }
            Action::Theme => self.show_theme_picker(),
            Action::Equalizer => self.overlay = Some(Overlay::Equalizer),
            Action::ToggleEq => self.eq_toggle(),
            Action::CrossfadeMenu => {
                let items = CROSSFADE_CHOICES
                    .iter()
                    .map(|&s| MenuItem {
                        checked: Some(self.settings.crossfade == s),
                        ..MenuItem::new(if s == 0 { "Off".to_string() } else { format!("{s} seconds") }, "", Action::SetCrossfade(s))
                    })
                    .collect();
                self.menu = Some(OpenMenu::new(items, menu_at.unwrap_or(Anchor::Click { col: 2, row: 2 })));
            }
            Action::SetCrossfade(s) => {
                self.settings.crossfade = s;
                self.engine.set_crossfade(s);
                self.settings.save();
                self.say(if s == 0 { "crossfade off: songs join without a gap".to_string() } else { format!("crossfade: {s} seconds between songs") });
            }
            Action::ReplayGainMenu => {
                let items = [ReplayGain::Off, ReplayGain::Track, ReplayGain::Album]
                    .into_iter()
                    .map(|m| MenuItem { checked: Some(self.settings.replaygain == m), ..MenuItem::new(replaygain_name(m), "", Action::SetReplayGain(m)) })
                    .collect();
                self.menu = Some(OpenMenu::new(items, menu_at.unwrap_or(Anchor::Click { col: 2, row: 2 })));
            }
            Action::SetReplayGain(m) => {
                self.settings.replaygain = m;
                self.engine.set_replaygain(m);
                self.settings.save();
                self.say(match m {
                    ReplayGain::Off => "ReplayGain off: songs play as they are",
                    ReplayGain::Track => "ReplayGain: every song at the same loudness",
                    ReplayGain::Album => "ReplayGain: every album at the same loudness",
                });
            }
            Action::EqPreset(i) => self.eq_preset(i),
            Action::EqSavePreset => self.eq_save_preset(),
            Action::EqMyPresetMenu(id) => {
                let at = self.menu.as_ref().map_or(Anchor::At { x: self.eq_preset_x, y: self.eq_rect.y }, |m| m.anchor);
                self.my_preset_menu(id, at);
            }
            Action::EqRenamePreset(id) => self.eq_rename_preset(id),
            Action::EqDeletePreset(id) => {
                self.menu = None;
                self.eq_delete_preset(id);
            }
            Action::SetFlipped(f) => {
                self.settings.flipped = f;
                self.settings.save();
            }
            Action::ToggleCompact => {
                self.settings.compact = !self.settings.compact;
                self.settings.save();
            }
            Action::ToggleVisual(visual) => self.toggle_visual(visual),
            Action::ToggleHidden => {
                self.settings.show_hidden = !self.settings.show_hidden;
                self.settings.save();
                self.refresh();
            }
            Action::PlaySelection | Action::ShuffleSelection if self.queue_view.is_none() && self.tick_count() > 0 => {
                self.play_ticked(action == Action::ShuffleSelection);
            }
            Action::PlaySelection | Action::ShuffleSelection => {
                if self.queue_view.is_some() {
                    if action == Action::ShuffleSelection {
                        self.shuffle_queue();
                    } else {
                        self.queue_play();
                    }
                } else if self.library.is_some() {
                    self.play_library(action == Action::ShuffleSelection);
                } else if self.playlists.is_some() {
                    self.play_playlist(action == Action::ShuffleSelection);
                } else if let Some((e, col)) = self.selection_or_current() {
                    self.play_entry(&e, col, action == Action::ShuffleSelection);
                }
            }
            Action::OpenSelection => self.enter_selected(),
            Action::TogglePinSelection => {
                if let Some(e) = self.selected_entry().cloned() {
                    let target = if e.kind == Kind::Folder { e.path } else { self.current_dir() };
                    self.toggle_pin(target);
                }
            }
            Action::Help => self.overlay = Some(Overlay::Help),
            Action::About => self.overlay = Some(Overlay::About),
        }
    }
}
