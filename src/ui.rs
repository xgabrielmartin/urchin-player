use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use std::borrow::Cow;
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Divider, TickRow, menu_title_x, Anchor, App, Hit, NameFor, OpenButton, OpenMenu, OpenMode, Overlay, Target, MENUS};
use crate::audio::State;
use crate::fsx::{self, Kind};
use crate::library;
use crate::search;
use crate::settings::Visual;
use crate::slider;
use crate::theme::{Palette, Shade, THEMES};

fn st(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

fn fill(buf: &mut Buffer, r: Rect, style: Style) {
    let r = r.intersection(buf.area);
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].reset();
            buf[(x, y)].set_symbol(" ").set_style(style);
        }
    }
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max: u16) -> u16 {
    if max == 0 || y >= buf.area.height || x >= buf.area.width {
        return 0;
    }
    let max = max.min(buf.area.width - x);
    let clipped;
    let text: &str = if s.width() as u16 > max {
        let mut out = String::new();
        let mut w = 0u16;
        for ch in s.chars() {
            let cw = ch.width().unwrap_or(0) as u16;
            if w + cw + 1 > max {
                break;
            }
            out.push(ch);
            w += cw;
        }
        out.push('…');
        clipped = out;
        &clipped
    } else {
        s
    };
    let (end, _) = buf.set_stringn(x, y, text, max as usize, style);
    end - x
}

fn put_right(buf: &mut Buffer, right: u16, y: u16, s: &str, style: Style) -> u16 {
    let w = s.width() as u16;
    let x = right.saturating_sub(w);
    put(buf, x, y, s, style, w);
    x
}

fn pane_box(buf: &mut Buffer, compact: bool, r: Rect, border: Style, title: &str, title_style: Style) {
    if !compact {
        frame_box(buf, r, border, title, title_style);
    } else if !title.is_empty() && r.width > 4 && r.height > 0 {
        put(buf, r.x + 2, r.y, &format!(" {title} "), title_style, r.width - 4);
    }
}

fn frame_box(buf: &mut Buffer, r: Rect, border: Style, title: &str, title_style: Style) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width - 1, r.y + r.height - 1);
    for x in x0..=x1 {
        buf[(x, y0)].set_symbol("─").set_style(border);
        buf[(x, y1)].set_symbol("─").set_style(border);
    }
    for y in y0..=y1 {
        buf[(x0, y)].set_symbol("│").set_style(border);
        buf[(x1, y)].set_symbol("│").set_style(border);
    }
    buf[(x0, y0)].set_symbol("╭");
    buf[(x1, y0)].set_symbol("╮");
    buf[(x0, y1)].set_symbol("╰");
    buf[(x1, y1)].set_symbol("╯");
    if !title.is_empty() && r.width > 4 {
        put(buf, x0 + 2, y0, &format!(" {title} "), title_style, r.width - 4);
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let p = app.palette();
    app.hits.clear();
    let buf = f.buffer_mut();
    buf.set_style(area, st(p.fg, p.bg));
    if area.width < 30 || area.height < 10 {
        put(buf, 0, 0, "Urchin Player needs a bigger window", st(p.fg, p.bg), area.width);
        return;
    }

    let layout = Layout::sized(
        area,
        app.settings.flipped,
        app.settings.analyzer,
        app.settings.visual_rows,
        app.settings.compact,
        app.settings.visual.slot_shift(),
    );
    app.dividers.clear();
    app.panes_rect = layout.folders;
    app.analyzer_rect = layout.analyzer;
    app.screen_h = area.height;
    if !app.settings.compact {
        draw_menu_bar(buf, app, &p, area);
    }
    app.toolbar_row = layout.toolbar.y;
    if !app.toolbar_hidden() {
        draw_toolbar(buf, app, &p, area, layout.toolbar.y);
    }
    draw_columns(buf, app, &p, layout.folders);
    if layout.analyzer.height > 0 && layout.folders.height >= 2 {
        let f = layout.folders;
        let y = if app.settings.flipped {
            f.y + f.height - 1
        } else {
            f.y - visual_gap(false, app.settings.compact)
        };
        divider_edge(buf, app, &p, Divider::Visual, Rect { x: f.x, y, width: f.width, height: 1 });
    }
    if layout.analyzer.height > 0 {
        let area = layout.analyzer;
        match app.settings.visual {
            Visual::Spectrum => draw_analyzer(buf, app, &p, area),
            visual => match visual.scope_mode() {
                Some(mode) => draw_scope(buf, app, area, mode),
                None => draw_picture(buf, app, visual, area),
            },
        }
    }
    draw_transport(buf, app, &p, layout.transport);
    app.content_rect = layout.content;
    app.free_rect = layout.free;
    let content = layout.content;

    if app.overlay.is_some() {
        let backdrop = if matches!(app.overlay, Some(Overlay::Equalizer)) { content } else { area };
        app.hits.push(Hit { rect: backdrop, target: Target::Backdrop });
        match app.overlay {
            Some(Overlay::Equalizer) => draw_equalizer(buf, app, &p, area),
            Some(Overlay::Theme(_)) => draw_theme_picker(buf, app, &p, area),
            Some(Overlay::Search(_)) => draw_search(buf, app, &p, area),
            Some(Overlay::Open(_)) => draw_open_dialog(buf, app, &p, area),
            Some(Overlay::PlaylistName(_, ref what)) if what.is_eq() => {
                draw_equalizer(buf, app, &p, area);
                app.hits.push(Hit { rect: area, target: Target::Backdrop });
                draw_playlist_name(buf, app, &p, area);
            }
            Some(Overlay::PlaylistName(..)) => draw_playlist_name(buf, app, &p, area),
            Some(Overlay::Help) => draw_text_box(buf, app, &p, area, "Keyboard & Mouse", HELP),
            Some(Overlay::About) => draw_about(buf, app, &p, area),
            None => {}
        }
    }
    if app.settings.compact && (app.menu_peek || app.menu.as_ref().is_some_and(|m| m.title_index.is_some())) {
        draw_menu_bar(buf, app, &p, area);
    }
    if app.menu.is_some() {
        draw_menu(buf, app, &p, area);
    }
}

fn draw_menu_bar(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let bar = Rect { x: 0, y: 0, width: area.width, height: 1 };
    fill(buf, bar, st(p.fg, p.panel));
    if app.settings.compact {
        app.hits.push(Hit { rect: bar, target: Target::MenuBar });
    }
    let open = app.menu.as_ref().and_then(|m| m.title_index);
    for (i, name) in MENUS.iter().enumerate() {
        let x = menu_title_x(i);
        let rect = Rect { x: x - 1, y: 0, width: name.width() as u16 + 2, height: 1 };
        let hovered = app.hover == Some(Target::Menu(i)) || app.key_focus == Some(Target::Menu(i));
        let style = if open == Some(i) {
            st(p.accent_fg, p.accent)
        } else if hovered {
            st(p.fg, p.raised)
        } else {
            st(p.fg, p.panel)
        };
        fill(buf, rect, style);
        put(buf, x, 0, name, style, rect.width);
        app.hits.push(Hit { rect, target: Target::Menu(i) });
    }
    let state = match app.status.state {
        State::Playing => "▶ ",
        State::Paused => "❚❚ ",
        State::Stopped => "",
    };
    put_right(buf, area.width - 1, 0, &format!("{state}Urchin Player"), st(p.dim, p.panel));
}

const TOOLBAR_PATH_ROOM: u16 = 26;

fn draw_toolbar(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect, y: u16) {
    let base = st(p.fg, p.bg);
    let nav = |enabled: bool, focused: bool| {
        if focused {
            st(p.accent_fg, p.accent)
        } else {
            st(if enabled { p.fg } else { p.dim }, p.bg)
        }
    };
    put(buf, 1, y, "◀", nav(app.in_view() || app.can_go_up(), app.key_focus == Some(Target::Back)), 1);
    app.hits.push(Hit { rect: Rect { x: 0, y, width: 3, height: 1 }, target: Target::Back });
    put(buf, 3, y, "▶", nav(app.in_view() || app.can_go_down(), app.key_focus == Some(Target::Forward)), 1);
    app.hits.push(Hit { rect: Rect { x: 3, y, width: 2, height: 1 }, target: Target::Forward });

    let buttons: [(&str, &str, Target); 3] = [
        ("≋ EQ", "≋", Target::EqBadge),
        ("⇄ Shuffle", "⇄", Target::ToolShuffle),
        ("★ Pin Folder", "★", Target::ToolPin),
    ];
    let full: u16 = buttons.iter().map(|(label, _, _)| label.width() as u16 + 3).sum();
    let compact = area.width.saturating_sub(1) < full + TOOLBAR_PATH_ROOM;
    let mut right = area.width - 1;
    let mut button_rects = Vec::new();
    for (label, symbol, target) in buttons.iter().rev() {
        let text = format!("[{}]", if compact { symbol } else { label });
        let w = text.width() as u16;
        if right < w + 6 {
            break;
        }
        let x = right - w;
        let hovered = app.hover.as_ref() == Some(target) || app.key_focus.as_ref() == Some(target);
        let style = if hovered {
            st(p.accent_fg, p.accent)
        } else {
            st(p.fg, p.bg)
        };
        put(buf, x, y, &text, style, w);
        button_rects.push(Hit { rect: Rect { x, y, width: w, height: 1 }, target: target.clone() });
        right = x.saturating_sub(1);
    }

    if app.in_view() {
        let name = if app.queue_view.is_some() {
            "Queue"
        } else if app.library.is_some() {
            "Library"
        } else {
            "Playlists"
        };
        put(buf, 6, y, name, base.add_modifier(Modifier::BOLD), right.saturating_sub(7));
        app.hits.extend(button_rects);
        return;
    }
    let mut segs: Vec<(Cow<str>, &Path)> = vec![(fsx::contract_tilde_path(&app.root), app.root.as_path())];
    for c in app.levels.iter().take(app.focus + 1).skip(1) {
        segs.push((crate::app::short_name_cow(&c.dir), c.dir.as_path()));
    }
    let avail = right.saturating_sub(7);
    let total = |s: &[(Cow<str>, &Path)]| s.iter().map(|x| x.0.width() as u16 + 3).sum::<u16>();
    let mut start = 0;
    while start + 1 < segs.len() && total(&segs[start..]) > avail {
        start += 1;
    }
    let mut x = 6;
    if start > 0 {
        let hidden = segs[start - 1].1;
        let hovered = matches!(&app.hover, Some(Target::Crumb(h)) if h.as_path() == hidden);
        let style = if hovered { st(p.accent_fg, p.accent) } else { st(p.dim, p.bg) };
        let w = put(buf, x, y, "… ›", style, 3);
        app.hits.push(Hit { rect: Rect { x, y, width: w, height: 1 }, target: Target::Crumb(hidden.to_path_buf()) });
        x += w;
        x += put(buf, x, y, " ", st(p.dim, p.bg), 1);
    }
    for (i, (name, path)) in segs.iter().enumerate().skip(start) {
        let last = i == segs.len() - 1;
        let hovered = matches!(&app.hover, Some(Target::Crumb(h)) if h.as_path() == *path);
        let style = if hovered {
            st(p.accent_fg, p.accent)
        } else if last {
            base.add_modifier(Modifier::BOLD)
        } else {
            st(p.folder, p.bg)
        };
        let w = put(buf, x, y, name, style, right.saturating_sub(x));
        app.hits.push(Hit { rect: Rect { x, y, width: w, height: 1 }, target: Target::Crumb(path.to_path_buf()) });
        x += w;
        if !last {
            x += put(buf, x, y, " › ", st(p.dim, p.bg), 3);
        }
    }
    app.hits.extend(button_rects);
}

