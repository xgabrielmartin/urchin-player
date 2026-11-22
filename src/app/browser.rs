use super::*;

impl App {
    pub fn check_solo(&mut self) {
        self.check_resume_solo();
        let Some(serial) = self.solo_for else { return };
        if self.in_view() {
            return;
        }
        if self.status.plays >= serial && self.status.state == State::Playing && self.status.position >= SOLO_AFTER_SECS {
            self.solo_for = None;
            self.enter_solo();
        }
    }

    fn check_resume_solo(&mut self) {
        if self.solo {
            self.solo_seen = true;
            self.resume_solo = None;
            return;
        }
        let song = self.status.current().cloned();
        if std::mem::take(&mut self.solo_seen) {
            if let (State::Playing, Some(song)) = (self.status.state, song.clone()) {
                if self.solo_for.is_none() {
                    self.resume_solo = Some((self.status.plays, song, self.status.position + SOLO_AFTER_SECS));
                }
            }
            return;
        }
        let Some((serial, expected, at)) = self.resume_solo.clone() else { return };
        if self.status.plays != serial || song.as_ref() != Some(&expected) || self.solo_for.is_some() {
            self.resume_solo = None;
            return;
        }
        if self.in_view() || self.overlay.is_some() || self.menu.is_some() || self.status.state != State::Playing {
            return;
        }
        if self.status.position >= at {
            self.resume_solo = None;
            self.enter_solo();
            self.solo_seen = true;
        }
    }

    pub(super) fn touch_resume_solo(&mut self) {
        if let Some((_, _, at)) = self.resume_solo.as_mut() {
            *at = self.status.position + SOLO_AFTER_SECS;
        }
    }

    pub(super) fn listening_in_this_folder(&self) -> bool {
        self.status.state == State::Playing
            && self.status.current().and_then(|song| song.parent()).is_some_and(|folder| folder == self.levels[self.focus].dir.as_path())
    }

    pub(super) fn enter_solo(&mut self) {
        let Some(song) = self.status.current().cloned() else { return };
        let Some(folder) = song.parent().map(|p| p.to_path_buf()) else { return };
        if self.levels[self.focus].dir != folder {
            self.navigate_to(&folder);
        }
        if let Some(i) = self.levels[self.focus].entries.iter().position(|e| e.path == song) {
            self.select(self.focus, i);
        }
        self.solo = true;
    }

    pub fn toolbar_hidden(&self) -> bool {
        self.settings.compact && (self.solo || !self.toolbar_woken) && !self.in_view() && !self.toolbar_peeking()
    }

    fn toolbar_peeking(&self) -> bool {
        self.pointer_row == Some(self.toolbar_row) && self.toolbar_peek_until.is_some_and(|t| Instant::now() < t)
    }

    pub fn current_dir(&self) -> PathBuf {
        self.levels[self.focus].dir.clone()
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        let col = &self.levels[self.focus];
        col.selected.and_then(|i| col.entries.get(i))
    }

    pub(super) fn sync_preview(&mut self) {
        self.levels.truncate(self.focus + 1);
        let show_hidden = self.settings.show_hidden;
        if let Some(e) = self.selected_entry().cloned() {
            if e.kind == Kind::Folder {
                self.levels.push(Column::new(e.path, show_hidden, &self.order));
            }
        }
    }

    pub(crate) fn select(&mut self, col: usize, row: usize) {
        if col >= self.levels.len() {
            return;
        }
        self.focus = col;
        let c = &mut self.levels[col];
        if c.entries.is_empty() {
            return;
        }
        let row = row.min(c.entries.len() - 1);
        c.selected = Some(row);
        self.ensure_visible(col);
        self.sync_preview();
    }

