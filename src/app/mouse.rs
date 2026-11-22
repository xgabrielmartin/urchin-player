use super::*;

impl App {
    pub fn hit_test(&self, x: u16, y: u16) -> Option<Target> {
        self.hits
            .iter()
            .rev()
            .find(|h| x >= h.rect.x && x < h.rect.x + h.rect.width && y >= h.rect.y && y < h.rect.y + h.rect.height)
            .map(|h| h.target.clone())
    }

    pub(super) fn is_double(&mut self, t: &Target) -> bool {
        let now = Instant::now();
        let double = matches!(&self.last_click, Some((when, last)) if last == t && now.duration_since(*when) < DOUBLE_CLICK);
        self.last_click = if double { None } else { Some((now, t.clone())) };
        double
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let target = self.hit_test(m.column, m.row);
        let reveals_toolbar = self.toolbar_hidden() && m.row == self.toolbar_row;
        self.pointer_row = Some(m.row);
        self.toolbar_peek_until = (m.row == self.toolbar_row).then(|| Instant::now() + TOOLBAR_PEEK);
        if !matches!(m.kind, MouseEventKind::Moved) {
            self.touch_resume_solo();
        }
        if matches!(m.kind, MouseEventKind::Down(_)) && m.row == self.toolbar_row && !self.toolbar_hidden() {
            self.toolbar_woken = true;
        }
        match m.kind {
            MouseEventKind::Moved => {
                self.hover = target.clone();
                if m.row != 0 {
                    self.menu_peek_held = false;
                }
                self.menu_peek = self.settings.compact
                    && m.row == 0
                    && !self.menu_peek_held
                    && (self.menu_peek || (target.is_none() && !reveals_toolbar));
                if let (Some(menu), Some(Target::MenuItem(i))) = (self.menu.as_mut(), &target) {
                    menu.hover = Some(*i);
                }
                if let (Some(menu), Some(Target::Menu(i))) = (&self.menu, &target) {
                    if menu.title_index.is_some() && menu.title_index != Some(*i) {
                        self.open_menu(*i, menu_title_x(*i));
                    }
                }
            }
            MouseEventKind::Down(button) => self.mouse_down(button, target, m),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(m),
            MouseEventKind::Up(MouseButton::Left) => {
                if let (Some(Target::Seek), Some(pos)) = (&self.drag, self.drag_seek) {
                    self.engine.seek(pos);
                }
                if matches!(self.drag, Some(Target::Divider(_))) {
                    self.settings.save();
                    self.hover = target;
                }
                self.drag = None;
                self.drag_seek = None;
                self.tick_drag = None;
            }
            MouseEventKind::ScrollUp => self.wheel(target, -1),
            MouseEventKind::ScrollDown => self.wheel(target, 1),
            _ => {}
        }
    }

