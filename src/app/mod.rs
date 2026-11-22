mod equalizer;
mod focus;
mod browser;
mod session;
mod views;
mod menus;
mod dialogs;
mod keys;
mod ticks;
mod dividers;
mod mouse;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::albums;
use crate::audio::{Engine, Repeat, ReplayGain, State, Status};
use crate::eq::{self, Curve, Store};
use crate::fsx::{self, Entry, Kind};
use crate::library;
use crate::order::Order;
use crate::playlists;
use crate::queue;
use crate::search::Search;
use crate::visuals::Visuals;
use crate::settings::{Settings, Visual};
use crate::slider;
use crate::theme::{self, Palette, THEMES};

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Menu(usize),
    MenuBar,
    Scope,
    MenuItem(usize),
    Back,
    Forward,
    Crumb(PathBuf),
    ToolShuffle,
    ToolPin,
    Column(usize),
    Row { col: usize, row: usize },
    Prev,
    Toggle,
    Next,
    Seek,
    Volume,
    Overlay,
    Backdrop,
    ThemeRow(usize),
    ThemeList,
    ThemeKeep,
    ThemeCancel,
    OpenPath,
    OpenPlace(usize),
    OpenRow(usize),
    OpenList,
    OpenButton(OpenButton),
    PlaylistPane(usize),
    PlaylistRow(usize),
    PlaylistSong(usize),
    QueuePane,
    QueueRow(usize),
    LibraryPane(usize),
    LibraryRow { col: usize, row: usize },
    LibraryOpen(usize),
    SearchList,
    SearchRow(usize),
    SearchPlay,
    SearchClose,
    EqBadge,
    EqBand(usize),
    EqReset,
    EqPreset,
    EqToggle,
    EqClose,
    PlaylistNameOk,
    AboutLink,
    PlaylistNameCancel,
    Divider(Divider),
    Tick(TickRow),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickRow {
    Folder { col: usize, row: usize },
    Queue(usize),
    Playlist(usize),
    Library(usize),
    Search(usize),
}

impl TickRow {
    pub fn row(self) -> usize {
        match self {
            TickRow::Folder { row, .. } | TickRow::Queue(row) | TickRow::Playlist(row) | TickRow::Library(row) | TickRow::Search(row) => row,
        }
    }

    fn at(self, row: usize) -> TickRow {
        match self {
            TickRow::Folder { col, .. } => TickRow::Folder { col, row },
            TickRow::Queue(_) => TickRow::Queue(row),
            TickRow::Playlist(_) => TickRow::Playlist(row),
            TickRow::Library(_) => TickRow::Library(row),
            TickRow::Search(_) => TickRow::Search(row),
        }
    }

    pub fn row_target(self) -> Target {
        match self {
            TickRow::Folder { col, row } => Target::Row { col, row },
            TickRow::Queue(i) => Target::QueueRow(i),
            TickRow::Playlist(i) => Target::PlaylistSong(i),
            TickRow::Library(row) => Target::LibraryRow { col: library::SONGS, row },
            TickRow::Search(i) => Target::SearchRow(i),
        }
    }

