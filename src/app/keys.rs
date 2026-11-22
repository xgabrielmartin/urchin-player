use super::*;

impl App {
    pub(super) fn p_key(&mut self) {
        const DOUBLE_TAP: Duration = Duration::from_millis(400);
        let now = Instant::now();
        let double = matches!(self.last_p, Some(t) if now.duration_since(t) < DOUBLE_TAP);
        self.last_p = if double { None } else { Some(now) };
        if double {
            self.engine.restart();
        } else {
            self.run(Action::PlayPause);
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        self.touch_resume_solo();
        self.toolbar_woken = true;
        let k = match (k.code, k.modifiers.contains(KeyModifiers::CONTROL)) {
            (KeyCode::Char('p'), true) => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            (KeyCode::Char('n'), true) => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            (KeyCode::Char('b'), true) => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            (KeyCode::Char('f'), true) => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            _ => k,
        };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);

        if ctrl && k.code == KeyCode::Char('q') {
            self.quit = true;
            return;
        }

        if let Some(m) = self.menu.as_mut() {
            let len = m.items.len();
            let step = |from: usize, d: isize, items: &Vec<MenuItem>| {
                let mut i = from as isize;
                for _ in 0..len {
                    i = (i + d).rem_euclid(len as isize);
                    if items[i as usize].action.is_some() {
                        break;
                    }
                }
                i as usize
            };
            if let (KeyCode::Tab | KeyCode::BackTab, Some(t)) = (k.code, m.title_index) {
                self.menu = None;
                self.key_focus = Some(Target::Menu(t));
                self.focus_step(if k.code == KeyCode::Tab { 1 } else { -1 });
                return;
            }
            if let Some(id) = self.hovered_my_preset() {
                match k.code {
                    KeyCode::Delete => {
                        let at = self.menu.as_ref().and_then(|m| m.hover).unwrap_or(0);
                        self.eq_delete_preset(id);
                        self.eq_preset_menu();
                        if let Some(m) = self.menu.as_mut() {
                            let len = m.items.len();
                            m.hover = (at..len).chain((0..at).rev()).find(|&i| m.items[i].action.is_some());
                        }
                        return;
                    }
                    KeyCode::F(2) => return self.eq_rename_preset(id),
                    KeyCode::Char('m') | KeyCode::Menu => return self.run(Action::EqMyPresetMenu(id)),
                    _ => {}
                }
            }
            let Some(m) = self.menu.as_mut() else { return };
            match k.code {
                KeyCode::Esc => self.put_away_menu_bar(),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.hover = Some(step(m.hover.unwrap_or(len - 1), 1, &m.items)),
                KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => m.hover = Some(step(m.hover.unwrap_or(0), -1, &m.items)),
                KeyCode::Left | KeyCode::Right => {
                    if let Some(t) = m.title_index {
                        let n = MENUS.len();
                        let next = if k.code == KeyCode::Left { (t + n - 1) % n } else { (t + 1) % n };
                        let x = menu_title_x(next);
                        self.open_menu(next, x);
                    }
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(a) = m.hover.and_then(|h| m.items[h].action.clone()) {
                        self.run(a);
                    }
                }
                _ => {}
            }
            return;
        }

        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }

        if self.scope_keys {
            match self.scope_mode() {
                Some(mode) => match self.visuals.scopes().key(mode, k) {
                    crate::scopes::Key::Ignored => {}
                    crate::scopes::Key::Handled => return,
                    crate::scopes::Key::Switched(next) => return self.show_scope(next),
                    crate::scopes::Key::Quit => {
                        self.scope_keys = false;
                        return;
                    }
                },
                None => self.scope_keys = false,
            }
        }

        if k.code == KeyCode::Char('q') && !ctrl && !alt {
            self.quit = true;
            return;
        }

        if k.code == KeyCode::Menu || (k.code == KeyCode::F(10) && k.modifiers.contains(KeyModifiers::SHIFT)) {
            self.open_context_menu_by_key();
            return;
        }

        if k.code == KeyCode::F(10) {
            self.open_menu(0, menu_title_x(0));
            return;
        }
        if alt {
            if let KeyCode::Char(c) = k.code {
                if let Some(i) = MENUS.iter().position(|m| m.to_lowercase().starts_with(c.to_ascii_lowercase())) {
                    self.open_menu(i, menu_title_x(i));
                    return;
                }
            }
        }