    pub(super) fn mouse_down(&mut self, button: MouseButton, target: Option<Target>, m: MouseEvent) {
        self.scope_keys = self.menu.is_none() && self.overlay.is_none() && target == Some(Target::Scope) && self.scope_mode().is_some();
        let target = target.map(|t| match t {
            Target::Tick(r) if button != MouseButton::Left => r.row_target(),
            t => t,
        });
        if self.menu.is_some() {
            match target {
                Some(Target::MenuItem(i)) if button == MouseButton::Left => {
                    let action = self.menu.as_ref().and_then(|mm| mm.items.get(i).and_then(|it| it.action.clone()));
                    if let Some(a) = action {
                        self.run(a);
                    }
                }
                Some(Target::MenuBar) => self.put_away_menu_bar(),
                Some(Target::Menu(i)) if button == MouseButton::Left => {
                    let open = self.menu.as_ref().filter(|mm| mm.title_index == Some(i));
                    match open {
                        Some(mm) if mm.clicked => self.put_away_menu_bar(),
                        Some(_) => {
                            if let Some(mm) = self.menu.as_mut() {
                                mm.clicked = true;
                            }
                        }
                        None => self.click_menu(i),
                    }
                }
                Some(Target::MenuItem(i)) if button == MouseButton::Right => {
                    let action = self.menu.as_ref().and_then(|mm| mm.items.get(i).and_then(|it| it.action.clone()));
                    if let Some(Action::EqPreset(id)) = action {
                        let id = id as u8;
                        if self.eq_store.lock().unwrap().is_mine(id) {
                            self.my_preset_menu(id, Anchor::Click { col: m.column, row: m.row });
                        }
                    }
                }
                Some(Target::MenuItem(_)) => {}
                _ => self.menu = None,
            }
            return;
        }

        if self.settings.compact && self.menu_peek_held && m.row == 0 && button == MouseButton::Left && target.is_none() {
            self.menu_peek_held = false;
            self.menu_peek = true;
            if let Some(i) = menu_title_at(m.column) {
                self.click_menu(i);
            }
            return;
        }

        if target == Some(Target::MenuBar) {
            self.put_away_menu_bar();
            return;
        }

        if button == MouseButton::Left && self.tick_mouse(target.as_ref(), m) {
            return;
        }

        if self.overlay.is_some() {
            self.overlay_mouse(button, target, m);
            return;
        }

        let Some(target) = target else { return };
        if button == MouseButton::Right {
            if let Target::Row { col, row } = target {
                self.select(col, row);
                if let Some(e) = self.selected_entry().cloned() {
                    let items = self.ticked_menu().unwrap_or_else(|| self.context_items(&e));
                    self.menu = Some(OpenMenu::new(items, Anchor::Click { col: m.column, row: m.row }));
                }
            } else if let (Target::PlaylistRow(i) | Target::PlaylistSong(i), Some(v)) = (&target, self.playlists.as_mut()) {
                let songs = matches!(target, Target::PlaylistSong(_));
                let h = self.column_height;
                v.on_songs = songs;
                if songs {
                    v.select_song(*i, h);
                } else {
                    v.select_list(*i, h);
                }
                let items = self.ticked_menu().unwrap_or_else(|| self.playlist_menu_items(songs));
                self.menu = Some(OpenMenu::new(items, Anchor::Click { col: m.column, row: m.row }));
            } else if let (Target::QueueRow(i), true) = (&target, self.queue_view.is_some()) {
                let (len, h) = (self.status.queue.len(), self.column_height);
                if let Some(v) = self.queue_view.as_mut() {
                    v.select(*i, len, h);
                }
                let items = self.ticked_menu().unwrap_or_else(|| self.queue_menu_items());
                if !items.is_empty() {
                    self.menu = Some(OpenMenu::new(items, Anchor::Click { col: m.column, row: m.row }));
                }
            } else if let (Target::LibraryRow { col, row }, true) = (&target, self.library.is_some()) {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.select(*col, *row, h);
                }
                let items = self.ticked_menu().unwrap_or_else(|| self.library_menu_items());
                if !items.is_empty() {
                    self.menu = Some(OpenMenu::new(items, Anchor::Click { col: m.column, row: m.row }));
                }
            } else if target == Target::Volume {
                self.set_volume(if self.engine.volume() > 0.0 { 0.0 } else { self.settings.volume });
            }
            return;
        }
        if button == MouseButton::Middle {
            if let Target::Row { col, row } = target {
                self.select(col, row);
                self.run(Action::PlaySelection);
            }
            return;
        }