    pub(super) fn ensure_visible(&mut self, col: usize) {
        let h = self.column_height.max(1);
        let c = &mut self.levels[col];
        if let Some(s) = c.selected {
            if s < c.offset {
                c.offset = s;
            } else if s >= c.offset + h {
                c.offset = s + 1 - h;
            }
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let col = self.focus;
        let len = self.levels[col].entries.len();
        if len == 0 {
            return;
        }
        let cur = self.levels[col].selected.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.select(col, next);
    }

    pub(super) fn enter_selected(&mut self) {
        if let Some(e) = self.selected_entry().cloned() {
            if e.kind == Kind::Folder && self.levels.len() > self.focus + 1 {
                self.back.push(self.current_dir());
                self.forward.clear();
                self.focus += 1;
                if !self.levels[self.focus].entries.is_empty() && self.levels[self.focus].selected.is_none() {
                    self.levels[self.focus].selected = Some(0);
                    self.sync_preview();
                }
            }
        }
    }

    pub fn can_go_up(&self) -> bool {
        self.focus > 0 || self.root.parent().is_some()
    }

    pub(super) fn go_up(&mut self) {
        if self.focus > 0 {
            self.back.push(self.current_dir());
            self.forward.clear();
            self.focus -= 1;
            self.sync_preview();
        } else if let Some(parent) = self.root.parent().map(|p| p.to_path_buf()) {
            let child = self.root.clone();
            self.back.push(child.clone());
            self.forward.clear();
            self.set_root(parent);
            if let Some(i) = self.levels[0].entries.iter().position(|e| e.path == child) {
                self.select(0, i);
            }
        }
    }

    pub fn can_go_down(&self) -> bool {
        self.levels.len() > self.focus + 1 && self.selected_entry().is_some_and(|e| e.kind == Kind::Folder)
    }

    pub(super) fn nav_back(&mut self) {
        self.solo = false;
        self.go_up();
    }

    pub(super) fn nav_forward(&mut self) {
        self.solo = false;
        self.enter_selected();
    }

    pub(super) fn set_root(&mut self, root: PathBuf) {
        self.root = root.clone();
        self.levels = vec![Column::new(root, self.settings.show_hidden, &self.order)];
        self.focus = 0;
        if !self.levels[0].entries.is_empty() {
            self.levels[0].selected = Some(0);
        }
        self.sync_preview();
    }

    pub fn navigate_to(&mut self, dir: &Path) {
        let rel = dir.strip_prefix(&self.root).ok().map(|r| r.to_path_buf());
        let Some(rel) = rel else {
            self.set_root(dir.to_path_buf());
            return;
        };
        self.levels.truncate(1);
        self.focus = 0;
        let mut col = 0;
        for comp in rel.components() {
            let name = comp.as_os_str().to_string_lossy().to_string();
            let Some(i) = self.levels[col].entries.iter().position(|e| e.name == name) else { break };
            self.select(col, i);
            if self.levels.len() <= col + 1 {
                break;
            }
            col += 1;
            self.focus = col;
        }
        let c = &mut self.levels[self.focus];
        if c.selected.is_none() && !c.entries.is_empty() {
            c.selected = Some(0);
            self.sync_preview();
        }
    }

    pub(super) fn history_go(&mut self, back: bool) {
        let (from, to) = if back { (&mut self.back, &mut self.forward) } else { (&mut self.forward, &mut self.back) };
        if let Some(dir) = from.pop() {
            to.push(self.levels[self.focus].dir.clone());
            self.navigate_to(&dir);
        }
    }

    pub fn open_folder(&mut self, dir: &Path) {
        let dir = dir.to_path_buf();
        self.playlists = None;
        self.queue_view = None;
        self.library = None;
        self.solo = false;
        self.back.push(self.current_dir());
        self.forward.clear();
        self.navigate_to(&dir);
        self.settings.touch_recent(&dir);
        self.settings.save();
    }

    pub(super) fn watch_folders(&mut self) {
        let Some(until) = self.watch_until else { return };
        if Instant::now() >= until {
            self.watch_until = None;
        } else if self.dir_check.elapsed() >= DIR_CHECK {
            self.check_folders();
        }
    }

    pub fn on_focus_lost(&mut self) {
        self.pointer_row = None;
        self.toolbar_peek_until = None;
        self.menu_peek = false;
    }

    pub fn on_focus(&mut self) {
        if self.watch_until.take().is_some() {
            self.check_folders();
        }
    }

    pub(crate) fn check_folders(&mut self) {
        self.dir_check = Instant::now();
        let now: Vec<(PathBuf, Option<SystemTime>)> = self.levels.iter().map(|c| (c.dir.clone(), dir_time(&c.dir))).collect();
        let same_folders = now.len() == self.dir_stamps.len() && now.iter().zip(&self.dir_stamps).all(|(a, b)| a.0 == b.0);
        let changed = same_folders && now != self.dir_stamps;
        self.dir_stamps = now;
        if changed {
            self.reread_folders();
            self.dir_stamps = self.levels.iter().map(|c| (c.dir.clone(), dir_time(&c.dir))).collect();
        }
    }

    pub(super) fn reread_folders(&mut self) {
        let show_hidden = self.settings.show_hidden;
        let mut renamed: Option<(PathBuf, PathBuf)> = None;
        for c in &mut self.levels {
            if let Some((from, to)) = &renamed {
                if let Ok(rest) = c.dir.strip_prefix(from) {
                    c.dir = to.join(rest);
                }
            }
            let fresh = fsx::list_in(&c.dir, show_hidden, &self.order);
            renamed = None;
            if fresh.len() == c.entries.len() && fresh.iter().zip(&c.entries).all(|(a, b)| a.path == b.path && a.kind == b.kind) {
                continue;
            }
            let gone: Vec<&PathBuf> = c.entries.iter().map(|e| &e.path).filter(|p| !fresh.iter().any(|f| &f.path == *p)).collect();
            let added: Vec<&PathBuf> = fresh.iter().map(|e| &e.path).filter(|p| !c.entries.iter().any(|o| &o.path == *p)).collect();
            if let ([from], [to]) = (gone.as_slice(), added.as_slice()) {
                renamed = Some(((*from).clone(), (*to).clone()));
            }
            let follow = |p: &PathBuf| match &renamed {
                Some((from, to)) if p == from => to.clone(),
                _ => p.clone(),
            };
            let selected = c.selected.and_then(|i| c.entries.get(i)).map(|e| follow(&e.path));
            let ticked: Vec<PathBuf> = c.marks.iter().filter_map(|&i| c.entries.get(i)).map(|e| follow(&e.path)).collect();
            let was = c.selected;
            c.entries = fresh;
            let last = c.entries.len().checked_sub(1);
            c.selected = selected.and_then(|p| c.entries.iter().position(|e| e.path == p)).or_else(|| was.zip(last).map(|(i, l)| i.min(l)));
            let mut marks = crate::marks::Marks::default();
            for p in ticked {
                if let Some(i) = c.entries.iter().position(|e| e.path == p) {
                    marks.set(i, true);
                }
            }
            marks.anchor = None;
            c.marks = marks;
            c.offset = c.offset.min(last.unwrap_or(0));
        }
    }

    pub(super) fn refresh(&mut self) {
        let show_hidden = self.settings.show_hidden;
        for c in &mut self.levels {
            let selected_path = c.selected.and_then(|i| c.entries.get(i)).map(|e| e.path.clone());
            c.entries = fsx::list_in(&c.dir, show_hidden, &self.order);
            c.selected = selected_path.and_then(|p| c.entries.iter().position(|e| e.path == p));
        }
    }

    pub(super) fn play_entry(&mut self, e: &Entry, col: usize, shuffle: bool) {
        match e.kind {
            Kind::Folder => {
                let mut tracks = fsx::collect_tracks_in(&e.path, &self.order);
                if tracks.is_empty() {
                    self.say(format!("no playable files in {}", e.name));
                    return;
                }
                if shuffle {
                    self.mix(&mut tracks);
                }
                self.solo_for = Some(self.engine.play(tracks, 0));
                self.follow = Some((e.path.clone(), shuffle));
                self.settings.touch_recent(&e.path);
                self.settings.save();
            }
            Kind::Track => {
                let tracks: Vec<PathBuf> = self.levels[col]
                    .entries
                    .iter()
                    .filter(|x| x.kind == Kind::Track)
                    .map(|x| x.path.clone())
                    .collect();
                let index = tracks.iter().position(|p| *p == e.path).unwrap_or(0);
                self.solo_for = Some(self.engine.play(tracks, index));
                self.follow = Some((self.levels[col].dir.clone(), false));
            }
        }
    }

    pub(super) fn move_song(&mut self, delta: isize) {
        if self.in_view() {
            return;
        }
        let col = self.focus;
        let ticked = self.levels.get(col).map(|c| c.marks.sorted()).unwrap_or_default();
        if !ticked.is_empty() {
            self.move_ticked_songs(col, &ticked, delta);
            return;
        }
        let Some(from) = self.levels.get(col).and_then(|c| c.selected) else { return };
        if let Ok(to) = usize::try_from(from as isize + delta) {
            self.move_song_to(col, from, to);
        }
    }

    pub(super) fn move_ticked_songs(&mut self, col: usize, ticked: &[usize], delta: isize) {
        let Some(c) = self.levels.get_mut(col) else { return };
        let is_song = |i: usize| c.entries.get(i).is_some_and(|e| e.kind == Kind::Track);
        if !ticked.iter().all(|&i| is_song(i)) {
            self.say("only songs can be moved: untick the folders");
            return;
        }
        let moves = crate::marks::block_moves(ticked, delta, c.entries.len(), is_song);
        for &(from, to) in &moves {
            c.entries.swap(from, to);
            c.marks.moved(from, to);
            if c.selected == Some(from) {
                c.selected = Some(to);
            } else if c.selected == Some(to) {
                c.selected = Some(from);
            }
        }
        if moves.is_empty() {
            return;
        }
        let names = c.entries.iter().filter(|e| e.kind == Kind::Track).map(|e| e.name.clone()).collect();
        let dir = c.dir.clone();
        self.order.set(&dir, names);
        self.ensure_visible(col);
    }

    pub(super) fn move_song_to(&mut self, col: usize, from: usize, to: usize) -> bool {
        let Some(c) = self.levels.get_mut(col) else { return false };
        let is_song = |i: usize| c.entries.get(i).is_some_and(|e| e.kind == Kind::Track);
        if from == to || !is_song(from) || !is_song(to) {
            return false;
        }
        let entry = c.entries.remove(from);
        c.entries.insert(to, entry);
        c.marks.moved(from, to);
        c.selected = Some(to);
        let names = c.entries.iter().filter(|e| e.kind == Kind::Track).map(|e| e.name.clone()).collect();
        let dir = c.dir.clone();
        self.order.set(&dir, names);
        self.ensure_visible(col);
        true
    }

    pub(super) fn drag_song(&mut self, col: usize, from: usize, m: MouseEvent) {
        let rows: Vec<(usize, Rect)> = self
            .hits
            .iter()
            .filter_map(|h| match h.target {
                Target::Row { col: c, row } if c == col => Some((row, h.rect)),
                _ => None,
            })
            .collect();
        let (Some(top), Some(bottom)) = (rows.iter().min_by_key(|r| r.1.y), rows.iter().max_by_key(|r| r.1.y)) else { return };
        let to = if m.row < top.1.y {
            top.0.saturating_sub(1)
        } else if m.row > bottom.1.y {
            bottom.0 + 1
        } else if let Some(Target::Row { col: c, row }) = self.hit_test(m.column, m.row) {
            if c != col {
                return;
            }
            row
        } else {
            return;
        };
        if self.move_song_to(col, from, to) {
            self.drag = Some(Target::Row { col, row: to });
        }
    }

    pub(super) fn reset_order(&mut self) {
        let Some(dir) = self.levels.get(self.focus).map(|c| c.dir.clone()) else { return };
        if self.order.reset(&dir) {
            self.refresh();
            self.say("songs back in name order");
        } else {
            self.say("this folder is already in name order");
        }
    }

    pub(super) fn toggle_pin(&mut self, dir: PathBuf) {
        if let Some(i) = self.settings.pinned.iter().position(|p| *p == dir) {
            self.settings.pinned.remove(i);
            self.say("unpinned");
        } else {
            self.settings.pinned.push(dir);
            self.say("pinned to File menu and Open Folder");
        }
        self.settings.save();
    }
}