fn draw_columns(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    if app.queue_view.is_some() {
        draw_queue(buf, app, p, area);
        return;
    }
    if app.library.is_some() {
        draw_library(buf, app, p, area);
        return;
    }
    if app.playlists.is_some() {
        draw_playlists(buf, app, p, area);
        return;
    }
    let max_cols = if area.width < 40 || app.solo { 1 } else { 2 };
    let len = app.levels.len();
    let end = if app.solo { app.focus + 1 } else { len.min(app.focus + 2).max(app.focus + 1) };
    let start = end.saturating_sub(max_cols);
    let shown = start..end;
    let n = shown.len().max(1) as u16;
    let widths = if n == 2 { pane_widths(area.width, &[app.settings.folder_split]) } else { vec![area.width] };
    app.column_height = area.height.saturating_sub(2) as usize;
    let playing = app.status.current();
    let mut label = String::new();
    let mut x = area.x;
    let mut ticks = Vec::new();

    for (k, ci) in shown.enumerate() {
        let w = widths[k];
        x += if k > 0 { widths[k - 1] } else { 0 };
        let rect = Rect { x, y: area.y, width: w, height: area.height };
        let focused = ci == app.focus;
        let col = &app.levels[ci];
        let mut title = if ci == 0 { fsx::contract_tilde_path(&col.dir) } else { crate::app::short_name_cow(&col.dir) };
        if !col.marks.is_empty() {
            title = Cow::Owned(format!("{title}{}", ticked_note(col.marks.len())));
        }
        let border = if focused { st(p.accent, p.bg) } else { st(p.frame, p.bg) };
        let title_style = if focused { st(p.accent, p.bg).add_modifier(Modifier::BOLD) } else { st(p.dim, p.bg) };
        pane_box(buf, app.settings.compact, rect, border, &title, title_style);
        app.hits.push(Hit { rect, target: Target::Column(ci) });

        let inner = Rect { x: x + 1, y: area.y + 1, width: w.saturating_sub(2), height: area.height.saturating_sub(2) };
        if col.entries.is_empty() {
            put(buf, inner.x + 1, inner.y, "no music here", st(p.dim, p.bg), inner.width.saturating_sub(1));
            continue;
        }
        let rows = inner.height as usize;
        for (r, idx) in (col.offset..col.entries.len()).take(rows).enumerate() {
            let e = &col.entries[idx];
            let y = inner.y + r as u16;
            let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let selected = col.selected == Some(idx);
            let is_playing = e.kind == Kind::Track && playing == Some(&e.path);
            let hovered = row_hovered(app, TickRow::Folder { col: ci, row: idx });
            let bg = if hovered { p.hover_bg } else { p.bg };
            let normal = if e.kind == Kind::Folder { p.folder } else { p.fg };
            let style = row_style(p, bg, normal, selected, focused, is_playing);
            fill(buf, row_rect, style);
            let marker = if is_playing { "▶ " } else { "  " };
            let name = if e.kind == Kind::Track { fsx::title_cow(&e.path) } else { Cow::Borrowed(e.name.as_str()) };
            let tail = if e.kind == Kind::Folder { 2 } else { 0 };
            label.clear();
            label.push_str(marker);
            label.push_str(&name);
            put(buf, inner.x, y, &label, style, inner.width.saturating_sub(tail));
            if e.kind == Kind::Folder && inner.width > 2 {
                put(buf, inner.x + inner.width - 2, y, "›", style, 1);
            }
            app.hits.push(Hit { rect: row_rect, target: Target::Row { col: ci, row: idx } });
            ticks.push((inner.x, y, TickRow::Folder { col: ci, row: idx }, style));
        }
        if !app.settings.compact && col.entries.len() > rows && rows > 0 {
            let track = rows as f32;
            let pos = (col.offset as f32 / (col.entries.len() - rows) as f32 * (track - 1.0)).round() as u16;
            buf[(x + w - 1, inner.y + pos)].set_symbol("┃").set_style(border);
        }
    }
    for (x, y, t, style) in ticks {
        tick_cell(buf, app, p, x, y, t, style);
    }
    if n == 2 {
        divider_between(buf, app, p, Divider::Folders, area, area.x + widths[0]);
    }
}

fn scroll_mark(buf: &mut Buffer, x: u16, y: u16, rows: usize, total: usize, offset: usize, style: Style) {
    if total > rows && rows > 0 {
        let pos = (offset as f32 / (total - rows) as f32 * (rows as f32 - 1.0)).round() as u16;
        buf[(x, y + pos)].set_symbol("┃").set_style(style);
    }
}

fn draw_playlists(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let Some(v) = app.playlists.as_ref() else { return };
    app.column_height = area.height.saturating_sub(2) as usize;
    let panes: &[usize] = if area.width < 40 { if v.on_songs { &[1] } else { &[0] } } else { &[0, 1] };
    let n = panes.len() as u16;
    let widths = if n == 2 { pane_widths(area.width, &[app.settings.playlist_split]) } else { vec![area.width] };
    let playing = app.status.current();
    let mut ticks = Vec::new();

    for (k, &pane) in panes.iter().enumerate() {
        let x = area.x + widths[..k].iter().sum::<u16>();
        let w = widths[k];
        let rect = Rect { x, y: area.y, width: w, height: area.height };
        let focused = (pane == 1) == v.on_songs;
        let border = if focused { st(p.accent, p.bg) } else { st(p.frame, p.bg) };
        let title_style = if focused { st(p.accent, p.bg).add_modifier(Modifier::BOLD) } else { st(p.dim, p.bg) };
        let title = if pane == 0 {
            "Playlists".to_string()
        } else {
            let name = v.names.get(v.selected).map_or("", String::as_str);
            let ticked = ticked_note(v.song_marks.len());
            match (v.songs.len(), v.skipped) {
                (_, 0) => format!("{name}{ticked}"),
                (_, s) => format!("{name} · {s} can't be played{ticked}"),
            }
        };
        pane_box(buf, app.settings.compact, rect, border, &title, title_style);
        app.hits.push(Hit { rect, target: Target::PlaylistPane(pane) });

        let inner = Rect { x: x + 1, y: area.y + 1, width: w.saturating_sub(2), height: area.height.saturating_sub(2) };
        let rows = inner.height as usize;
        let (total, offset, selected) = if pane == 0 { (v.files.len(), v.offset, v.selected) } else { (v.songs.len(), v.song_offset, v.song_selected) };
        if total == 0 {
            let (a, b) = if pane == 0 {
                ("No Playlists Yet", "File › Add New Playlist")
            } else if v.files.is_empty() {
                ("", "")
            } else {
                ("no songs that can be played", "")
            };
            put(buf, inner.x + 1, inner.y, a, st(p.dim, p.bg), inner.width.saturating_sub(1));
            put(buf, inner.x + 1, inner.y + 1, b, st(p.dim, p.bg), inner.width.saturating_sub(1));
            continue;
        }
        for (r, idx) in (offset..total).take(rows).enumerate() {
            let y = inner.y + r as u16;
            let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let is_selected = idx == selected;
            let song = if pane == 1 { Some(&v.songs[idx]) } else { None };
            let is_playing = song.is_some_and(|s| playing == Some(s));
            let target = if pane == 0 { Target::PlaylistRow(idx) } else { Target::PlaylistSong(idx) };
            let hovered = app.hover.as_ref() == Some(&target) || (pane == 1 && row_hovered(app, TickRow::Playlist(idx)));
            let bg = if hovered { p.hover_bg } else { p.bg };
            let normal = if pane == 0 { p.folder } else { p.fg };
            let style = row_style(p, bg, normal, is_selected, focused, is_playing);
            fill(buf, row_rect, style);
            let marker = if is_playing { "▶ " } else { "  " };
            let name: Cow<str> = match song {
                Some(s) => fsx::title_cow(s),
                None => Cow::Borrowed(v.names[idx].as_str()),
            };
            put(buf, inner.x, y, &format!("{marker}{name}"), style, inner.width);
            app.hits.push(Hit { rect: row_rect, target });
            if pane == 1 {
                ticks.push((inner.x, y, TickRow::Playlist(idx), style));
            }
        }
        if !app.settings.compact {
            scroll_mark(buf, x + w - 1, inner.y, rows, total, offset, border);
        }
    }
    for (x, y, t, style) in ticks {
        tick_cell(buf, app, p, x, y, t, style);
    }
    if n == 2 {
        divider_between(buf, app, p, Divider::Playlists, area, area.x + widths[0]);
    }
}