        let double = self.is_double(&target);
        match target {
            Target::Menu(i) => self.click_menu(i),
            Target::Back | Target::Forward if self.in_view() => self.leave_view(),
            Target::Back => self.nav_back(),
            Target::Forward => self.nav_forward(),
            Target::Crumb(p) => {
                self.solo = false;
                self.back.push(self.current_dir());
                self.forward.clear();
                self.navigate_to(&p);
            }
            Target::ToolShuffle => self.run(Action::ShuffleSelection),
            Target::ToolPin if self.in_view() => {}
            Target::ToolPin => {
                let dir = match self.selected_entry() {
                    Some(e) if e.kind == Kind::Folder => e.path.clone(),
                    _ => self.current_dir(),
                };
                self.toggle_pin(dir);
            }
            Target::PlaylistPane(pane) => {
                if let Some(v) = self.playlists.as_mut() {
                    v.on_songs = pane == 1 && !v.songs.is_empty();
                }
            }
            Target::PlaylistRow(i) => {
                let h = self.column_height;
                if let Some(v) = self.playlists.as_mut() {
                    v.on_songs = false;
                    v.select_list(i, h);
                }
                if double {
                    self.play_playlist(false);
                }
            }
            Target::QueuePane => {}
            Target::LibraryPane(col) => {
                if let Some(v) = self.library.as_mut() {
                    v.focus = col.min(library::SONGS);
                }
            }
            Target::LibraryRow { col, row } => {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.select(col, row, h);
                }
                if double {
                    self.play_library(false);
                }
            }
            Target::LibraryOpen(row) => {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.select(2, row, h);
                    v.show_songs(h);
                }
            }
            Target::QueueRow(i) => {
                let (len, h) = (self.status.queue.len(), self.column_height);
                if let Some(v) = self.queue_view.as_mut() {
                    v.select(i, len, h);
                }
                if double {
                    self.drag = None;
                    self.queue_play();
                } else {
                    self.drag = Some(Target::QueueRow(i));
                }
            }
            Target::PlaylistSong(i) => {
                let h = self.column_height;
                if let Some(v) = self.playlists.as_mut() {
                    v.on_songs = true;
                    v.select_song(i, h);
                }
                if double {
                    self.play_playlist(false);
                }
            }
            Target::Row { col, row } => {
                if double {
                    self.select(col, row);
                    if let Some(e) = self.selected_entry().cloned() {
                        self.play_entry(&e, col, false);
                    }
                } else {
                    self.select(col, row);
                    let c = &self.levels[col];
                    if c.selected.and_then(|i| c.entries.get(i)).is_some_and(|e| e.kind == Kind::Track) {
                        self.drag = Some(Target::Row { col, row: c.selected.unwrap_or(row) });
                    }
                }
            }
            Target::Column(col) => {
                if col < self.levels.len() {
                    self.focus = col;
                }
            }
            Target::Prev => self.engine.previous(),
            Target::Toggle => self.run(Action::PlayPause),
            Target::Next => self.engine.next(),
            Target::Seek => {
                self.drag = Some(Target::Seek);
                self.mouse_drag(m);
            }
            Target::Volume => {
                self.drag = Some(Target::Volume);
                self.mouse_drag(m);
            }
            Target::EqBadge => self.run(Action::Equalizer),
            Target::Divider(d) if double => self.reset_divider(d),
            Target::Divider(d) => {
                let edge = self.divider_edge(d).unwrap_or(0) as i32;
                let at = if d == Divider::Visual { m.row } else { m.column } as i32;
                self.drag_grab = at - edge;
                self.drag = Some(Target::Divider(d));
            }
            _ => {}
        }
    }

    pub(super) fn mouse_drag(&mut self, m: MouseEvent) {
        let frac = |r: Rect| -> f64 {
            if r.width <= 1 {
                return 0.0;
            }
            ((m.column.saturating_sub(r.x)) as f64 / (r.width - 1) as f64).clamp(0.0, 1.0)
        };
        match self.drag {
            Some(Target::Seek) => {
                if self.status.state != State::Stopped && self.status.duration > 0.0 {
                    self.drag_seek = Some(frac(self.seek_rect) * self.status.duration);
                }
            }
            Some(Target::Volume) => {
                let bar = self.volume_rect;
                self.set_volume(slider::volume_at(bar.x, bar.width, m.column));
            }
            Some(Target::Row { col, row: from }) => self.drag_song(col, from, m),
            Some(Target::Tick(start)) => {
                let (Some((_, on)), Some(row)) = (self.tick_drag, self.hit_test(m.column, m.row).as_ref().and_then(TickRow::of)) else { return };
                if std::mem::discriminant(&row) == std::mem::discriminant(&start) && row == start.at(row.row()) {
                    self.tick_range(start, row, on);
                }
            }
            Some(Target::Divider(d)) => {
                self.last_click = None;
                if d == Divider::Visual {
                    self.set_visual_edge(m.row as i32 - self.drag_grab);
                } else {
                    self.set_divider_edge(d, m.column as i32 - self.drag_grab);
                }
            }
            Some(Target::QueueRow(from)) => {
                if let (Some(Target::QueueRow(to)), Some(song)) = (self.hit_test(m.column, m.row), self.status.queue.get(from).cloned()) {
                    if to != from {
                        self.engine.move_in_queue(from, to, song);
                        self.settle_engine();
                        self.drag = Some(Target::QueueRow(to));
                        let (len, h) = (self.status.queue.len(), self.column_height);
                        if let Some(v) = self.queue_view.as_mut() {
                            v.select(to, len, h);
                        }
                    }
                }
            }
            Some(Target::EqBand(_)) => {
                let band = match self.hit_test(m.column, self.eq_rect.y) {
                    Some(Target::EqBand(b)) => b,
                    _ => self.eq_band,
                };
                self.eq_band = band;
                let db = self.eq_db_at(m.row);
                self.eq_set(band, db);
            }
            _ => {}
        }
    }

    pub(super) fn wheel(&mut self, target: Option<Target>, dir: isize) {
        let target = target.map(|t| if let Target::Tick(r) = t { r.row_target() } else { t });
        let lines = WHEEL_LINES as isize * dir;
        if let (Some(m), Some(Target::MenuItem(_) | Target::Overlay)) = (self.menu.as_mut(), &target) {
            let max = m.items.len().saturating_sub(1) as isize;
            m.offset = (m.offset as isize + lines).clamp(0, max) as usize;
            m.hover = None;
            return;
        }
        match target {
            Some(Target::Scope) if self.overlay.is_none() && self.menu.is_none() => {
                if let Some(s) = self.visuals.scopes_if_shown() {
                    s.wheel(dir < 0);
                }
            }
            Some(Target::Row { col, .. } | Target::Column(col)) if self.overlay.is_none() && self.menu.is_none() => {
                if let Some(c) = self.levels.get_mut(col) {
                    let max = c.entries.len().saturating_sub(self.column_height);
                    c.offset = (c.offset as isize + lines).clamp(0, max as isize) as usize;
                }
            }
            Some(Target::QueueRow(_) | Target::QueuePane) if self.overlay.is_none() && self.menu.is_none() => {
                let max = self.status.queue.len().saturating_sub(self.column_height);
                if let Some(v) = self.queue_view.as_mut() {
                    v.offset = (v.offset as isize + lines).clamp(0, max as isize) as usize;
                }
            }
            Some(Target::LibraryRow { col, .. } | Target::LibraryPane(col)) if self.overlay.is_none() && self.menu.is_none() => {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.scroll(col, lines, h);
                }
            }
            Some(Target::LibraryOpen(_)) if self.overlay.is_none() && self.menu.is_none() => {
                let h = self.column_height;
                if let Some(v) = self.library.as_mut() {
                    v.scroll(2, lines, h);
                }
            }
            Some(Target::PlaylistRow(_) | Target::PlaylistSong(_) |
Target::PlaylistPane(_)) if self.overlay.is_none() => {
                let rows = self.column_height;
                if let Some(v) = self.playlists.as_mut() {
                    let songs = matches!(target, Some(Target::PlaylistSong(_) | Target::PlaylistPane(1)));
                    let (len, offset) = if songs { (v.songs.len(), &mut v.song_offset) } else { (v.files.len(), &mut v.offset) };
                    let max = len.saturating_sub(rows);
                    *offset = (*offset as isize + lines).clamp(0, max as isize) as usize;
                }
            }
            Some(Target::Volume) => self.set_volume(self.engine.volume() - dir as f32 * 0.01),
            Some(Target::EqBand(b)) => {
                self.eq_band = b;
                self.eq_nudge(b, -dir as f32 * 0.5);
            }
            Some(Target::Seek) => self.run(Action::SeekBy(-dir as f64 * 5.0)),
            Some(Target::SearchRow(_) | Target::SearchList) => {
                let h = self.list_height;
                if let Some(s) = self.search_mut() {
                    s.scroll(lines, h);
                }
            }
            Some(Target::ThemeRow(_) | Target::ThemeList) => {
                if let Some(Overlay::Theme(p)) = self.overlay.as_mut() {
                    let max = p.list.len().saturating_sub(self.list_height);
                    p.offset = (p.offset as isize + lines).clamp(0, max as isize) as usize;
                }
            }
            Some(Target::OpenRow(_) | Target::OpenList) => {
                if let Some(Overlay::Open(d)) = self.overlay.as_mut() {
                    let max = (d.entries.len() + 1).saturating_sub(self.list_height);
                    d.offset = (d.offset as isize + lines).clamp(0, max as isize) as usize;
                }
            }
            _ => {}
        }
    }

    pub(super) fn overlay_mouse(&mut self, button: MouseButton, target: Option<Target>, m: MouseEvent) {
        if button == MouseButton::Right {
            if let (Some(Overlay::Equalizer), Some(Target::EqBand(b))) = (&self.overlay, &target) {
                self.eq_set(*b, 0.0);
            }
            if let (Some(Overlay::Search(_)), Some(Target::SearchRow(i))) = (&self.overlay, &target) {
                let i = *i;
                self.search_select(i);
                self.search_menu(Some(Anchor::Click { col: m.column, row: m.row }));
            }
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        let double = target.as_ref().is_some_and(|t| self.is_double(t));
        let importing = matches!(&self.overlay, Some(Overlay::Open(d)) if d.mode == OpenMode::ImportPlaylists);
        match (self.overlay.as_ref(), target) {
            (Some(Overlay::Open(_)), Some(Target::OpenRow(i))) if importing => {
                self.open_dialog_select(i);
                let file = match &self.overlay {
                    Some(Overlay::Open(d)) => i.checked_sub(1).and_then(|k| d.entries.get(k)).filter(|e| e.kind == Kind::Track).map(|e| e.path.clone()),
                    _ => None,
                };
                match file {
                    Some(path) if double => self.import_playlists(vec![path]),
                    Some(_) => self.import_toggle(Some(i)),
                    None if double => self.open_dialog_enter(),
                    None => {}
                }
            }
            (Some(Overlay::Equalizer), Some(Target::EqBand(b))) => {
                self.key_focus = None;
                self.eq_band = b;
                if double {
                    self.eq_set(b, 0.0);
                } else {
                    let db = self.eq_db_at(m.row);
                    self.eq_set(b, db);
                    self.drag = Some(Target::EqBand(b));
                }
            }
            (Some(Overlay::Equalizer), Some(Target::EqReset)) => self.eq_reset_song(),
            (Some(Overlay::Equalizer), Some(Target::EqPreset)) => self.eq_preset_menu(),
            (Some(Overlay::Equalizer), Some(Target::EqToggle)) => self.eq_toggle(),
            (Some(Overlay::Equalizer), Some(Target::EqClose | Target::Backdrop)) => self.close_eq(),
            (Some(Overlay::Equalizer), Some(Target::Prev)) => self.engine.previous(),
            (Some(Overlay::Equalizer), Some(Target::Toggle)) => self.run(Action::PlayPause),
            (Some(Overlay::Equalizer), Some(Target::Next)) => self.engine.next(),
            (Some(Overlay::Equalizer), Some(Target::Seek)) => {
                self.drag = Some(Target::Seek);
                self.mouse_drag(m);
            }
            (Some(Overlay::Equalizer), Some(Target::Volume)) => {
                self.drag = Some(Target::Volume);
                self.mouse_drag(m);
            }
            (Some(Overlay::About), Some(Target::AboutLink)) => self.open_about_link(),
            (Some(Overlay::Help | Overlay::About), _) => self.overlay = None,
            (Some(Overlay::PlaylistName(..)), Some(Target::PlaylistNameOk)) => self.submit_playlist_name(),
            (Some(Overlay::PlaylistName(_, what)),
Some(Target::PlaylistNameCancel | Target::Backdrop)) => {
                self.overlay = if what.is_eq() { Some(Overlay::Equalizer) } else { None }
            }
            (Some(Overlay::Search(_)), Some(Target::SearchRow(i))) => {
                self.search_select(i);
                if double {
                    self.run(Action::SearchPlay);
                }
            }
            (Some(Overlay::Search(_)), Some(Target::SearchPlay)) => self.run(Action::SearchPlay),
            (Some(Overlay::Search(_)), Some(Target::SearchClose | Target::Backdrop)) => {
                self.overlay = None;
            }
            (Some(Overlay::Theme(_)), Some(Target::ThemeRow(i))) => {
                self.theme_select(i);
                if double {
                    self.theme_finish(true);
                }
            }
            (Some(Overlay::Theme(_)), Some(Target::ThemeKeep)) => self.theme_finish(true),
            (Some(Overlay::Theme(_)), Some(Target::ThemeCancel | Target::Backdrop)) => {
                self.theme_finish(false);
            }
            (Some(Overlay::Open(_)), Some(Target::OpenRow(i))) => {
                self.open_dialog_select(i);
                if double {
                    self.open_dialog_enter();
                }
            }
            (Some(Overlay::Open(_)), Some(Target::OpenPlace(i))) => {
                let p = match &self.overlay {
                    Some(Overlay::Open(d)) => d.places.get(i).map(|x| x.1.clone()),
                    _ => None,
                };
                if let Some(p) = p {
                    if p.is_dir() {
                        self.open_dialog_go(p);
                    } else {
                        self.say("that folder isn't available right now");
                    }
                }
            }
            (Some(Overlay::Open(_)), Some(Target::OpenPath)) => {
                if let Some(Overlay::Open(d)) = self.overlay.as_mut() {
                    d.editing_path = true;
                }
            }
            (Some(Overlay::Open(_)), Some(Target::OpenButton(b))) => self.open_dialog_button(b),
            (Some(Overlay::Open(_)), Some(Target::Backdrop)) => self.overlay = None,
            _ => {}
        }
    }
}
