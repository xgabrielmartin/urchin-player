use super::*;

impl App {
    pub(super) fn focus_stops(&self) -> Vec<Target> {
        let mut found: Vec<(u16, u16, Target)> = self
            .hits
            .iter()
            .filter(|h| {
                matches!(
                    h.target,
                    Target::Menu(_)
                        | Target::Back
                        | Target::Forward
                        | Target::EqBadge
                        | Target::ToolShuffle
                        | Target::ToolPin
                        | Target::Prev
                        | Target::Toggle
                        | Target::Next
                        | Target::Scope
                )
            })
            .map(|h| (h.rect.y, h.rect.x, h.target.clone()))
            .collect();
        found.sort_by_key(|f| (f.0, f.1));
        found.into_iter().map(|f| f.2).collect()
    }

    pub(super) fn focus_step(&mut self, dir: isize) {
        let stops = self.focus_stops();
        if stops.is_empty() {
            return;
        }
        let n = stops.len() as isize;
        let next = match self.key_focus.as_ref().and_then(|t| stops.iter().position(|s| s == t)) {
            Some(i) => (i as isize + dir).rem_euclid(n),
            None if dir > 0 => 0,
            None => n - 1,
        };
        self.key_focus = Some(stops[next as usize].clone());
    }

    pub fn in_view(&self) -> bool {
        self.playlists.is_some() || self.queue_view.is_some() || self.library.is_some()
    }

    pub(super) fn leave_view(&mut self) {
        if let Some(v) = self.library.as_mut() {
            if v.focus == library::SONGS {
                v.leave_songs();
                return;
            }
        }
        self.playlists = None;
        self.queue_view = None;
        self.library = None;
    }

    pub(super) fn press_focused(&mut self) {
        let Some(t) = self.key_focus.take() else { return };
        match t {
            Target::Menu(i) => self.open_menu(i, menu_title_x(i)),
            Target::Scope => self.scope_keys = self.scope_mode().is_some(),
            Target::Back | Target::Forward if self.in_view() => self.leave_view(),
            Target::Back => self.nav_back(),
            Target::Forward => self.nav_forward(),
            Target::ToolShuffle => self.run(Action::ShuffleSelection),
            Target::ToolPin if self.in_view() => {}
            Target::ToolPin => {
                let dir = match self.selected_entry() {
                    Some(e) if e.kind == Kind::Folder => e.path.clone(),
                    _ => self.current_dir(),
                };
                self.toggle_pin(dir);
            }
            Target::Prev => self.engine.previous(),
            Target::Toggle => self.run(Action::PlayPause),
            Target::Next => self.engine.next(),
            Target::EqBadge => self.run(Action::Equalizer),
            _ => {}
        }
    }
}