        match k.code {
            KeyCode::Char('x') if !alt => {
                if let Some(row) = self.active_row() {
                    self.set_tick(row, None);
                    self.step_active();
                }
                return;
            }
            KeyCode::Char('X') if !ctrl && !alt => {
                if let Some(row) = self.active_row() {
                    self.extend_ticks(row);
                }
                return;
            }
            KeyCode::Esc if self.clear_ticks() => return,
            _ => {}
        }

        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Left if shift && !ctrl && !alt => return self.nudge_divider(-1),
            KeyCode::Right if shift && !ctrl && !alt => return self.nudge_divider(1),
            KeyCode::Char('<') if !ctrl && !alt => return self.nudge_divider(-1),
            KeyCode::Char('>') if !ctrl && !alt => return self.nudge_divider(1),
            KeyCode::Up if alt && !ctrl => return self.nudge_visual(-1),
            KeyCode::Down if alt && !ctrl => return self.nudge_visual(1),
            KeyCode::Char('k' | 'K') if alt && !ctrl => return self.nudge_visual(-1),
            KeyCode::Char('j' | 'J') if alt && !ctrl => return self.nudge_visual(1),
            _ => {}
        }

        let vertical = !ctrl
            && matches!(
                k.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End | KeyCode::Char('j' | 'k')
            );
        if self.in_view() {
        } else if self.solo {
            let goes_elsewhere = matches!(
                k.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Backspace | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h' | 'l')
            ) && !ctrl
                || (k.code == KeyCode::Enter && !self.selected_entry().is_some_and(|e| e.kind == Kind::Track));
            if goes_elsewhere {
                self.solo = false;
            }
        } else if vertical && self.listening_in_this_folder() {
            self.solo = true;
        }
        if !matches!(k.code, KeyCode::Tab | KeyCode::BackTab) {
            if k.code == KeyCode::Enter && self.key_focus.is_some() {
                self.press_focused();
                return;
            }
            self.key_focus = None;
        }
        if self.playlists.is_some() && self.playlists_key(k) {
            return;
        }
        if self.queue_view.is_some() && self.queue_key(k) {
            return;
        }
        if self.library.is_some() && self.library_key(k) {
            return;
        }
        match k.code {
            KeyCode::Tab => self.focus_step(1),
            KeyCode::BackTab => self.focus_step(-1),
            KeyCode::Char('o') if ctrl => self.run(Action::OpenFolder),
            KeyCode::Char('t') if ctrl => self.run(Action::Theme),
            KeyCode::Char('e') if ctrl => self.run(Action::Equalizer),
            KeyCode::F(1) => self.run(Action::Help),
            KeyCode::Up if k.modifiers.contains(KeyModifiers::SHIFT) => self.move_song(-1),
            KeyCode::Down if k.modifiers.contains(KeyModifiers::SHIFT) => self.move_song(1),
            KeyCode::Char('K') if !ctrl => self.move_song(-1),
            KeyCode::Char('J') if !ctrl => self.move_song(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-(self.column_height as isize)),
            KeyCode::PageDown => self.move_selection(self.column_height as isize),
            KeyCode::Home => self.select(self.focus, 0),
            KeyCode::End => self.select(self.focus, usize::MAX),
            KeyCode::Right | KeyCode::Char('l') => self.enter_selected(),
            KeyCode::Left | KeyCode::Char('h') => self.go_up(),
            KeyCode::Backspace => self.history_go(true),
            KeyCode::Enter => self.run(Action::PlaySelection),
            KeyCode::Char(' ') => self.run(Action::PlayPause),
            KeyCode::Char('n') => self.run(Action::Next),
            KeyCode::Char('A') if !ctrl => self.run(Action::NextAlbum),
            KeyCode::Char('p') => self.p_key(),
            KeyCode::Char('b') => self.run(Action::Previous),
            KeyCode::Char('s') => self.run(Action::Stop),
            KeyCode::Char('.') => self.run(Action::SeekBy(10.0)),
            KeyCode::Char(',') => self.run(Action::SeekBy(-10.0)),
            KeyCode::Char('+' | '=') => self.run(Action::VolumeBy(0.01)),
            KeyCode::Char('-') => self.run(Action::VolumeBy(-0.01)),
            KeyCode::Char('a') if !ctrl => self.add_selection_to_playlist(),
            KeyCode::Char('Q') if !ctrl => self.enqueue_selection(false),
            KeyCode::Char('N') if !ctrl => self.enqueue_selection(true),
            KeyCode::Char('W') if !ctrl => self.run(Action::Queue),
            KeyCode::Char('i') if !ctrl => self.run(Action::Library),
            KeyCode::Char('/') if !ctrl => self.run(Action::Search),
            KeyCode::Char('m') if !ctrl => self.open_context_menu_by_key(),
            _ => {}
        }
    }

    pub(super) fn overlay_key(&mut self, k: KeyEvent) {
        match self.overlay.as_mut() {
            Some(Overlay::About) if k.code == KeyCode::Enter => self.open_about_link(),
            Some(Overlay::Help | Overlay::About) => self.overlay = None,
            Some(Overlay::Equalizer) if self.key_focus.is_some() => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_eq(),
                KeyCode::Tab => self.eq_focus_step(1),
                KeyCode::BackTab => self.eq_focus_step(-1),
                KeyCode::Right | KeyCode::Char('l') => self.eq_button_step(1),
                KeyCode::Left | KeyCode::Char('h') => self.eq_button_step(-1),
                KeyCode::Up | KeyCode::Char('k') => self.key_focus = None,
                KeyCode::Enter => self.eq_press_focused(),
                KeyCode::Char(']') => self.eq_cycle_preset(1),
                KeyCode::Char('[') => self.eq_cycle_preset(-1),
                KeyCode::Char('s' | 'S') => self.eq_save_preset(),
                KeyCode::Char(' ') => self.run(Action::PlayPause),
                KeyCode::Char('n') => self.engine.next(),
                KeyCode::Char('A') => self.run(Action::NextAlbum),
                KeyCode::Char('p') => self.p_key(),
                KeyCode::Char('b') => self.engine.previous(),
                _ => {}
            },
            Some(Overlay::Equalizer) => match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.close_eq(),
                KeyCode::Tab => self.eq_focus_step(1),
                KeyCode::BackTab => self.eq_focus_step(-1),
                KeyCode::Left | KeyCode::Char('h') => self.eq_band = self.eq_band.saturating_sub(1),
                KeyCode::Right | KeyCode::Char('l') => self.eq_band = (self.eq_band + 1).min(10),
                KeyCode::Up | KeyCode::Char('k') => self.eq_nudge(self.eq_band, 0.5),
                KeyCode::Down | KeyCode::Char('j') => self.eq_nudge(self.eq_band, -0.5),
                KeyCode::PageUp => self.eq_nudge(self.eq_band, 3.0),
                KeyCode::PageDown => self.eq_nudge(self.eq_band, -3.0),
                KeyCode::Char('0') => self.eq_set(self.eq_band, 0.0),
                KeyCode::Char('r') => self.eq_reset_song(),
                KeyCode::Char(']') => self.eq_cycle_preset(1),
                KeyCode::Char('[') => self.eq_cycle_preset(-1),
                KeyCode::Char('s' | 'S') => self.eq_save_preset(),
                KeyCode::Char('e') => self.eq_toggle(),
                KeyCode::Char(' ') => self.run(Action::PlayPause),
                KeyCode::Char('n') => self.engine.next(),
                KeyCode::Char('A') => self.run(Action::NextAlbum),
                KeyCode::Char('p') => self.p_key(),
                KeyCode::Char('b') => self.engine.previous(),
                _ => {}
            },
            Some(Overlay::Theme(p)) => match k.code {
                KeyCode::Esc => self.theme_finish(false),
                KeyCode::Enter => self.theme_finish(true),
                KeyCode::Up => {
                    let s = p.selected.saturating_sub(1);
                    self.theme_select(s);
                }
                KeyCode::Down => {
                    let s = p.selected + 1;
                    self.theme_select(s);
                }
                KeyCode::PageUp => {
                    let s = p.selected.saturating_sub(self.list_height);
                    self.theme_select(s);
                }
                KeyCode::PageDown => {
                    let s = p.selected + self.list_height;
                    self.theme_select(s);
                }
                KeyCode::Backspace => {
                    p.filter.pop();
                    self.theme_filter();
                }
                KeyCode::Char(c) => {
                    p.filter.push(c);
                    self.theme_filter();
                }
                _ => {}
            },
            Some(Overlay::Search(_)) => self.search_key(k),
            Some(Overlay::PlaylistName(name, what)) => match k.code {
                KeyCode::Esc if what.is_eq() => self.overlay = Some(Overlay::Equalizer),
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => self.submit_playlist_name(),
                KeyCode::Backspace => {
                    name.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && !c.is_control() => name.push(c),
                _ => {}
            },
            Some(Overlay::Open(d)) => {
                if d.editing_path {
                    match k.code {
                        KeyCode::Esc => d.editing_path = false,
                        KeyCode::Tab => {
                            if let Some(c) = fsx::complete_path(&d.path) {
                                d.path = c;
                            }
                        }
                        KeyCode::Backspace => {
                            d.path.pop();
                        }
                        KeyCode::Enter => {
                            let p = PathBuf::from(fsx::expand_tilde(d.path.trim()));
                            d.editing_path = false;
                            if p.is_dir() {
                                self.open_dialog_go(p);
                            } else {
                                self.say("no such folder");
                            }
                        }
                        KeyCode::Char(c) => d.path.push(c),
                        _ => {}
                    }
                    return;
                }
                match k.code {
                    KeyCode::Esc => self.overlay = None,
                    KeyCode::Up => {
                        let s = d.selected.saturating_sub(1);
                        self.open_dialog_select(s);
                    }
                    KeyCode::Down => {
                        let s = d.selected + 1;
                        self.open_dialog_select(s);
                    }
                    KeyCode::Right => self.open_dialog_enter(),
                    KeyCode::Left | KeyCode::Backspace => {
                        if let Some(p) = d.dir.parent().map(|p| p.to_path_buf()) {
                            self.open_dialog_go(p);
                        }
                    }
                    KeyCode::Enter if d.mode == OpenMode::ImportPlaylists => self.import_dialog_enter(),
                    KeyCode::Char(' ') if d.mode == OpenMode::ImportPlaylists => self.import_toggle(None),
                    KeyCode::Enter => self.open_dialog_button(OpenButton::Open),
                    KeyCode::Tab | KeyCode::Char('/' | '~') => {
                        d.editing_path = true;
                        if let KeyCode::Char(c) = k.code {
                            d.path = c.to_string();
                        }
                    }
                    _ => {}
                }
            }
            None => {}
        }
    }

}