fn draw_library(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let Some(v) = app.library.as_ref() else { return };
    app.column_height = area.height.saturating_sub(2) as usize;
    if v.focus == library::SONGS {
        draw_library_songs(buf, app, p, area);
        return;
    }
    let focus = v.focus;
    let shown_cols: usize = if area.width >= 60 {
        3
    } else if area.width >= 40 {
        2
    } else {
        1
    };
    let start = (focus + 1).saturating_sub(shown_cols).min(3 - shown_cols);
    let widths = if shown_cols == 3 { pane_widths(area.width, &app.settings.library_split) } else { pane_widths(area.width, &[500; 2][..shown_cols - 1]) };
    let playing_text = |v: &library::View| -> String {
        let (done, total) = v.progress.get();
        if total == 0 {
            "looking for songs…".to_string()
        } else {
            format!("{done} of {total} songs")
        }
    };

    for (k, c) in (start..start + shown_cols).enumerate() {
        let x = area.x + widths[..k].iter().sum::<u16>();
        let w = widths[k];
        let rect = Rect { x, y: area.y, width: w, height: area.height };
        let focused = c == focus;
        let border = if focused { st(p.accent, p.bg) } else { st(p.frame, p.bg) };
        let title_style = if focused { st(p.accent, p.bg).add_modifier(Modifier::BOLD) } else { st(p.dim, p.bg) };
        pane_box(buf, app.settings.compact, rect, border, ["Genre", "Artist", "Album"][c], title_style);
        app.hits.push(Hit { rect, target: Target::LibraryPane(c) });

        let Some(v) = app.library.as_ref() else { return };
        let inner = Rect { x: x + 1, y: area.y + 1, width: w.saturating_sub(2), height: area.height.saturating_sub(2) };
        let dim = st(p.dim, p.bg);
        if v.reading() {
            if c == 0 || shown_cols == 1 {
                put(buf, inner.x + 1, inner.y, "Reading your music…", dim, inner.width.saturating_sub(1));
                put(buf, inner.x + 1, inner.y + 1, &playing_text(v), dim, inner.width.saturating_sub(1));
            }
            continue;
        }
        if v.index().is_some_and(|ix| ix.tracks.is_empty()) {
            if c == 0 || shown_cols == 1 {
                put(buf, inner.x + 1, inner.y, "No songs found", dim, inner.width.saturating_sub(1));
                put(buf, inner.x + 1, inner.y + 1, &format!("in {}", fsx::contract_tilde_path(v.root())), dim, inner.width.saturating_sub(1));
            }
            continue;
        }
        let col = &v.cols[c];
        let rows = inner.height as usize;
        let show_artist = c == 2 && v.cols[1].id_is_all();
        let playing_album = app.status.current().and_then(|cur| v.locate(cur));
        for (r, idx) in (col.offset..col.rows.len()).take(rows).enumerate() {
            let row = &col.rows[idx];
            let y = inner.y + r as u16;
            let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let target = Target::LibraryRow { col: c, row: idx };
            let selected = idx == col.selected;
            let bg = if app.hover.as_ref() == Some(&target) { p.hover_bg } else { p.bg };
            let is_playing = playing_album.is_some_and(|(g, a, b)| row.id.is_some_and(|id| id == [g, a, b][c]));
            let style = row_style(p, bg, p.folder, selected, focused, is_playing);
            let dim_style = st(p.dim, bg);
            fill(buf, row_rect, style);

            let arrow = c == 2 && inner.width >= 12;
            let right_edge = inner.x + inner.width - if arrow { 3 } else { 1 };
            let side = if show_artist && row.id.is_some() { row.by.clone() } else { row.count.to_string() };
            let avail = right_edge.saturating_sub(inner.x + 2);
            let (name_full, side_full) = (row.name.width() as u16, side.width() as u16);
            let mut side_w = if name_full + 1 + side_full <= avail {
                side_full
            } else {
                side_full.min((avail / 3).max(avail.saturating_sub(name_full + 1)))
            };
            if avail.saturating_sub(side_w + 1) < 8 {
                side_w = 0;
            }
            let name_w = avail.saturating_sub(if side_w > 0 { side_w + 1 } else { 0 });
            let marker = if is_playing { "▶ " } else { "  " };
            put(buf, inner.x, y, &format!("{marker}{}", row.name), style, name_w + 2);
            if side_w > 0 {
                put(buf, right_edge - side_w, y, &side, dim_style, side_w);
            }
            if arrow {
                put(buf, inner.x + inner.width - 2, y, "▸", dim_style, 1);
            }
            app.hits.push(Hit { rect: row_rect, target });
            if arrow {
                app.hits.push(Hit { rect: Rect { x: inner.x + inner.width - 3, y, width: 3, height: 1 }, target: Target::LibraryOpen(idx) });
            }
        }
        if !app.settings.compact {
            scroll_mark(buf, x + w - 1, inner.y, rows, col.rows.len(), col.offset, border);
        }
    }
    if shown_cols == 3 {
        for i in 0..2 {
            divider_between(buf, app, p, Divider::Library(i), area, area.x + widths[..=i].iter().sum::<u16>());
        }
    }
}

fn draw_library_songs(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let Some(v) = app.library.as_ref() else { return };
    let Some(ix) = v.index() else { return };
    let ix = ix.clone();
    let title = match v.songs.len() {
        1 => format!("{} · 1 song{}", v.songs_title(), ticked_note(v.song_marks.len())),
        n => format!("{} · {n} songs{}", v.songs_title(), ticked_note(v.song_marks.len())),
    };
    pane_box(buf, app.settings.compact, area, st(p.accent, p.bg), &title, st(p.accent, p.bg).add_modifier(Modifier::BOLD));
    app.hits.push(Hit { rect: area, target: Target::LibraryPane(library::SONGS) });
    let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
    if v.songs.is_empty() {
        put(buf, inner.x + 1, inner.y, "No songs", st(p.dim, p.bg), inner.width.saturating_sub(1));
        return;
    }
    let (selected, offset) = (v.song_selected, v.song_offset);
    let artist_shown = v.cols[1].id_is_all();
    let album_shown = v.cols[2].id_is_all();
    let playing = app.status.current();
    let rows = inner.height as usize;
    let mut ticks = Vec::new();
    for (r, idx) in (offset..v.songs.len()).take(rows).enumerate() {
        let t = &ix.tracks[v.songs[idx] as usize];
        let y = inner.y + r as u16;
        let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
        let target = Target::LibraryRow { col: library::SONGS, row: idx };
        let is_selected = idx == selected;
        let is_playing = playing == Some(&t.path);
        let bg = if row_hovered(app, TickRow::Library(idx)) { p.hover_bg } else { p.bg };
        let style = row_style(p, bg, p.fg, is_selected, true, is_playing);
        let dim_style = st(p.dim, bg);
        fill(buf, row_rect, style);
        let marker = if is_playing { "▶ " } else { "  " };
        let number = if t.no > 0 { format!("{:>3}  ", t.no) } else { "     ".to_string() };
        let side = match (artist_shown, album_shown) {
            (true, _) => format!("{} › {}", ix.artists[t.artist as usize], ix.albums[t.album as usize].name),
            (false, true) => ix.albums[t.album as usize].name.clone(),
            _ => String::new(),
        };
        let lead = (marker.width() + number.width()) as u16;
        let room = inner.width.saturating_sub(lead + 2);
        let side_w = if room < 30 { 0 } else { (side.width() as u16).min(inner.width * 2 / 5).min(room.saturating_sub(t.title.width() as u16).max(inner.width / 5)) };
        let title_w = room.saturating_sub(if side_w > 0 { side_w + 2 } else { 0 });
        put(buf, inner.x, y, &format!("{marker}{number}"), dim_style, lead);
        put(buf, inner.x + lead, y, &t.title, style, title_w);
        if side_w > 0 {
            put(buf, inner.x + inner.width - 1 - side_w, y, &side, dim_style, side_w);
        }
        app.hits.push(Hit { rect: row_rect, target });
        ticks.push((inner.x, y, TickRow::Library(idx), style));
    }
    let total = v.songs.len();
    for (x, y, t, style) in ticks {
        tick_cell(buf, app, p, x, y, t, style);
    }
    if !app.settings.compact {
        scroll_mark(buf, area.x + area.width - 1, inner.y, rows, total, offset, st(p.accent, p.bg));
    }
}

fn draw_queue(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let Some(v) = app.queue_view.as_ref() else { return };
    let (selected, offset) = (v.selected, v.offset);
    app.column_height = area.height.saturating_sub(2) as usize;
    let queue = app.status.queue.clone();
    let playing = app.status.playing_index();
    let ticked = ticked_note(v.marks.len());
    let title = match queue.len() {
        0 => "Queue".to_string(),
        1 => format!("Queue · 1 song{ticked}"),
        n => format!("Queue · {n} songs{ticked}"),
    };
    pane_box(buf, app.settings.compact, area, st(p.accent, p.bg), &title, st(p.accent, p.bg).add_modifier(Modifier::BOLD));
    app.hits.push(Hit { rect: area, target: Target::QueuePane });

    let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
    if queue.is_empty() {
        let dim = st(p.dim, p.bg);
        put(buf, inner.x + 1, inner.y, "The queue is empty", dim, inner.width.saturating_sub(1));
        put(buf, inner.x + 1, inner.y + 1, "Play a folder or a playlist, or right-click a song › Add to Queue", dim, inner.width.saturating_sub(1));
        return;
    }
    let rows = inner.height as usize;
    for (r, idx) in (offset..queue.len()).take(rows).enumerate() {
        let y = inner.y + r as u16;
        let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
        let target = Target::QueueRow(idx);
        let is_playing = playing == Some(idx);
        let played = playing.is_some_and(|i| idx < i);
        let bg = if row_hovered(app, TickRow::Queue(idx)) { p.hover_bg } else { p.bg };
        let normal = if played { p.dim } else { p.fg };
        let style = row_style(p, bg, normal, idx == selected, true, is_playing);
        fill(buf, row_rect, style);
        let marker = if is_playing { "▶ " } else { "  " };
        let text = format!("{marker}{}", fsx::title_cow(&queue[idx]));
        put(buf, inner.x, y, &text, style, inner.width);
        app.hits.push(Hit { rect: row_rect, target });
        tick_cell(buf, app, p, inner.x, y, TickRow::Queue(idx), style);
    }
    if !app.settings.compact {
        scroll_mark(buf, area.x + area.width - 1, inner.y, rows, queue.len(), offset, st(p.accent, p.bg));
    }
}

pub fn pulse(position: f64) -> f32 {
    const PERIOD: f64 = 8.0;
    const MAX: f64 = 0.7;
    let phase = (position.max(0.0) % PERIOD) / PERIOD * std::f64::consts::TAU;
    ((1.0 - phase.cos()) / 2.0 * MAX) as f32
}

const MARQUEE_STEP: f64 = 0.5;
const MARQUEE_HOLD: f64 = 3.0;
const MARQUEE_GAP: usize = 6;

pub fn marquee(text: &str, width: usize, position: f64) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut cells: Vec<Option<char>> = Vec::with_capacity(text.len() + MARQUEE_GAP);
    for ch in text.chars() {
        match ch.width().unwrap_or(0) {
            0 => {}
            w => {
                cells.push(Some(ch));
                cells.extend(std::iter::repeat_n(None, w - 1));
            }
        }
    }
    cells.extend(std::iter::repeat_n(Some(' '), MARQUEE_GAP));
    let lap = cells.len();
    let cycle = MARQUEE_HOLD + lap as f64 * MARQUEE_STEP;
    let t = position.max(0.0) % cycle;
    let offset = if t < MARQUEE_HOLD { 0 } else { (((t - MARQUEE_HOLD) / MARQUEE_STEP) as usize).min(lap - 1) };
    let mut out = String::with_capacity(width * 3);
    let mut x = 0;
    while x < width {
        let i = (offset + x) % lap;
        match cells[i] {
            None => {
                out.push(' ');
                x += 1;
            }
            Some(ch) => {
                let w = ch.width().unwrap_or(1);
                if x + w > width {
                    out.push(' ');
                    x += 1;
                } else {
                    out.push(ch);
                    x += w;
                }
            }
        }
    }
    out
}

