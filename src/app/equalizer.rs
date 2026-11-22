use super::*;

impl App {
    pub fn eq_song(&self) -> Option<PathBuf> {
        self.status.current().cloned()
    }

    pub fn eq_curve(&self) -> Curve {
        self.status.current().map(|s| self.eq_store.lock().unwrap().get(s)).unwrap_or_default()
    }

    pub fn eq_source(&self) -> eq::Source {
        self.status.current().map_or(eq::Source::None, |s| self.eq_store.lock().unwrap().source(s))
    }

    pub(super) fn eq_set(&mut self, band: usize, db: f32) {
        let Some(song) = self.eq_song() else {
            self.say("start a song to adjust its sound");
            return;
        };
        let mut c = self.eq_curve();
        c.set(band, db);
        self.engine.set_song_curve(&song, c);
    }

    pub(super) fn eq_nudge(&mut self, band: usize, delta: f32) {
        let c = self.eq_curve();
        self.eq_set(band, c.get(band) + delta);
    }

    pub(super) fn eq_preset(&mut self, i: usize) {
        let Some(song) = self.eq_song() else {
            self.say("start a song to adjust its sound");
            return;
        };
        let Some(curve) = self.eq_store.lock().unwrap().preset_curve(i as u8) else { return };
        self.engine.set_song_curve(&song, curve);
        let Some(dir) = song.parent().map(Path::to_path_buf) else { return };
        self.engine.set_folder_preset(&dir, Some(i as u8));
        if let Some(m) = self.moves.as_mut() {
            m.folder = Some((dir.clone(), folder_print_songs(&dir), false));
        }
        let others: Vec<PathBuf> = fsx::list(&dir, true)
            .into_iter()
            .filter(|e| e.kind == Kind::Track && e.path != song)
            .map(|e| e.path)
            .collect();
        let keep = {
            let store = self.eq_store.lock().unwrap();
            others.iter().filter(|p| store.has(p)).count()
        };
        let name = self.eq_store.lock().unwrap().preset_name(i as u8);
        let folder = short_name(&dir);
        let follow = others.len() - keep;
        let msg = match (follow, keep) {
            (0, 0) => name,
            (0, k) => format!("{name} for this song ({} in {folder} keep{} their own EQ)", plural(k, "the other song", "the other songs"), if k == 1 { "s" } else { "" }),
            (n, 0) => format!("{name} for this song and {} in {folder}", plural(n, "the other song", "the other songs")),
            (n, k) => format!("{name} for this song and {n} more in {folder} ({k} keep{} their own EQ)", if k == 1 { "s" } else { "" }),
        };
        self.say(msg);
    }

    pub(super) fn eq_cycle_preset(&mut self, dir: isize) {
        let order = self.eq_store.lock().unwrap().preset_order();
        let n = order.len() as isize;
        let at = self.eq_curve().based_on.and_then(|id| order.iter().position(|&o| o == id));
        let next = match at {
            Some(i) => (i as isize + dir).rem_euclid(n),
            None => if dir > 0 { 0 } else { n - 1 },
        };
        self.eq_preset(order[next as usize] as usize);
    }

    pub(super) fn eq_preset_menu(&mut self) {
        let current = self.eq_curve();
        let store = self.eq_store.lock().unwrap();
        let item = |id: u8| MenuItem {
            checked: Some(current.based_on == Some(id)),
            ..MenuItem::new(store.preset_name(id), "", Action::EqPreset(id as usize))
        };
        let mut items = vec![MenuItem::new("Save as Preset…", "S", Action::EqSavePreset), MenuItem::sep()];
        let mine = store.my_presets();
        if !mine.is_empty() {
            items.push(MenuItem::heading("Your Presets"));
            items.extend(mine.into_iter().map(item));
            items.push(MenuItem::heading("Built-in"));
        }
        items.extend(eq::preset_order().into_iter().map(|i| item(i as u8)));
        drop(store);
        let row = items
            .iter()
            .position(|it| it.checked == Some(true))
            .or_else(|| items.iter().position(|it| matches!(it.action, Some(Action::EqPreset(_)))));
        let y = self.eq_rect.y.saturating_sub(3);
        let mut menu = OpenMenu::new(items, Anchor::At { x: self.eq_preset_x, y });
        menu.hover = row;
        self.menu = Some(menu);
    }

    pub(super) fn hovered_my_preset(&self) -> Option<u8> {
        let m = self.menu.as_ref()?;
        let Some(Action::EqPreset(id)) = m.hover.and_then(|h| m.items.get(h)).and_then(|it| it.action.clone()) else { return None };
        let id = u8::try_from(id).ok()?;
        self.eq_store.lock().unwrap().is_mine(id).then_some(id)
    }

    pub(super) fn my_preset_menu(&mut self, id: u8, at: Anchor) {
        let items = vec![
            MenuItem::new("Rename…", "F2", Action::EqRenamePreset(id)),
            MenuItem::new("Delete", "Del", Action::EqDeletePreset(id)),
        ];
        self.menu = Some(OpenMenu::new(items, at));
    }

    pub(super) fn eq_save_preset(&mut self) {
        self.menu = None;
        if self.eq_song().is_none() {
            self.say("start a song and shape its sound, then save it as a preset");
            return;
        }
        let curve = self.eq_curve();
        if curve.is_flat() {
            self.say("the sound is flat; shape it first, then save it as a preset");
            return;
        }
        let name = {
            let store = self.eq_store.lock().unwrap();
            curve.based_on.filter(|&id| store.is_mine(id)).map(|id| store.preset_name(id)).unwrap_or_default()
        };
        self.overlay = Some(Overlay::PlaylistName(name, NameFor::EqPreset));
    }

