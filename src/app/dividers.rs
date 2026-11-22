use super::*;

impl App {
    pub(super) fn divider_edge(&self, d: Divider) -> Option<u16> {
        self.dividers.iter().find(|(e, _)| *e == d).map(|&(_, at)| at)
    }

    pub(super) fn set_divider_edge(&mut self, d: Divider, x: i32) {
        let area = self.panes_rect;
        let edge = x - area.x as i32;
        match d {
            Divider::Folders => crate::ui::move_split(area.width, std::slice::from_mut(&mut self.settings.folder_split), 0, edge),
            Divider::Playlists => crate::ui::move_split(area.width, std::slice::from_mut(&mut self.settings.playlist_split), 0, edge),
            Divider::Library(i) => crate::ui::move_split(area.width, &mut self.settings.library_split, i, edge),
            Divider::Visual => {}
        }
    }

    pub(super) fn set_visual_edge(&mut self, y: i32) {
        let Some((lo, hi)) = crate::ui::visual_rows_range(self.screen_h, self.settings.flipped, self.settings.compact) else { return };
        let an = self.analyzer_rect;
        let rows = if self.settings.flipped {
            (an.y + an.height) as i32 - (y + 1)
        } else {
            y - an.y as i32
        };
        let rows = rows - self.settings.visual.slot_shift() as i32;
        self.settings.visual_rows = rows.clamp(lo as i32, hi as i32) as u16;
    }

    pub(super) fn reset_divider(&mut self, d: Divider) {
        let start = crate::settings::Settings::default();
        match d {
            Divider::Folders => self.settings.folder_split = start.folder_split,
            Divider::Playlists => self.settings.playlist_split = start.playlist_split,
            Divider::Library(_) => self.settings.library_split = start.library_split,
            Divider::Visual => self.settings.visual_rows = 0,
        }
        self.drag = None;
        self.settings.save();
    }

    pub(super) fn key_divider(&self) -> Option<Divider> {
        let side = |d: &Divider| *d != Divider::Visual;
        if let Some(v) = self.library.as_ref() {
            let want = Divider::Library(v.focus.min(1));
            if self.divider_edge(want).is_some() {
                return Some(want);
            }
        }
        self.dividers.iter().map(|(d, _)| *d).find(side)
    }

    pub(super) fn nudge_divider(&mut self, by: i32) {
        let Some(d) = self.key_divider() else { return };
        let Some(edge) = self.divider_edge(d) else { return };
        self.set_divider_edge(d, edge as i32 + by);
        if let Some(e) = self.dividers.iter_mut().find(|(e, _)| *e == d) {
            let w = self.panes_rect;
            let widths = match d {
                Divider::Folders => crate::ui::pane_widths(w.width, &[self.settings.folder_split]),
                Divider::Playlists => crate::ui::pane_widths(w.width, &[self.settings.playlist_split]),
                Divider::Library(_) => crate::ui::pane_widths(w.width, &self.settings.library_split),
                Divider::Visual => return,
            };
            let i = if let Divider::Library(i) = d { i } else { 0 };
            e.1 = w.x + widths[..=i].iter().sum::<u16>();
        }
    }

    pub(super) fn nudge_visual(&mut self, by: i32) {
        if self.analyzer_rect.height == 0 {
            return;
        }
        let Some(edge) = self.divider_edge(Divider::Visual) else { return };
        self.set_visual_edge(edge as i32 + by);
        let Some((lo, hi)) = crate::ui::visual_rows_range(self.screen_h, self.settings.flipped, self.settings.compact) else { return };
        let slot = self.settings.visual_rows.clamp(lo, hi);
        let new = crate::ui::shifted_slot(self.screen_h, self.settings.flipped, self.settings.compact, slot, self.settings.visual.slot_shift());
        let old = self.analyzer_rect.height;
        let moved = new as i32 - old as i32;
        let moved_edge = if self.settings.flipped { -moved } else { moved };
        if let Some(e) = self.dividers.iter_mut().find(|(e, _)| *e == Divider::Visual) {
            e.1 = (e.1 as i32 + moved_edge) as u16;
        }
        if self.settings.flipped {
            self.analyzer_rect.y = (self.analyzer_rect.y as i32 - moved) as u16;
        }
        self.analyzer_rect.height = new;
    }

    pub fn pointer_shape(&self) -> &'static str {
        match self.drag.as_ref().or(self.hover.as_ref()) {
            Some(Target::Divider(Divider::Visual)) => "row-resize",
            Some(Target::Divider(_)) => "col-resize",
            _ => "default",
        }
    }
}