fn blend(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ => a,
    }
}

pub struct Layout {
    pub toolbar: Rect,
    pub transport: Rect,
    pub analyzer: Rect,
    pub folders: Rect,
    pub content: Rect,
    pub free: Rect,
}

fn row_style(p: &Palette, bg: Color, normal: Color, selected: bool, focused: bool, playing: bool) -> Style {
    if playing {
        st(p.playing, bg).add_modifier(Modifier::BOLD)
    } else if selected && focused {
        st(p.cursor, bg).add_modifier(Modifier::BOLD)
    } else if selected {
        st(p.cursor, bg)
    } else {
        st(normal, bg)
    }
}

fn tick_cell(buf: &mut Buffer, app: &mut App, p: &Palette, x: u16, y: u16, t: TickRow, row_style: Style) {
    let target = Target::Tick(t);
    if app.is_ticked(t) {
        buf[(x, y)].set_symbol("✓").set_style(row_style.fg(p.accent).add_modifier(Modifier::BOLD));
    } else if app.hover.as_ref() == Some(&target) {
        buf[(x, y)].set_symbol("✓").set_style(row_style.fg(p.dim));
    }
    app.hits.push(Hit { rect: Rect { x, y, width: 2, height: 1 }, target });
}

fn row_hovered(app: &App, t: TickRow) -> bool {
    app.hover.as_ref().is_some_and(|h| *h == t.row_target() || *h == Target::Tick(t))
}

fn ticked_note(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!(" · {n} ticked")
    }
}

pub const MIN_PANE_W: u16 = 12;

pub fn pane_widths(total: u16, splits: &[u16]) -> Vec<u16> {
    let n = splits.len() as u16 + 1;
    let min = MIN_PANE_W.min(total / n);
    let mut widths = Vec::with_capacity(n as usize);
    let mut prev = 0u16;
    for (i, &s) in splits.iter().enumerate() {
        let edge = ((total as u32 * s.min(1000) as u32 + 500) / 1000) as u16;
        let lo = prev + min;
        let hi = total - min * (n - 1 - i as u16);
        let edge = edge.max(lo).min(hi);
        widths.push(edge - prev);
        prev = edge;
    }
    widths.push(total - prev);
    widths
}

pub fn split_at(total: u16, edge: u16) -> u16 {
    if total == 0 {
        return 500;
    }
    ((edge.min(total) as u32 * 1000 + total as u32 / 2) / total as u32) as u16
}

pub fn move_split(total: u16, splits: &mut [u16], i: usize, edge: i32) {
    let widths = pane_widths(total, splits);
    let min = MIN_PANE_W.min(total / widths.len() as u16) as i32;
    let left: i32 = widths[..i].iter().map(|&w| w as i32).sum();
    let right: i32 = widths[..=i + 1].iter().map(|&w| w as i32).sum();
    let edge = edge.max(left + min).min(right - min).max(0) as u16;
    splits[i] = split_at(total, edge);
}

pub type Dividers = Vec<(Divider, u16)>;

fn divider_between(buf: &mut Buffer, app: &mut App, p: &Palette, d: Divider, area: Rect, edge: u16) {
    if edge == 0 || area.height == 0 {
        return;
    }
    app.dividers.push((d, edge));
    divider_edge(buf, app, p, d, Rect { x: edge - 1, y: area.y, width: 2, height: area.height });
}

fn divider_edge(buf: &mut Buffer, app: &mut App, p: &Palette, d: Divider, rect: Rect) {
    if d == Divider::Visual {
        app.dividers.push((d, rect.y));
    }
    app.hits.push(Hit { rect, target: Target::Divider(d) });
    let t = Some(Target::Divider(d));
    if (app.hover == t || app.drag == t) && app.settings.compact {
        let blank = |buf: &Buffer, x: u16, y: u16| buf[(x, y)].symbol() == " ";
        if rect.width == 2 {
            for y in rect.y..rect.y + rect.height {
                buf[(rect.x, y)].set_symbol("│");
            }
        } else {
            for y in rect.y..rect.y + rect.height {
                for x in rect.x..rect.x + rect.width {
                    let beside_title = |nx: u16| nx >= rect.x && nx < rect.x + rect.width && !blank(buf, nx, y) && !matches!(buf[(nx, y)].symbol(), "─" | "│");
                    if blank(buf, x, y) && !(x > 0 && beside_title(x - 1)) && !beside_title(x + 1) {
                        buf[(x, y)].set_symbol("─");
                    }
                }
            }
        }
    }
    if app.hover == t || app.drag == t {
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let cell = &mut buf[(x, y)];
                if matches!(cell.symbol(), "─" | "│" | "╭" | "╮" | "╰" | "╯" | "┃") {
                    cell.set_fg(p.accent);
                }
            }
        }
    }
}

fn fixed_rows(flipped: bool, compact: bool) -> u16 {
    const PATH_BAR: u16 = 1;
    let menu_bar = u16::from(!compact);
    let bottom_gap = u16::from(!flipped);
    menu_bar + PATH_BAR + TRANSPORT_H + bottom_gap + visual_gap(flipped, compact)
}

fn visual_gap(flipped: bool, compact: bool) -> u16 {
    u16::from(compact && !flipped)
}

impl Layout {
    #[cfg(test)]
    pub fn new(area: Rect, flipped: bool, show_analyzer: bool) -> Layout {
        Layout::sized(area, flipped, show_analyzer, 0, false, 0)
    }

    pub fn sized(area: Rect, flipped: bool, show_analyzer: bool, visual_rows: u16, compact: bool, shift: i16) -> Layout {
        let h = area.height;
        let rows = |y: u16, height: u16| Rect { x: area.x, y: area.y + y, width: area.width, height };
        let an = match (show_analyzer, visual_rows_range(h, flipped, compact)) {
            (false, _) => 0,
            (true, Some((lo, hi))) if visual_rows > 0 => visual_rows.clamp(lo, hi),
            (true, _) => analyzer_rows_for(h, flipped, compact),
        };
        let an = shifted_slot(h, flipped, compact, an, shift);
        let folders_h = h.saturating_sub(fixed_rows(flipped, compact) + an);
        let top = u16::from(!compact);
        if flipped {
            Layout {
                toolbar: rows(top, 1),
                folders: rows(top + 1, folders_h),
                analyzer: rows(top + 1 + folders_h, an),
                transport: rows(h - TRANSPORT_H, TRANSPORT_H),
                content: rows(top, h - top - TRANSPORT_H),
                free: rows(top, h - top - TRANSPORT_H - an),
            }
        } else {
            let content_y = top + TRANSPORT_H;
            Layout {
                transport: rows(top, TRANSPORT_H),
                analyzer: rows(content_y, an),
                folders: rows(content_y + an + visual_gap(flipped, compact), folders_h),
                toolbar: rows(h - 2, 1),
                content: rows(content_y, h - content_y),
                free: rows(content_y + an, h - content_y - an),
            }
        }
    }
}

#[cfg(test)]
pub fn analyzer_rows(screen_h: u16, flipped: bool) -> u16 {
    analyzer_rows_for(screen_h, flipped, false)
}

pub fn analyzer_rows_for(screen_h: u16, flipped: bool, compact: bool) -> u16 {
    const MIN_SEGMENTS: u16 = 4;
    const MIN_FOLDER_ROWS: u16 = 8;
    const LABELS: u16 = 1;
    let room = screen_h.saturating_sub(fixed_rows(flipped, compact) + MIN_FOLDER_ROWS + LABELS);
    let segments = (screen_h / 3).min(room);
    if segments < MIN_SEGMENTS { 0 } else { segments + LABELS }
}

const MIN_PICTURE_ROWS: u16 = 4;
const MIN_PANE_ROWS: u16 = 5;

pub fn shifted_slot(screen_h: u16, flipped: bool, compact: bool, slot: u16, shift: i16) -> u16 {
    if slot == 0 {
        return 0;
    }
    let by = shift.unsigned_abs();
    if shift < 0 {
        slot - by.min(slot.saturating_sub(MIN_PICTURE_ROWS))
    } else {
        let panes = screen_h.saturating_sub(fixed_rows(flipped, compact) + slot);
        slot + by.min(panes.saturating_sub(MIN_PANE_ROWS))
    }
}

pub fn visual_rows_range(screen_h: u16, flipped: bool, compact: bool) -> Option<(u16, u16)> {
    const MIN_VISUAL: u16 = MIN_PICTURE_ROWS + 1;
    let hi = screen_h.saturating_sub(fixed_rows(flipped, compact) + MIN_PANE_ROWS);
    (hi >= MIN_VISUAL).then_some((MIN_VISUAL, hi))
}

const FREQ_LABELS: [(f32, &str); 10] = [
    (20.0, "20 Hz"),
    (60.0, "60 Hz"),
    (125.0, "125 Hz"),
    (250.0, "250 Hz"),
    (500.0, "500 Hz"),
    (1000.0, "1 kHz"),
    (2000.0, "2 kHz"),
    (4000.0, "4 kHz"),
    (8000.0, "8 kHz"),
    (16000.0, "16 kHz"),
];
const LABEL_PRIORITY: [usize; 10] = [0, 9, 5, 3, 7, 1, 4, 6, 8, 2];

struct Label {
    index: usize,
    bar: i32,
    ideal: i32,
    width: i32,
}

fn place_labels(columns: usize, cw: u16, x0: u16, rate: u32) -> Vec<(usize, u16)> {
    let (x0, cw) = (x0 as i32, cw as i32);
    let end = x0 + columns as i32 * (cw + 1) - 1;
    let mut labels: Vec<Label> = FREQ_LABELS
        .iter()
        .enumerate()
        .filter_map(|(index, &(freq, text))| {
            let bar = x0 + crate::spectrum::column_of(freq, columns, rate)? as i32 * (cw + 1);
            let width = text.chars().count() as i32;
            let ideal = if index == 0 {
                x0
            } else if index == FREQ_LABELS.len() - 1 {
                end - width
            } else {
                (2 * bar + cw - width) / 2
            };
            Some(Label { index, bar, ideal, width })
        })
        .collect();
    loop {
        let ideals: Vec<(i32, i32)> = labels.iter().map(|l| (l.ideal, l.width)).collect();
        let starts = spread(&ideals, x0, end)
            .filter(|starts| labels.iter().zip(starts).all(|(l, &s)| s < l.bar + cw && s + l.width > l.bar));
        if let Some(starts) = starts {
            return labels.iter().zip(starts).map(|(l, s)| (l.index, s as u16)).collect();
        }
        let Some(&least) = LABEL_PRIORITY.iter().rev().find(|&&i| labels.iter().any(|l| l.index == i)) else {
            return Vec::new();
        };
        labels.retain(|l| l.index != least);
    }
}