    pub(super) fn eq_rename_preset(&mut self, id: u8) {
        self.menu = None;
        let name = self.eq_store.lock().unwrap().preset_name(id);
        self.overlay = Some(Overlay::PlaylistName(name, NameFor::EqRename(id)));
    }

    pub(super) fn eq_delete_preset(&mut self, id: u8) {
        let name = self.eq_store.lock().unwrap().preset_name(id);
        let folders = self.eq_store.lock().unwrap().delete_mine(id);
        self.after_preset_change();
        self.say(match folders {
            0 => format!("deleted preset {name}"),
            n => format!("deleted preset {name}; {}", plural(n, "the folder that used it is flat again", "the folders that used it are flat again")),
        });
    }

    pub(super) fn after_preset_change(&mut self) {
        self.engine.refresh_curves();
        self.eq_store.lock().unwrap().save_if_changed();
    }

    pub(super) fn submit_preset_name(&mut self, name: String, what: NameFor) {
        match what {
            NameFor::EqPreset => {
                let Some(song) = self.eq_song() else {
                    self.overlay = Some(Overlay::Equalizer);
                    return;
                };
                let curve = self.eq_curve();
                let saved = self.eq_store.lock().unwrap().save_mine(&name, &curve);
                match saved {
                    Ok(s) => {
                        let id = match s {
                            eq::Saved::New(id) | eq::Saved::Updated(id) => id,
                        };
                        self.engine.set_song_curve(&song, Curve { based_on: Some(id), ..curve });
                        self.after_preset_change();
                        self.overlay = Some(Overlay::Equalizer);
                        let name = self.eq_store.lock().unwrap().preset_name(id);
                        self.say(match s {
                            eq::Saved::New(_) => format!("saved preset {name}"),
                            eq::Saved::Updated(_) => format!("updated preset {name}; folders that use it follow"),
                        });
                    }
                    Err(e) => self.say(e.to_string()),
                }
            }
            NameFor::EqRename(id) => {
                let old = self.eq_store.lock().unwrap().preset_name(id);
                let renamed = self.eq_store.lock().unwrap().rename_mine(id, &name);
                match renamed {
                    Ok(()) => {
                        self.after_preset_change();
                        self.overlay = Some(Overlay::Equalizer);
                        let now = self.eq_store.lock().unwrap().preset_name(id);
                        if now != old {
                            self.say(format!("renamed preset {old} to {now}"));
                        }
                    }
                    Err(e) => self.say(e.to_string()),
                }
            }
            _ => {}
        }
    }

    pub(super) fn eq_focus_step(&mut self, dir: isize) {
        const STOPS: isize = 11 + EQ_BUTTONS.len() as isize;
        let at = match &self.key_focus {
            Some(t) => 11 + EQ_BUTTONS.iter().position(|b| b == t).unwrap_or(0) as isize,
            None => self.eq_band as isize,
        };
        let next = (at + dir).rem_euclid(STOPS);
        if next < 11 {
            self.key_focus = None;
            self.eq_band = next as usize;
        } else {
            self.key_focus = Some(EQ_BUTTONS[(next - 11) as usize].clone());
        }
    }

    pub(super) fn eq_button_step(&mut self, dir: isize) {
        let n = EQ_BUTTONS.len() as isize;
        let at = self.key_focus.as_ref().and_then(|t| EQ_BUTTONS.iter().position(|b| b == t)).unwrap_or(0) as isize;
        self.key_focus = Some(EQ_BUTTONS[(at + dir).rem_euclid(n) as usize].clone());
    }

    pub(super) fn eq_press_focused(&mut self) {
        match self.key_focus.clone() {
            Some(Target::EqPreset) => {
                self.eq_preset_menu();
            }
            Some(Target::EqReset) => self.eq_reset_song(),
            Some(Target::EqToggle) => self.eq_toggle(),
            Some(Target::EqClose) => self.close_eq(),
            _ => {}
        }
    }

    pub(super) fn eq_reset_song(&mut self) {
        if let Some(song) = self.eq_song() {
            self.engine.set_song_curve(&song, Curve::default());
            let folder = self.eq_store.lock().unwrap().folder_preset_for(&song);
            match folder {
                Some(id) => {
                    let name = self.eq_store.lock().unwrap().preset_name(id);
                    self.say(format!("this song follows its folder's preset again: {name}"));
                }
                None => self.say("this song is back to flat"),
            }
        }
    }

    pub(super) fn eq_toggle(&mut self) {
        self.settings.eq_enabled = !self.settings.eq_enabled;
        self.engine.set_eq_enabled(self.settings.eq_enabled);
        self.settings.save();
    }

    pub(super) fn eq_db_at(&self, y: u16) -> f32 {
        let r = self.eq_rect;
        if r.height < 2 {
            return 0.0;
        }
        let frac = (y.saturating_sub(r.y)).min(r.height - 1) as f32 / (r.height - 1) as f32;
        eq::MAX_DB - frac * 2.0 * eq::MAX_DB
    }

    pub(super) fn close_eq(&mut self) {
        self.overlay = None;
        self.key_focus = None;
        self.eq_store.lock().unwrap().save_if_changed();
    }
}