    pub fn of(t: &Target) -> Option<TickRow> {
        match *t {
            Target::Tick(r) => Some(r),
            Target::Row { col, row } => Some(TickRow::Folder { col, row }),
            Target::QueueRow(i) => Some(TickRow::Queue(i)),
            Target::PlaylistSong(i) => Some(TickRow::Playlist(i)),
            Target::LibraryRow { col, row } if col == library::SONGS => Some(TickRow::Library(row)),
            Target::SearchRow(i) => Some(TickRow::Search(i)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Divider {
    Folders,
    Playlists,
    Library(usize),
    Visual,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpenButton {
    Open,
    Import,
    Play,
    Shuffle,
    Cancel,
}

pub struct Hit {
    pub rect: Rect,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenFolder,
    ImportPlaylist,
    NewPlaylist,
    RenamePlaylist(PathBuf),
    DeletePlaylist(PathBuf),
    Playlists,
    Queue,
    Library,
    LibraryShowSongs,
    LibraryShow,
    Search,
    SearchPlay,
    SearchPlayNext,
    SearchAddToQueue,
    SearchShow,
    PlayNext,
    AddToQueue,
    QueuePlay,
    QueueMoveUp,
    QueueMoveDown,
    MoveSongUp,
    MoveSongDown,
    ResetOrder,
    QueueRemove,
    ClearQueue,
    ShuffleQueue,
    SaveQueue,
    AddToPlaylistMenu(PathBuf),
    AddToPlaylist { playlist: PathBuf, song: PathBuf },
    AddManyToPlaylist { playlist: PathBuf, songs: Vec<PathBuf> },
    ClearTicks,
    OpenRecent(PathBuf),
    OpenPinned(PathBuf),
    ClearRecent,
    Quit,
    PlayPause,
    Next,
    NextAlbum,
    Previous,
    Stop,
    SeekBy(f64),
    VolumeBy(f32),
    ToggleRepeat(Repeat),
    ToggleShuffleAlbums,
    ToggleContinue,
    Reveal(PathBuf),
    CopyPath(PathBuf),
    Theme,
    Equalizer,
    EqPreset(usize),
    EqSavePreset,
    EqMyPresetMenu(u8),
    EqRenamePreset(u8),
    EqDeletePreset(u8),
    ToggleEq,
    CrossfadeMenu,
    SetCrossfade(u32),
    ReplayGainMenu,
    SetReplayGain(ReplayGain),
    SetFlipped(bool),
    ToggleCompact,
    ToggleVisual(Visual),
    MusicFolder,
    ToggleHidden,
    PlaySelection,
    ShuffleSelection,
    TogglePinSelection,
    OpenSelection,
    Help,
    About,
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: &'static str,
    pub action: Option<Action>,
    pub checked: Option<bool>,
    pub heading: bool,
}

impl MenuItem {
    fn new(label: impl Into<String>, shortcut: &'static str, action: Action) -> MenuItem {
        MenuItem { label: label.into(), shortcut, action: Some(action), checked: None, heading: false }
    }
    fn sep() -> MenuItem {
        MenuItem { label: String::new(), shortcut: "", action: None, checked: None, heading: false }
    }
    fn heading(label: &str) -> MenuItem {
        MenuItem { label: label.into(), shortcut: "", action: None, checked: None, heading: true }
    }
    pub fn is_separator(&self) -> bool {
        self.action.is_none() && self.label.is_empty()
    }
}

pub const MENUS: &[&str] = &["File", "Playback", "Settings", "Help"];

pub struct OpenMenu {
    pub items: Vec<MenuItem>,
    pub anchor: Anchor,
    pub title_index: Option<usize>,
    pub hover: Option<usize>,
    pub offset: usize,
    pub clicked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    At { x: u16, y: u16 },
    Click { col: u16, row: u16 },
}

impl Anchor {
    fn as_click(self) -> Anchor {
        match self {
            Anchor::At { x, y } => Anchor::Click { col: x, row: y.saturating_sub(1) },
            click => click,
        }
    }
}

impl OpenMenu {
    pub fn new(items: Vec<MenuItem>, anchor: Anchor) -> OpenMenu {
        OpenMenu { items, anchor, title_index: None, hover: None, offset: 0, clicked: false }
    }
}

pub struct Column {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub offset: usize,
    pub marks: crate::marks::Marks,
}

impl Column {
    fn new(dir: PathBuf, show_hidden: bool, order: &Order) -> Column {
        let entries = fsx::list_in(&dir, show_hidden, order);
        Column { dir, entries, selected: None, offset: 0, marks: Default::default() }
    }
}

pub struct ThemePicker {
    pub filter: String,
    pub list: Vec<usize>,
    pub selected: usize,
    pub offset: usize,
    pub original: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum OpenMode {
    Open,
    MusicFolder,
    ImportPlaylists,
}

pub struct OpenDialog {
    pub mode: OpenMode,
    pub path: String,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub offset: usize,
    pub places: Vec<(String, PathBuf)>,
    pub editing_path: bool,
    pub checked: Vec<PathBuf>,
}

pub enum Overlay {
    Theme(ThemePicker),
    Open(OpenDialog),
    Help,
    About,
    Equalizer,
    PlaylistName(String, NameFor),
    Search(Search),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NameFor {
    New,
    Rename(PathBuf),
    Queue,
    EqPreset,
    EqRename(u8),
}

pub struct Moves {
    done: std::collections::HashMap<PathBuf, Option<String>>,
    asked: std::collections::HashSet<PathBuf>,
    tx: std::sync::mpsc::Sender<(PathBuf, Option<String>)>,
    rx: std::sync::mpsc::Receiver<(PathBuf, Option<String>)>,
    song: Option<PathBuf>,
    find_song: bool,
    found_song: bool,
    folder: Option<(PathBuf, Vec<PathBuf>, bool)>,
}

impl Moves {
    fn new() -> Moves {
        let (tx, rx) = std::sync::mpsc::channel();
        Moves { done: Default::default(), asked: Default::default(), tx, rx, song: None, find_song: false, found_song: false, folder: None }
    }

    fn ask(&mut self, paths: &[PathBuf]) {
        let new: Vec<PathBuf> = paths.iter().filter(|p| !self.done.contains_key(*p) && !self.asked.contains(*p)).cloned().collect();
        if new.is_empty() {
            return;
        }
        self.asked.extend(new.iter().cloned());
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new().name("urchin-fingerprint".into()).spawn(move || {
            for p in new {
                let print = crate::fingerprint::of(&p);
                if tx.send((p, print)).is_err() {
                    break;
                }
            }
        });
    }

    fn collect(&mut self) {
        while let Ok((p, print)) = self.rx.try_recv() {
            self.asked.remove(&p);
            self.done.insert(p, print);
        }
    }

    fn get(&self, p: &Path) -> Option<Option<&String>> {
        self.done.get(p).map(|o| o.as_ref())
    }
}

fn shared_folder(songs: &[PathBuf]) -> Option<PathBuf> {
    let mut dir = songs.first()?.parent()?.to_path_buf();
    while !songs.iter().all(|s| s.starts_with(&dir)) {
        dir = dir.parent()?.to_path_buf();
    }
    Some(dir)
}

fn next_folder(from: &Path, root: &Path, show_hidden: bool, songs_of: impl Fn(&Path) -> Vec<PathBuf>) -> Option<(PathBuf, Vec<PathBuf>)> {
    let mut at = from.to_path_buf();
    while at != root && at.starts_with(root) {
        let parent = at.parent()?.to_path_buf();
        let folders: Vec<PathBuf> = fsx::list(&parent, show_hidden).into_iter().filter(|e| e.kind == Kind::Folder).map(|e| e.path).collect();
        if let Some(i) = folders.iter().position(|f| *f == at) {
            for f in &folders[i + 1..] {
                let songs = songs_of(f);
                if !songs.is_empty() {
                    return Some((f.clone(), songs));
                }
            }
        }
        at = parent;
    }
    None
}

fn resume_place(songs: &[PathBuf], r: &crate::settings::Resume) -> Option<(usize, f64)> {
    let found = songs.iter().enumerate().filter(|(_, s)| **s == r.song).map(|(i, _)| i).min_by_key(|i| i.abs_diff(r.index));
    match found {
        Some(i) => Some((i, r.seconds)),
        None if r.index < songs.len() => Some((r.index, 0.0)),
        None => None,
    }
}

fn file_items(path: &Path) -> [MenuItem; 2] {
    [MenuItem::new(crate::reveal::label(), "", Action::Reveal(path.to_path_buf())), MenuItem::new("Copy Path", "", Action::CopyPath(path.to_path_buf()))]
}

pub(crate) fn refresh_stale_prints(store: &Mutex<eq::Store>) {
    let (songs, folders) = store.lock().unwrap().stale_prints();
    for song in songs {
        let print = crate::fingerprint::of(&song);
        store.lock().unwrap().refresh_print(&song, print);
    }
    for dir in folders {
        let prints: Vec<String> = folder_print_songs(&dir).iter().filter_map(|p| crate::fingerprint::of(p)).collect();
        store.lock().unwrap().refresh_folder_prints(&dir, prints);
    }
    store.lock().unwrap().save_if_changed();
}

fn folder_print_songs(dir: &Path) -> Vec<PathBuf> {
    fsx::list(dir, true).into_iter().filter(|e| e.kind == Kind::Track).take(eq::FOLDER_PRINTS).map(|e| e.path).collect()
}

impl NameFor {
    pub fn is_eq(&self) -> bool {
        matches!(self, NameFor::EqPreset | NameFor::EqRename(_))
    }
}

type Revealed = (PathBuf, crate::reveal::Shown);

pub type LabelKey = (usize, u16, u16, u32);

pub struct App {
    pub settings: Settings,
    pub engine: Engine,
    pub eq_store: Arc<Mutex<Store>>,
    pub eq_band: usize,
    pub eq_rect: Rect,
    pub eq_preset_x: u16,
    pub key_focus: Option<Target>,
    pub scope_keys: bool,
    pub content_rect: Rect,
    pub free_rect: Rect,
    pub tap: Arc<crate::spectrum::Tap>,
    pub visuals: Visuals,
    viz_frozen: bool,
    viz_paused_on: Option<(u64, usize, Option<PathBuf>)>,
    viz_thawed: bool,
    trace_for: Option<(PathBuf, u64, u64)>,
    pub label_cache: Option<(LabelKey, Vec<(usize, u16)>)>,
    pub solo: bool,
    solo_for: Option<u64>,
    solo_seen: bool,
    resume_solo: Option<(u64, PathBuf, f64)>,
    pub status: Status,
    media: Option<crate::mediakeys::Media>,
    kept: Option<queue::Keeper>,
    left_off: Option<(crate::settings::Resume, f64)>,
    left_off_saved: Option<(Option<PathBuf>, State)>,
    follow: Option<(PathBuf, bool)>,
    pub osc_out: Vec<String>,
    reveals: (Sender<Revealed>, Receiver<Revealed>),
    order: Order,
    tag_cache: Option<PathBuf>,
    moves: Option<Moves>,
    pub theme: usize,
    pub root: PathBuf,
    pub levels: Vec<Column>,
    dir_check: Instant,
    watch_until: Option<Instant>,
    dir_stamps: Vec<(PathBuf, Option<SystemTime>)>,
    pub focus: usize,
    pub playlists: Option<playlists::View>,
    pub queue_view: Option<queue::View>,
    pub library: Option<library::View>,
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,
    pub menu: Option<OpenMenu>,
    pub overlay: Option<Overlay>,
    pub hits: Vec<Hit>,
    pub hover: Option<Target>,
    pub menu_peek: bool,
    pub menu_peek_held: bool,
    pub pointer_row: Option<u16>,
    pub toolbar_row: u16,
    pub toolbar_peek_until: Option<Instant>,
    pub toolbar_woken: bool,
    pub drag: Option<Target>,
    pub drag_seek: Option<f64>,
    drag_grab: i32,
    tick_drag: Option<(TickRow, bool)>,
    pub dividers: crate::ui::Dividers,
    pub panes_rect: Rect,
    pub analyzer_rect: Rect,
    pub screen_h: u16,
    pub seek_rect: Rect,
    pub volume_rect: Rect,
    pub column_height: usize,
    pub list_height: usize,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    last_click: Option<(Instant, Target)>,
    last_p: Option<Instant>,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DIR_CHECK: Duration = Duration::from_secs(1);
const WATCH_FOR: Duration = Duration::from_secs(10 * 60);

fn dir_time(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}
const WHEEL_LINES: usize = 3;

impl App {
    pub fn new(start: Option<PathBuf>) -> App {
        let settings = Settings::load();
        let theme = theme::index_of(&settings.theme).unwrap_or_else(theme::default_index);
        let eq_store = Arc::new(Mutex::new(Store::load()));
        let engine = Engine::start(settings.volume, eq_store.clone(), settings.eq_enabled);
        engine.set_repeat(settings.repeat);
        engine.set_crossfade(settings.crossfade);
        engine.set_replaygain(settings.replaygain);
        let tap = engine.tap();
        let root = settings
            .music_root
            .clone()
            .filter(|p| p.is_dir())
            .unwrap_or_else(fsx::default_music_dir);
        let mut app = App {
            levels: vec![Column::new(root.clone(), settings.show_hidden, &Order::empty())],
            dir_check: Instant::now(),
            watch_until: None,
            dir_stamps: Vec::new(),
            settings,
            engine,
            eq_store,
            eq_band: 1,
            eq_rect: Rect::default(),
            eq_preset_x: 0,
            key_focus: None,
            scope_keys: false,
            content_rect: Rect::default(),
            free_rect: Rect::default(),
            tap,
            visuals: Visuals::default(),
            viz_frozen: false,
            viz_paused_on: None,
            viz_thawed: false,
            trace_for: None,
            label_cache: None,
            solo: false,
            solo_for: None,
            solo_seen: false,
            resume_solo: None,
            status: Status::default(),
            media: None,
            kept: None,
            left_off: None,
            left_off_saved: None,
            follow: None,
            osc_out: Vec::new(),
            reveals: std::sync::mpsc::channel(),
            order: Order::empty(),
            tag_cache: None,
            moves: None,
            theme,
            root,
            focus: 0,
            playlists: None,
            queue_view: None,
            library: None,
            back: Vec::new(),
            forward: Vec::new(),
            menu: None,
            overlay: None,
            hits: Vec::new(),
            hover: None,
            menu_peek: false,
            menu_peek_held: false,
            pointer_row: None,
            toolbar_row: 0,
            toolbar_peek_until: None,
            toolbar_woken: false,
            drag: None,
            drag_seek: None,
            drag_grab: 0,
            tick_drag: None,
            dividers: Vec::new(),
            panes_rect: Rect::default(),
            analyzer_rect: Rect::default(),
            screen_h: 0,
            seek_rect: Rect::default(),
            volume_rect: Rect::default(),
            column_height: 10,
            list_height: 10,
            toast: None,
            quit: false,
            last_click: None,
            last_p: None,
        };
        if !app.levels[0].entries.is_empty() {
            app.levels[0].selected = Some(0);
            app.sync_preview();
        }
        if let Some(dir) = start.filter(|p| p.is_dir()) {
            app.open_folder(&dir);
        }
        app
    }

    pub fn palette(&self) -> Palette {
        THEMES[self.theme].palette()
    }

    pub fn scope_mode(&self) -> Option<crate::scopes::Mode> {
        self.settings.analyzer.then(|| self.settings.visual.scope_mode()).flatten()
    }

    fn show_scope(&mut self, mode: crate::scopes::Mode) {
        self.settings.visual = mode.into();
        self.settings.save();
    }

    fn toggle_visual(&mut self, visual: Visual) {
        if (self.settings.analyzer, self.settings.visual) == (true, visual) {
            self.settings.analyzer = false;
        } else {
            self.settings.visual = visual;
            self.settings.analyzer = true;
        }
        self.settings.save();
    }

    pub fn trace_seed(&mut self) -> u64 {
        let length_ms = (self.status.duration * 1000.0) as u64;
        let Some(path) = self.status.current() else {
            return self.trace_for.as_ref().map_or(0, |t| t.2);
        };
        if let Some((known, ms, seed)) = &self.trace_for {
            if known == path && *ms == length_ms {
                return *seed;
            }
        }
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        let seed = crate::viz::song_seed(size, length_ms);
        self.trace_for = Some((path.clone(), length_ms, seed));
        seed
    }

    pub fn frame_interval(&self) -> Duration {
        let moving = self.visuals.is_active(self.settings.visual, &self.tap);
        let animating = self.settings.analyzer && !self.viz_frozen && (self.status.state == State::Playing || moving);
        let ms = if animating {
            33
        } else if !self.engine.caught_up(&self.status) || matches!(&self.overlay, Some(Overlay::Search(s)) if s.indexing())
            || self.library.as_ref().is_some_and(|v| v.reading())
        {
            20
        } else if self.status.state == State::Playing {
            250
        } else {
            500
        };
        Duration::from_millis(ms)
    }

    pub fn tick(&mut self) {
        self.status = self.engine.status();
        self.follow_pause();
        let (len, h) = (self.status.queue.len(), self.column_height);
        if let Some(v) = self.queue_view.as_mut() {
            v.clamp(len, h);
        }
        let h = self.column_height;
        if let Some(v) = self.library.as_mut() {
            v.poll();
            v.fit(h);
        }
        self.poll_media();
        self.save_settled();
        self.track_left_off();
        self.continue_into_next_folder();
        self.reveal_results();
        self.follow_moves();
        self.watch_folders();
        self.check_solo();
        if let Some(s) = self.search_mut() {
            s.poll();
        }
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(4) {
                self.toast = None;
            }
        }
    }

    pub(crate) fn follow_pause(&mut self) {
        let was = self.viz_frozen;
        let st = &self.status;
        if st.state == State::Paused && !st.output_lost {
            let song = (st.plays, st.index, st.current().cloned());
            let on = self.viz_paused_on.get_or_insert_with(|| song.clone());
            self.viz_frozen = *on == song;
        } else {
            self.viz_paused_on = None;
            self.viz_frozen = false;
        }
        if was && !self.viz_frozen {
            self.viz_thawed = true;
        }
    }

    pub fn viz_hold(&mut self) -> bool {
        let hold = self.viz_frozen || self.viz_thawed;
        self.viz_thawed = false;
        hold
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn set_volume(&mut self, v: f32) {
        let v = (v * 100.0).round() / 100.0;
        self.engine.set_volume(v);
        if v > 0.0 {
            self.settings.volume = v.clamp(0.0, 1.0);
        }
    }

    pub fn save_on_exit(&mut self) {
        self.status = self.engine.status();
        self.save_waiting();
        if self.kept.is_some() {
            self.save_left_off();
        }
        self.eq_store.lock().unwrap().save_if_changed();
        let v = self.engine.volume();
        if v > 0.0 {
            self.settings.volume = v;
        }
        self.settings.save();
    }

}

fn load_dialog_dir(d: &mut OpenDialog, dir: &Path, show_hidden: bool) {
    d.dir = dir.to_path_buf();
    d.entries = if d.mode == OpenMode::ImportPlaylists {
        fsx::list_playlists(dir, show_hidden)
    } else {
        fsx::subfolders(dir, show_hidden)
    };
    d.selected = 0;
    d.offset = 0;
    d.path = fsx::contract_tilde(&dir.to_string_lossy());
}

pub fn short_name_cow(p: &Path) -> Cow<'_, str> {
    p.file_name().map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy())
}

pub fn short_name(p: &Path) -> String {
    short_name_cow(p).into_owned()
}

pub fn menu_title_x(index: usize) -> u16 {
    let mut x = 2u16;
    for m in MENUS.iter().take(index) {
        x += m.chars().count() as u16 + 3;
    }
    x
}

pub fn menu_title_at(col: u16) -> Option<usize> {
    MENUS.iter().enumerate().find_map(|(i, name)| {
        let x = menu_title_x(i);
        (col + 1 >= x && col < x + name.chars().count() as u16 + 1).then_some(i)
    })
}

const CROSSFADE_CHOICES: [u32; 7] = [0, 2, 4, 6, 8, 10, crate::audio::MAX_CROSSFADE];

fn crossfade_label(seconds: u32) -> String {
    if seconds == 0 {
        "Crossfade: Off".to_string()
    } else {
        format!("Crossfade: {seconds} s")
    }
}

fn replaygain_name(m: ReplayGain) -> &'static str {
    match m {
        ReplayGain::Off => "Off",
        ReplayGain::Track => "Track",
        ReplayGain::Album => "Album",
    }
}

fn plural_songs(n: usize) -> String {
    if n == 1 { "1 song".to_string() } else { format!("{n} songs") }
}

const EQ_BUTTONS: [Target; 4] = [Target::EqPreset, Target::EqReset, Target::EqToggle, Target::EqClose];

const VISUALIZER_MENU: [(&str, Visual); 12] = [
    ("Bokeh", Visual::Bokeh),
    ("Bokeh Light", Visual::BokehLight),
    ("Club Lights", Visual::ClubLights),
    ("Galaxy", Visual::Galaxy),
    ("Lines", Visual::Lines),
    ("Oscilloscope", Visual::Oscilloscope),
    ("Spectroscope", Visual::Spectroscope),
    ("Stereo", Visual::Spectrum),
    ("SynthMoonWave", Visual::SynthMoonWave),
    ("Synthwave", Visual::Synthwave),
    ("Trace", Visual::Trace),
    ("Vectorscope", Visual::Vectorscope),
];

pub const SOLO_AFTER_SECS: f64 = 5.0;

pub const TOOLBAR_PEEK: Duration = Duration::from_secs(3);

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_string()
    } else {
        many.replacen("the ", &format!("the {n} "), 1)
    }
}