fn spread(labels: &[(i32, i32)], lo: i32, hi: i32) -> Option<Vec<i32>> {
    let mut packed = Vec::with_capacity(labels.len());
    let mut used = -1;
    for &(_, width) in labels {
        packed.push(used + 1);
        used += width + 1;
    }
    let (min, max) = (lo, hi - used);
    if max < min {
        return None;
    }
    let mut blocks: Vec<(i32, i32)> = Vec::new();
    for (&(ideal, _), &p) in labels.iter().zip(&packed) {
        let mut block = (ideal - p, 1);
        while let Some(&(sum, n)) = blocks.last() {
            if sum * block.1 <= block.0 * n {
                break;
            }
            blocks.pop();
            block = (block.0 + sum, block.1 + n);
        }
        blocks.push(block);
    }
    let shifts = blocks.iter().flat_map(|&(sum, n)| {
        let mean = (2 * sum + n).div_euclid(2 * n);
        std::iter::repeat_n(mean.clamp(min, max), n as usize)
    });
    Some(shifts.zip(packed).map(|(shift, p)| shift + p).collect())
}

const SEGMENT: &str = "▆";

fn draw_analyzer(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let colors = p.analyzer;
    let glass = colors.glass;
    fill(buf, area, st(glass, glass));
    let segs = area.height.saturating_sub(1);
    let inner = area.width.saturating_sub(2);
    if segs == 0 || inner < 8 {
        return;
    }
    let mut cw = 2u16;
    while (inner + 1) / (cw + 1) > 48 && cw < 4 {
        cw += 1;
    }
    let n = ((inner + 1) / (cw + 1)).clamp(1, 64);
    let total = n * (cw + 1) - 1;
    let x0 = area.x + (area.width - total) / 2;
    let now = std::time::Instant::now();
    let hold = app.viz_hold();
    let analyzer = app.visuals.spectrum();
    if hold {
        analyzer.hold(now);
    }
    let lit = analyzer.update(&app.tap, n as usize, segs, now);
    for (c, &on) in lit.iter().enumerate() {
        let x = x0 + c as u16 * (cw + 1);
        for s in 0..segs {
            let y = area.y + segs - 1 - s;
            let color = match Shade::of(s, on) {
                Some(shade) => colors.lit(shade),
                None => colors.unlit,
            };
            for dx in 0..cw {
                buf[(x + dx, y)].set_symbol(SEGMENT).set_style(st(color, glass));
            }
        }
    }

    let label_y = area.y + segs;
    let key = (n as usize, cw, x0, app.tap.rate());
    if app.label_cache.as_ref().map(|c| c.0) != Some(key) {
        app.label_cache = Some((key, place_labels(key.0, cw, x0, key.3)));
    }
    if let Some((_, labels)) = &app.label_cache {
        for &(i, start) in labels {
            let text = FREQ_LABELS[i].1;
            put(buf, start, label_y, text, st(colors.label, glass), text.chars().count() as u16);
        }
    }
}

fn draw_picture(buf: &mut Buffer, app: &mut App, visual: Visual, area: Rect) {
    let (theme, seed, hold) = (app.theme, app.trace_seed(), app.viz_hold());
    let now = std::time::Instant::now();
    let picture = app.visuals.picture(visual);
    picture.use_theme(theme);
    if hold {
        picture.hold(now);
    }
    picture.update(&app.tap, area.width as usize, area.height, seed, now);
    picture.paint(buf, area);
}

fn draw_scope(buf: &mut Buffer, app: &mut App, area: Rect, mode: crate::scopes::Mode) {
    let (theme, frozen) = (app.theme, app.viz_hold());
    let scopes = app.visuals.scopes();
    scopes.use_theme(theme, || THEMES[theme].scope_colors());
    scopes.update(&app.tap, frozen, std::time::Instant::now());
    scopes.paint(buf, area, mode);
    app.hits.push(Hit { rect: area, target: Target::Scope });
}

pub const TRANSPORT_H: u16 = 5;

const EIGHTHS: [&str; 8] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

const TICK: &str = "▏";

fn draw_slider(buf: &mut Buffer, x: u16, y: u16, width: u16, volume: f32, p: &Palette) {
    for (i, cell) in slider::cells(volume, width).enumerate() {
        let (symbol, style) = match cell {
            slider::Cell::Empty => (" ", st(p.dim, p.raised)),
            slider::Cell::EmptyTick => (TICK, st(p.dim, p.raised)),
            slider::Cell::Edge(n) => (EIGHTHS[usize::from(n)], st(p.lead, p.raised)),
            slider::Cell::Lit => (" ", st(p.accent, p.accent)),
            slider::Cell::LitTick => (TICK, st(p.accent_fg, p.accent)),
            slider::Cell::Lead => (" ", st(p.lead, p.lead)),
            slider::Cell::LeadTick => (TICK, st(p.lead_fg, p.lead)),
        };
        buf[(x + i as u16, y)].set_symbol(symbol).set_style(style);
    }
}

fn draw_transport(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    fill(buf, area, st(p.fg, p.panel));
    let y0 = area.y + 1;
    let y1 = area.y + 3;
    let s = &app.status;
    let button = |hovered: bool| if hovered { st(p.accent_fg, p.accent) } else { st(p.fg, p.panel) };

    let toggle_label = "  ▶  ";
    let controls: [(String, Target, u16, u16); 3] = [
        (" |◀ ".to_string(), Target::Prev, 1, 4),
        (toggle_label.to_string(), Target::Toggle, 6, 5),
        (" ▶| ".to_string(), Target::Next, 12, 4),
    ];
    for (label, t, cx, w) in controls {
        let hovered = app.hover.as_ref() == Some(&t)
            || app.key_focus.as_ref() == Some(&t)
            || (t == Target::Toggle && s.state == State::Paused);
        put(buf, cx, y0, &label, button(hovered), w);
        app.hits.push(Hit { rect: Rect { x: cx, y: y0, width: w, height: 1 }, target: t });
    }
    let x = 17;

    let vol = app.engine.volume();
    let pct = format!(" {:>3}%", (vol * 100.0).round() as u32);
    let bar_w = slider::width_for(area.width);
    let right = area.width - 1;
    let pct_x = put_right(buf, right, y0, &pct, st(p.dim, p.panel));
    let bar_x = pct_x - bar_w;
    draw_slider(buf, bar_x, y0, bar_w, vol, p);
    put(buf, bar_x - 4, y0, "vol", st(p.dim, p.panel), 3);
    let eq_x = bar_x - 5;
    app.volume_rect = Rect { x: bar_x, y: y0, width: bar_w, height: 1 };
    app.hits.push(Hit { rect: Rect { x: bar_x - 4, y: y0, width: bar_w + 4, height: 1 }, target: Target::Volume });

    let title = if s.state == State::Stopped {
        "double-click a folder or song to play".to_string()
    } else {
        let mut t = s.meta.title.clone();
        if !s.meta.artist.is_empty() {
            t.push_str(" — ");
            t.push_str(&s.meta.artist);
        }
        if !s.meta.album.is_empty() {
            t.push_str(" · ");
            t.push_str(&s.meta.album);
        }
        t
    };
    let title_style = match s.state {
        State::Stopped => st(p.dim, p.panel),
        State::Playing => st(blend(p.fg, p.panel, pulse(s.position)), p.panel).add_modifier(Modifier::BOLD),
        State::Paused => st(p.fg, p.panel).add_modifier(Modifier::BOLD),
    };
    let gap_start = x;
    let gap_end = eq_x;
    let avail = gap_end.saturating_sub(gap_start + 2);
    let tw = (title.width() as u16).min(avail);
    let tx = gap_start + 1 + (avail - tw) / 2;
    if s.state != State::Stopped && title.width() as u16 > avail {
        put(buf, tx, y0, &marquee(&title, avail as usize, s.position), title_style, avail);
    } else {
        put(buf, tx, y0, &title, title_style, tw);
    }

    let pos = app.drag_seek.unwrap_or(s.position);
    let dur = s.duration;
    let left = format!(" {} ", fsx::fmt_time(pos));
    let lw = put(buf, 1, y1, &left, st(p.fg, p.panel), 10);
    let message = app.toast.as_ref().map(|(m, _)| m.clone()).or_else(|| s.error.clone()).unwrap_or_default();
    if !message.is_empty() {
        let msg_w = (message.width() as u16).min(area.width / 2);
        put(buf, right - msg_w, area.y, &message, st(p.warn, p.panel), msg_w);
    }
    let right_time = format!(" {} ", fsx::fmt_time(dur));
    let rt_x = right.saturating_sub(right_time.width() as u16 - 1);
    put(buf, rt_x, y1, &right_time, st(p.dim, p.panel), 10);
    let bar = Rect { x: 1 + lw, y: y1, width: rt_x.saturating_sub(1 + lw), height: 1 };
    let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 };
    let knob = bar.x + ((bar.width.saturating_sub(1)) as f64 * frac).round() as u16;
    for i in bar.x..bar.x + bar.width {
        let (sym, color) = if s.state == State::Stopped {
            ("─", p.dim)
        } else if i < knob {
            ("━", p.accent)
        } else if i == knob {
            ("┃", p.fg)
        } else {
            ("─", p.dim)
        };
        buf[(i, y1)].set_symbol(sym).set_style(st(color, p.panel));
    }
    app.seek_rect = bar;
    app.hits.push(Hit { rect: bar, target: Target::Seek });
}

pub fn menu_rect(menu: &OpenMenu, area: Rect) -> Rect {
    let label_w = menu.items.iter().map(|i| i.label.width()).max().unwrap_or(0) as u16;
    let key_w = menu.items.iter().map(|i| i.shortcut.width()).max().unwrap_or(0) as u16;
    let width = (label_w + key_w + 8).min(area.width.saturating_sub(2));
    let want = menu.items.len() as u16 + 2;
    let (left, y, height) = match menu.anchor {
        Anchor::At { x, y } => (x.saturating_sub(1), y, want.min(area.height.saturating_sub(y))),
        Anchor::Click { col, row } => {
            let below = area.height.saturating_sub(row + 1);
            let above = row.saturating_sub(area.y + 1);
            let left = col.saturating_sub(1);
            let left = if left + width > area.width { (col + 1).saturating_sub(width) } else { left };
            if want <= below || above <= below {
                (left, row + 1, want.min(below))
            } else {
                let height = want.min(above);
                (left, row - height, height)
            }
        }
    };
    Rect { x: left.min(area.width.saturating_sub(width)), y, width, height }
}

fn draw_menu(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let Some(menu) = app.menu.as_ref() else { return };
    let key_w = menu.items.iter().map(|i| i.shortcut.width()).max().unwrap_or(0) as u16;
    let rect = menu_rect(menu, area);
    fill(buf, rect, st(p.fg, p.panel));
    frame_box(buf, rect, st(p.raised, p.panel), "", st(p.fg, p.panel));
    app.hits.push(Hit { rect, target: Target::Overlay });
    let items = menu.items.clone();
    let hover = menu.hover;
    let rows = rect.height.saturating_sub(2) as usize;
    let mut offset = menu.offset.min(items.len().saturating_sub(rows));
    if let Some(h) = hover {
        if h < offset {
            offset = h;
        } else if rows > 0 && h >= offset + rows {
            offset = h + 1 - rows;
        }
    }
    if let Some(m) = app.menu.as_mut() {
        m.offset = offset;
    }
    if offset > 0 {
        put(buf, rect.x + rect.width - 3, rect.y, "▲", st(p.accent, p.panel), 1);
    }
    if offset + rows < items.len() {
        put(buf, rect.x + rect.width - 3, rect.y + rect.height - 1, "▼", st(p.accent, p.panel), 1);
    }
    for (i, item) in items.iter().enumerate().skip(offset) {
        let y = rect.y + 1 + (i - offset) as u16;
        if y >= rect.y + rect.height - 1 {
            break;
        }
        let row = Rect { x: rect.x + 1, y, width: rect.width - 2, height: 1 };
        if item.is_separator() {
            for cx in row.x..row.x + row.width {
                buf[(cx, y)].set_symbol("─").set_style(st(p.raised, p.panel));
            }
            continue;
        }
        if item.heading {
            let st_h = st(p.dim, p.panel);
            fill(buf, row, st_h);
            let tw = item.label.width() as u16;
            put(buf, row.x + row.width.saturating_sub(tw) / 2, y, &item.label, st_h, row.width);
            continue;
        }
        let enabled = item.action.is_some();
        let style = if hover == Some(i) && enabled {
            st(p.accent_fg, p.accent)
        } else if enabled {
            st(p.fg, p.panel)
        } else {
            st(p.dim, p.panel)
        };
        fill(buf, row, style);
        let check = match item.checked {
            Some(true) => "✓ ",
            _ => "  ",
        };
        put(buf, row.x, y, &format!("{check}{}", item.label), style, row.width.saturating_sub(key_w + 2));
        if !item.shortcut.is_empty() {
            put_right(buf, row.x + row.width - 1, y, item.shortcut, if hover == Some(i) { style } else { st(p.dim, p.panel) });
        }
        if enabled {
            app.hits.push(Hit { rect: row, target: Target::MenuItem(i) });
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn dialog(buf: &mut Buffer, app: &mut App, p: &Palette, rect: Rect, title: &str) {
    fill(buf, rect, st(p.fg, p.panel));
    frame_box(buf, rect, st(p.accent, p.panel), title, st(p.accent, p.panel).add_modifier(Modifier::BOLD));
    app.hits.push(Hit { rect, target: Target::Overlay });
}

fn button(buf: &mut Buffer, app: &mut App, p: &Palette, (x, y): (u16, u16), label: &str, target: Target, primary: bool) -> u16 {
    let focused = app.key_focus.as_ref() == Some(&target);
    let text = if focused { format!("▸ {label} ◂") } else { format!("[ {label} ]") };
    let w = text.width() as u16;
    let hovered = app.hover.as_ref() == Some(&target) || focused;
    let mut style = if hovered || primary { st(p.accent_fg, p.accent) } else { st(p.fg, p.raised) };
    if focused {
        style = style.add_modifier(Modifier::BOLD);
    }
    put(buf, x, y, &text, style, w);
    app.hits.push(Hit { rect: Rect { x, y, width: w, height: 1 }, target });
    w
}

fn draw_theme_picker(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let rect = centered(area, 64, area.height.saturating_sub(4));
    dialog(buf, app, p, rect, "Settings › Theme");
    let Some(Overlay::Theme(tp)) = &app.overlay else { return };
    let (filter, list, selected, offset) = (tp.filter.clone(), tp.list.clone(), tp.selected, tp.offset);

    let ix = rect.x + 2;
    let iw = rect.width - 4;
    put(buf, ix, rect.y + 1, "Search ", st(p.dim, p.panel), 7);
    put(buf, ix + 7, rect.y + 1, &format!("{filter}▌"), st(p.fg, p.raised), iw.saturating_sub(24));
    put_right(buf, ix + iw, rect.y + 1, &format!("{} of {} themes", list.len(), THEMES.len()), st(p.dim, p.panel));

    let list_rect = Rect { x: ix, y: rect.y + 3, width: iw, height: rect.height.saturating_sub(7) };
    app.list_height = list_rect.height as usize;
    app.hits.push(Hit { rect: list_rect, target: Target::ThemeList });
    for r in 0..list_rect.height as usize {
        let i = offset + r;
        let Some(&ti) = list.get(i) else { break };
        let y = list_rect.y + r as u16;
        let row = Rect { x: list_rect.x, y, width: list_rect.width, height: 1 };
        let hovered = app.hover == Some(Target::ThemeRow(i));
        let style = if i == selected {
            st(p.sel_fg, p.sel_bg)
        } else if hovered {
            st(p.fg, p.hover_bg)
        } else {
            st(p.fg, p.panel)
        };
        fill(buf, row, style);
        put(buf, row.x + 1, y, THEMES[ti].name, style, row.width.saturating_sub(36));
        let t = &THEMES[ti];
        let sw_x = row.x + row.width.saturating_sub(34);
        let swatches = if row.width >= 34 + 4 { 17 } else { 0 };
        for (k, c) in std::iter::once(t.bg).chain(t.ansi.iter().copied().take(16)).take(swatches).enumerate() {
            let color = Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
            let cell_x = sw_x + k as u16 * 2;
            if cell_x + 1 < row.x + row.width {
                buf[(cell_x, y)].set_symbol("█").set_style(Style::default().fg(color));
                buf[(cell_x + 1, y)].set_symbol("█").set_style(Style::default().fg(color));
            }
        }
        app.hits.push(Hit { rect: row, target: Target::ThemeRow(i) });
    }
    if list.is_empty() {
        put(buf, list_rect.x + 1, list_rect.y, "no themes match", st(p.dim, p.panel), list_rect.width);
    }
    let by = rect.y + rect.height - 2;
    put(buf, ix, by, "type to search · ↑↓ preview · Enter keeps · Esc cancels", st(p.dim, p.panel), iw.saturating_sub(24));
    let cancel_w = "[ Cancel ]".width() as u16;
    let keep_w = "[ Keep ]".width() as u16;
    button(buf, app, p, (ix + iw - cancel_w, by), "Cancel", Target::ThemeCancel, false);
    button(buf, app, p, (ix + iw - cancel_w - keep_w - 2, by), "Keep", Target::ThemeKeep, true);
}

fn tail(s: &str, cells: usize) -> &str {
    let mut used = 0;
    for (i, ch) in s.char_indices().rev() {
        used += ch.width().unwrap_or(0);
        if used > cells {
            return &s[i + ch.len_utf8()..];
        }
    }
    s
}

fn draw_search(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let rect = centered(area, 100, area.height.saturating_sub(4));
    let by_tags = matches!(&app.overlay, Some(Overlay::Search(s)) if s.by_tags());
    dialog(buf, app, p, rect, if by_tags { "Playback › Library › Search" } else { "Playback › Search" });
    let ix = rect.x + 2;
    let iw = rect.width - 4;
    let list_rect = Rect { x: ix, y: rect.y + 3, width: iw, height: rect.height.saturating_sub(8) };
    let h = list_rect.height as usize;
    app.list_height = h;

    if let Some(Overlay::Search(s)) = app.overlay.as_mut() {
        s.fit(h);
    }
    let Some(Overlay::Search(s)) = &app.overlay else { return };
    let (query, count, total, indexing, blank) = (s.query.clone(), s.count(), s.total(), s.indexing(), s.is_blank());
    let (selected, offset) = (s.selected, s.offset);
    let root_name = fsx::contract_tilde_path(s.root()).to_string();
    let rows: Vec<(usize, String, Vec<String>)> =
        (offset..offset + h).map_while(|i| s.result(i).map(|song| (i, song.title().into_owned(), s.trail(song)))).collect();
    let location = s.highlighted().filter(|_| (offset..offset + h).contains(&selected)).map(|song| s.trail(song));

    let songs = if total == 1 { "song" } else { "songs" };
    let ticked = match &app.overlay {
        Some(Overlay::Search(s)) if !s.marks.is_empty() => format!("{} ticked · ", s.marks.len()),
        _ => String::new(),
    };
    let counter = if indexing {
        "reading folders…".to_string()
    } else if blank {
        format!("{ticked}{total} {songs}")
    } else {
        format!("{ticked}{count} of {total} {songs}")
    };
    put(buf, ix, rect.y + 1, "Search ", st(p.dim, p.panel), 7);
    let field = Rect { x: ix + 7, y: rect.y + 1, width: iw.saturating_sub(7 + counter.width() as u16 + 2).max(8), height: 1 };
    fill(buf, field, st(p.fg, p.raised));
    let typed = format!("{}▌", tail(&query, field.width.saturating_sub(3) as usize));
    put(buf, field.x + 1, field.y, &typed, st(p.fg, p.raised), field.width.saturating_sub(1));
    put_right(buf, ix + iw, rect.y + 1, &counter, st(p.dim, p.panel));

    app.hits.push(Hit { rect: list_rect, target: Target::SearchList });
    for (i, title, folders) in &rows {
        let i = *i;
        let y = list_rect.y + (i - offset) as u16;
        let row = Rect { x: list_rect.x, y, width: list_rect.width, height: 1 };
        let (title_style, trail_style) = if i == selected {
            (st(p.sel_fg, p.sel_bg), st(p.sel_fg, p.sel_bg))
        } else if row_hovered(app, TickRow::Search(i)) {
            (st(p.fg, p.hover_bg), st(p.dim, p.hover_bg))
        } else {
            (st(p.fg, p.panel), st(p.dim, p.panel))
        };
        fill(buf, row, title_style);
        let whole = search::trail_text(folders, usize::MAX);
        let (title_cells, trail_cells) = search::row_cells(title.width(), whole.width(), row.width.saturating_sub(3) as usize);
        put(buf, row.x + 2, y, title, title_style, title_cells as u16);
        if trail_cells > 0 {
            let trail = search::trail_text(folders, trail_cells);
            let w = trail.width().min(trail_cells) as u16;
            put(buf, row.x + row.width - 1 - w, y, &trail, trail_style, w);
        }
        app.hits.push(Hit { rect: row, target: Target::SearchRow(i) });
        tick_cell(buf, app, p, row.x, y, TickRow::Search(i), title_style);
    }
    scroll_mark(buf, rect.x + rect.width - 1, list_rect.y, h, count, offset, st(p.accent, p.panel));

    let dim = st(p.dim, p.panel);
    if rows.is_empty() {
        let (first, second) = if indexing {
            ("Reading the folders…".to_string(), String::new())
        } else if blank && by_tags {
            ("Type any part of a title, artist, album or genre".to_string(), format!("Searching the tags of {total} {songs}"))
        } else if blank {
            ("Type any part of a song, album or artist".to_string(), format!("Searching {total} {songs} in {root_name}"))
        } else if by_tags {
            ("No songs match".to_string(), "Every word has to be in the song's title, artist, album or genre".to_string())
        } else {
            ("No songs match".to_string(), "Every word has to be in the song's name or the folders it is in".to_string())
        };
        put(buf, list_rect.x + 1, list_rect.y, &first, dim, list_rect.width.saturating_sub(1));
        put(buf, list_rect.x + 1, list_rect.y + 1, &second, dim, list_rect.width.saturating_sub(1));
    }

    if let Some(folders) = location {
        let y = rect.y + rect.height - 4;
        put(buf, ix, y, "in ", dim, 3);
        put(buf, ix + 3, y, &search::trail_text(&folders, iw.saturating_sub(3) as usize), st(p.fg, p.panel), iw.saturating_sub(3));
    }

    let by = rect.y + rect.height - 2;
    let note = match &app.toast {
        Some((message, _)) => (message.clone(), st(p.accent, p.panel)),
        None => ("↑↓ choose · Enter plays · → more · Esc closes".to_string(), dim),
    };
    put(buf, ix, by, &note.0, note.1, iw.saturating_sub(22));
    let close_w = "[ Close ]".width() as u16;
    let play_w = "[ Play ]".width() as u16;
    button(buf, app, p, (ix + iw - close_w, by), "Close", Target::SearchClose, false);
    button(buf, app, p, (ix + iw - close_w - play_w - 2, by), "Play", Target::SearchPlay, true);
}

fn draw_open_dialog(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let rect = centered(area, 92, area.height.saturating_sub(4).min(28));
    let Some(Overlay::Open(d)) = &app.overlay else { return };
    let mode = d.mode;
    let title = match mode {
        OpenMode::MusicFolder => "Settings › Music Folder",
        OpenMode::ImportPlaylists => "File › Add New Playlist",
        OpenMode::Open => "File › Open Folder",
    };
    let (path, editing, dir, entries, selected, offset, places, checked) =
        (d.path.clone(), d.editing_path, d.dir.clone(), d.entries.clone(), d.selected, d.offset, d.places.clone(), d.checked.clone());
    dialog(buf, app, p, rect, title);

    let ix = rect.x + 2;
    let iw = rect.width - 4;
    put(buf, ix, rect.y + 1, "Path ", st(p.dim, p.panel), 5);
    let field = Rect { x: ix + 5, y: rect.y + 1, width: iw - 5, height: 1 };
    let fstyle = if editing { st(p.fg, p.raised).add_modifier(Modifier::BOLD) } else { st(p.fg, p.raised) };
    fill(buf, field, fstyle);
    let shown = if editing { format!("{path}▌") } else { path };
    put(buf, field.x + 1, field.y, &shown, fstyle, field.width.saturating_sub(16));
    put_right(buf, field.x + field.width - 1, field.y, if editing { "Tab completes" } else { "click to type" }, st(p.dim, p.raised));
    app.hits.push(Hit { rect: field, target: Target::OpenPath });

    let body_y = rect.y + 3;
    let body_h = rect.height.saturating_sub(6);
    let places_w = 24u16.min(iw / 3);
    let places_rect = Rect { x: ix, y: body_y, width: places_w, height: body_h };
    frame_box(buf, places_rect, st(p.raised, p.panel), "Places", st(p.dim, p.panel));
    for (i, (label, path)) in places.iter().enumerate() {
        let y = places_rect.y + 1 + i as u16;
        if y >= places_rect.y + places_rect.height - 1 {
            break;
        }
        let row = Rect { x: places_rect.x + 1, y, width: places_rect.width - 2, height: 1 };
        let available = path.is_dir();
        let hovered = app.hover == Some(Target::OpenPlace(i));
        let style = if hovered {
            st(p.accent_fg, p.accent)
        } else if *path == dir {
            st(p.sel_fg, p.sel_bg)
        } else if available {
            st(p.fg, p.panel)
        } else {
            st(p.dim, p.panel)
        };
        fill(buf, row, style);
        put(buf, row.x + 1, y, label, style, row.width - 1);
        app.hits.push(Hit { rect: row, target: Target::OpenPlace(i) });
    }

    let list_box = Rect { x: ix + places_w + 1, y: body_y, width: iw - places_w - 1, height: body_h };
    frame_box(buf, list_box, st(p.raised, p.panel), &fsx::contract_tilde_path(&dir), st(p.dim, p.panel));
    let list_rect = Rect { x: list_box.x + 1, y: list_box.y + 1, width: list_box.width - 2, height: list_box.height - 2 };
    app.list_height = list_rect.height as usize;
    app.hits.push(Hit { rect: list_rect, target: Target::OpenList });
    for r in 0..list_rect.height as usize {
        let i = offset + r;
        if i > entries.len() {
            break;
        }
        let y = list_rect.y + r as u16;
        let row = Rect { x: list_rect.x, y, width: list_rect.width, height: 1 };
        let hovered = app.hover == Some(Target::OpenRow(i));
        let style = if i == selected {
            st(p.sel_fg, p.sel_bg)
        } else if hovered {
            st(p.fg, p.hover_bg)
        } else {
            st(p.folder, p.panel)
        };
        fill(buf, row, style);
        let file = i > 0 && entries[i - 1].kind == Kind::Track;
        let label = if i == 0 {
            "..".to_string()
        } else if file {
            format!("{} {}", if checked.contains(&entries[i - 1].path) { "✓" } else { " " }, entries[i - 1].name)
        } else {
            entries[i - 1].name.clone()
        };
        let style = if file && i != selected && !hovered { st(p.fg, p.panel) } else { style };
        put(buf, row.x + 1, y, &label, style, row.width.saturating_sub(3));
        if i > 0 && !file {
            put(buf, row.x + row.width - 2, y, "›", style, 1);
        }
        app.hits.push(Hit { rect: row, target: Target::OpenRow(i) });
    }

    let by = rect.y + rect.height - 2;
    let mut x = ix;
    if mode == OpenMode::MusicFolder {
        x += button(buf, app, p, (x, by), "Use This Folder", Target::OpenButton(OpenButton::Open), true) + 2;
    } else if mode == OpenMode::ImportPlaylists {
        x += button(buf, app, p, (x, by), "Import", Target::OpenButton(OpenButton::Import), true) + 2;
    } else {
        x += button(buf, app, p, (x, by), "Open", Target::OpenButton(OpenButton::Open), true) + 2;
        x += button(buf, app, p, (x, by), "Play", Target::OpenButton(OpenButton::Play), false) + 2;
        x += button(buf, app, p, (x, by), "Shuffle", Target::OpenButton(OpenButton::Shuffle), false) + 2;
    }
    let cancel_w = button(buf, app, p, (x, by), "Cancel", Target::OpenButton(OpenButton::Cancel), false);
    if mode == OpenMode::ImportPlaylists {
        let hint = "click or Space checks · Enter imports";
        let free = (ix + iw).saturating_sub(x + cancel_w + 2);
        if free as usize > hint.width() {
            put_right(buf, ix + iw, by, hint, st(p.dim, p.panel));
        }
    }
}

const HELP: &str = "\
MOUSE
  click            select a folder or song (folders open to the right)
  double-click     play a folder (everything inside) or play from a song
  right-click      menu: shuffle, play next, add to queue, open, pin, add to playlist
  middle-click     play
  wheel            scroll the pane under the pointer
  wheel on volume  ±1%      wheel on seek bar  ±5 s
  drag             the seek bar to scrub, the volume bar to set volume
  tick column      a row's first two cells: a click ticks the row, a drag
                   down them ticks every row passed (Ctrl/Shift-click too,
                   where the terminal passes them on)
  drag a divider   the lines where two panes meet (or the edge by the
                   visualizer) resize them; double-click one to reset it
  breadcrumb       click any part of the path to jump there
  ◀ ▶              up out of this folder / back down into the highlighted one
  Backspace        back through the folders you've visited

KEYBOARD
  ↑ ↓ / j k        move              → / l   open folder
  (no arrow keys?  h j k l, or Ctrl+B Ctrl+N Ctrl+P Ctrl+F)
  ← / h            up a level        Enter   play
  Space / p        play / pause      n / b   next / previous
  p p (twice)      restart song      a       add song to a playlist
  , / .            back / forward 10 s       + / -   volume
  Ctrl+O           open folder       Ctrl+T  theme
  /                search your music for a song (Playback › Search)
  Ctrl+E           equalizer: shapes the playing song, remembered for it
                   in it: Tab moves bars ⇄ buttons, Enter presses, [ ] presets,
                   s saves the sound as your own preset (in the Preset list:
                   right-click or m on yours for Rename and Delete, F2, Del)
  F10 / Alt+letter menus   F1  this help   q / Ctrl+Q  quit
  Shift+← → / < >  move the divider beside the highlighted pane
  x / X / Esc      tick the row and move down / tick from the last tick to
                   here / untick them all (Ctrl+X in the Search box). While
                   rows are ticked, Enter, Shuffle, Shift+Q, Shift+N, a, Delete
                   and Shift+↑ ↓ act on all of them
  Alt+↑ ↓ / Alt+K J  move the visualizer's edge up / down
  Oscilloscope, Vectorscope, Spectroscope (scope-tui): click the scope, or
                   Tab to it and Enter, to give it the keyboard; then
                   scope-tui's keys: ↑ ↓ zoom, ← → samples, s scatter,
                   r reference lines, Tab next scope, Esc reset;
                   Oscilloscope u tune to the note playing (it stands
                   still), t trigger, e edge, p peaks, - = trigger depth,
                   PgUp PgDn threshold; Vectorscope g goniometer;
                   Spectroscope w window, l log, p phase difference,
                   PgUp PgDn average. All three are the trace alone: no
                   text on them. q (or a click elsewhere) gives the
                   keyboard back. Wheel on the scope zooms
  Add New Playlist click or Space checks a playlist, Enter imports them
  New Playlist     type a name, Enter makes it (File menu)
  m / Menu key     the right-click menu of the highlighted row
  Playlists view   F2 renames, Delete deletes the highlighted playlist
                   (right-click or m: Shuffle, Rename, Delete)
  Playlists view   Enter plays, ← → switch panes, Esc goes back
  i                the Library view: Genre, Artist, Album by their tags.
                   ← → move between columns, Enter plays what is highlighted,
                   → or the ▸ on an album shows its songs, Esc goes back,
                   / searches titles, artists, albums and genres
  Shift+Q / Shift+N  add the song (or folder) to the queue / play it next
  Shift+A          next album: skips the rest of the album playing
  Shuffle Albums   (Playback menu) every Shuffle mixes whole albums, each in
                   order; albums by their tags, or by folder when untagged
  Continue Into    (Playback menu) when the folder you played ends, the next
  Next Folder      folder follows on, then the next one up, and so on
  right-click / m  on a song or folder: Show in Finder (Explorer, File
                   Manager) and Copy Path
  Shift+↑ ↓        move a song up or down in its folder (or J K), or drag it
                   with the mouse; Urchin remembers the order. Right-click:
                   Move Up, Move Down, Reset Order (back to name order)
  Shift+W          the Queue view: Enter plays, Delete takes a song out,
                   Shift+↑ ↓ (or J K) or a drag with the mouse moves it,
                   a adds it to a playlist, Esc goes back
                   z shuffles the songs still to come, Shift+C clears the
                   queue (the song playing stays), Shift+S saves it as a playlist";

const ABOUT_VERSION: &str = concat!("Urchin Player ", env!("CARGO_PKG_VERSION"));
const ABOUT_COPYRIGHT: &str = "Copyright ";
const ABOUT_LINK: &str = "github.com/xgabrielmartin";
pub const ABOUT_URL: &str = "https://github.com/xgabrielmartin";

fn draw_about(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let link_line = ABOUT_COPYRIGHT.width() + ABOUT_LINK.width();
    let w = ABOUT_VERSION.width().max(link_line) as u16 + 6;
    let rect = centered(area, w, 6);
    dialog(buf, app, p, rect, "About Urchin Player");
    let (x, max) = (rect.x + 3, rect.width.saturating_sub(6));
    put(buf, x, rect.y + 2, ABOUT_VERSION, st(p.fg, p.panel), max);
    let y = rect.y + 3;
    let lx = x + put(buf, x, y, ABOUT_COPYRIGHT, st(p.fg, p.panel), max);
    let mut style = st(p.accent, p.panel).add_modifier(Modifier::UNDERLINED);
    if app.hover.as_ref() == Some(&Target::AboutLink) {
        style = style.add_modifier(Modifier::BOLD);
    }
    let lw = put(buf, lx, y, ABOUT_LINK, style, max.saturating_sub(lx - x));
    app.hits.push(Hit { rect: Rect { x: lx, y, width: lw, height: 1 }, target: Target::AboutLink });
}

fn draw_playlist_name(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    let rect = centered(area, 56, 7);
    let Some(Overlay::PlaylistName(name, what)) = &app.overlay else { return };
    let (name, what) = (name.clone(), what.clone());
    let (title, ok, hint) = match what {
        NameFor::New => ("File › New Playlist", "Create", "Enter creates · Esc cancels"),
        NameFor::Rename(_) => ("Rename Playlist", "Rename", "Enter renames · Esc cancels"),
        NameFor::Queue => ("Save Queue as Playlist", "Save", "Enter saves · Esc cancels"),
        NameFor::EqPreset => ("Save as Preset", "Save", "Enter saves · Esc cancels"),
        NameFor::EqRename(_) => ("Rename Preset", "Rename", "Enter renames · Esc cancels"),
    };
    dialog(buf, app, p, rect, title);
    let ix = rect.x + 2;
    let iw = rect.width - 4;
    put(buf, ix, rect.y + 2, "Name ", st(p.dim, p.panel), 5);
    let field = Rect { x: ix + 5, y: rect.y + 2, width: iw.saturating_sub(5), height: 1 };
    fill(buf, field, st(p.fg, p.raised));
    let room = field.width.saturating_sub(2) as usize;
    let shown: String = {
        let chars: Vec<char> = name.chars().collect();
        chars[chars.len().saturating_sub(room.saturating_sub(1))..].iter().collect()
    };
    put(buf, field.x + 1, field.y, &format!("{shown}▌"), st(p.fg, p.raised), field.width.saturating_sub(1));
    let by = rect.y + rect.height - 2;
    let cancel_w = "[ Cancel ]".width() as u16;
    let make_w = format!("[ {ok} ]").width() as u16;
    button(buf, app, p, (ix + iw - cancel_w, by), "Cancel", Target::PlaylistNameCancel, false);
    button(buf, app, p, (ix + iw - cancel_w - make_w - 2, by), ok, Target::PlaylistNameOk, true);
    put(buf, ix, by, hint, st(p.dim, p.panel), iw.saturating_sub(24));
}

fn draw_text_box(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect, title: &str, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    let w = lines.iter().map(|l| l.width()).max().unwrap_or(20) as u16 + 6;
    let h = lines.len() as u16 + 4;
    let rect = centered(area, w, h);
    dialog(buf, app, p, rect, title);
    for (i, l) in lines.iter().enumerate() {
        let y = rect.y + 2 + i as u16;
        if y >= rect.y + rect.height - 1 {
            break;
        }
        let style = if l.chars().all(|c| c.is_uppercase() || c == ' ') && !l.trim().is_empty() {
            st(p.header, p.panel).add_modifier(Modifier::BOLD)
        } else {
            st(p.fg, p.panel)
        };
        put(buf, rect.x + 3, y, l, style, rect.width - 6);
    }
}

fn draw_equalizer(buf: &mut Buffer, app: &mut App, p: &Palette, area: Rect) {
    use crate::eq::{LABELS, MAX_DB};
    let col_w = 6u16;
    let scale_w = 6u16;
    let w = scale_w + col_w * 11 + 6;
    let free = app.free_rect;
    let (region, rows) = if free.height >= 12 + 8 {
        (free, 13u16.min(free.height - 12))
    } else {
        (app.content_rect, 13u16.min(area.height.saturating_sub(16)).max(5))
    };
    let h = rows + 10;
    let rect = centered(region, w, h);
    dialog(buf, app, p, rect, "Settings › Equalizer");

    let ix = rect.x + 2;
    let iw = rect.width - 4;
    let song = app.eq_song();
    let curve = app.eq_curve();
    let source = app.eq_source();
    let enabled = app.settings.eq_enabled;

    let (line1, line2) = match &song {
        None => ("Nothing playing".to_string(), "Start a song, then shape its sound here.".to_string()),
        Some(path) => {
            let mut t = app.status.meta.title.clone();
            if t.is_empty() {
                t = fsx::title_from_path(path);
            }
            if !app.status.meta.artist.is_empty() {
                t = format!("{t} — {}", app.status.meta.artist);
            }
            let note = if enabled {
                match source {
                    crate::eq::Source::Own => "Custom EQ settings are automatically saved & remembered for each song.".to_string(),
                    crate::eq::Source::Folder(id) => format!(
                        "This folder's preset: {}. Changes you make apply to this song only.",
                        app.eq_store.lock().unwrap().preset_name(id)
                    ),
                    crate::eq::Source::None => "Flat. A preset you pick also applies to the other songs in this folder.".to_string(),
                }
            } else {
                "The equalizer is off (Settings › Equalizer On).".to_string()
            };
            (t, note)
        }
    };
    put(buf, ix, rect.y + 1, &line1, st(p.fg, p.panel).add_modifier(Modifier::BOLD), iw);
    put(buf, ix, rect.y + 2, &line2, st(if enabled { p.dim } else { p.warn }, p.panel), iw);

    let top = rect.y + 4;
    let row_of = |db: f32| -> u16 { (((MAX_DB - db) / (2.0 * MAX_DB)) * (rows - 1) as f32).round() as u16 };
    let zero = row_of(0.0);
    for (db, label) in [(MAX_DB, "+20"), (10.0, "+10"), (0.0, "0"), (-10.0, "-10"), (-MAX_DB, "-20")] {
        put_right(buf, ix + scale_w - 1, top + row_of(db), label, st(p.dim, p.panel));
    }
    let cols_x = ix + scale_w;
    app.eq_rect = Rect { x: cols_x, y: top, width: col_w * 11, height: rows };
    let active = song.is_some() && enabled;

    for band in 0..11usize {
        let x = cols_x + col_w * band as u16;
        let selected = app.eq_band == band && app.key_focus.is_none();
        let hovered = app.hover == Some(Target::EqBand(band));
        let db = curve.get(band);
        let vrow = row_of(db);
        let bar_color = if !active { p.dim } else if selected { p.accent } else { p.folder };
        let col_rect = Rect { x, y: top, width: col_w, height: rows + 2 };
        if selected || hovered {
            fill(buf, Rect { x, y: top, width: col_w - 1, height: rows }, st(p.fg, p.hover_bg));
        }
        for r in 0..rows {
            let y = top + r;
            let bg = if selected || hovered { p.hover_bg } else { p.panel };
            let in_bar = (r >= vrow.min(zero)) && (r <= vrow.max(zero)) && vrow != zero;
            let (sym, fg) = if r == vrow {
                ("▆▆", if active { p.fg } else { p.dim })
            } else if in_bar {
                ("██", bar_color)
            } else if r == zero {
                ("──", p.raised)
            } else {
                ("╎ ", p.raised)
            };
            put(buf, x + 1, y, sym, st(fg, bg), 2);
        }
        let value = if db.abs() < 0.05 {
            "0".to_string()
        } else if db.fract() == 0.0 {
            format!("{db:+.0}")
        } else {
            format!("{db:+.1}")
        };
        let vstyle = if selected { st(p.accent, p.panel).add_modifier(Modifier::BOLD) } else { st(p.dim, p.panel) };
        put(buf, x, top + rows, &value, vstyle, col_w - 1);
        let label = if band == 0 { "pre" } else { LABELS[band - 1] };
        let lstyle = if selected { st(p.accent, p.panel).add_modifier(Modifier::BOLD) } else { st(p.fg, p.panel) };
        put(buf, x, top + rows + 1, label, lstyle, col_w - 1);
        app.hits.push(Hit { rect: col_rect, target: Target::EqBand(band) });
    }

    let hint_y = rect.y + rect.height - 3;
    put(
        buf,
        ix,
        hint_y,
        "Tab: buttons · h/l j/k, arrows · [ ] presets · s: save preset · n/b songs",
        st(p.dim, p.panel),
        iw,
    );
    let by = rect.y + rect.height - 2;
    let mut bx = ix;
    let preset = format!("Preset: {} ▾", app.eq_store.lock().unwrap().label(&curve));
    app.eq_preset_x = bx;
    bx += button(buf, app, p, (bx, by), &preset, Target::EqPreset, false) + 2;
    bx += button(buf, app, p, (bx, by), "Reset", Target::EqReset, false) + 2;
    bx += button(buf, app, p, (bx, by), if enabled { "EQ: On" } else { "EQ: Off" }, Target::EqToggle, false) + 2;
    let close_w = "[ Close ]".width() as u16;
    let _ = bx;
    button(buf, app, p, (ix + iw - close_w, by), "Close", Target::EqClose, true);
}
