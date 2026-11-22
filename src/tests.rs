#![allow(clippy::too_many_arguments, reason = "test drivers spell out every input")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use crate::settings::Visual;
use crate::app::{Action, App, MenuItem, NameFor, OpenMenu, Overlay, Target};
use crate::{theme, ui};

static LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn serial() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn write_wav(path: &Path, rate: u32, secs: f32) {
    let frames = (rate as f32 * secs) as u32;
    let mut data = Vec::with_capacity(frames as usize * 4);
    for i in 0..frames {
        let v = ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, out).unwrap();
}

fn library() -> PathBuf {
    let root = std::env::temp_dir().join(format!("urchin_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    fs::create_dir_all(&album).unwrap();
    for (i, name) in ["01 - Filler", "02 - I Don't Wanna Hear It", "10 - Seeing Red"].iter().enumerate() {
        write_wav(&album.join(format!("{name}.wav")), if i == 0 { 44_100 } else { 48_000 }, 0.5);
    }
    for g in ["ambient", "hip-hop", "jazz", "r&b", "rock", "techno"] {
        fs::create_dir_all(root.join("Music").join(g)).unwrap();
    }
    fs::create_dir_all(root.join("Music/punk/crust")).unwrap();
    fs::write(root.join("Music/punk/notes.txt"), "not music").unwrap();
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    root
}

fn draw_at(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

fn render_at(app: &mut App, w: u16, h: u16) -> String {
    let buf = draw_at(app, w, h);
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

fn render(app: &mut App) -> String {
    render_at(app, 110, 30)
}

fn click_at(app: &mut App, x: u16, y: u16, button: MouseButton) {
    let ev = |kind| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    app.on_mouse(ev(MouseEventKind::Down(button)));
    app.on_mouse(ev(MouseEventKind::Up(button)));
}

fn rect_of(app: &App, pred: impl Fn(&Target) -> bool) -> ratatui::layout::Rect {
    app.hits.iter().rev().find(|h| pred(&h.target)).map(|h| h.rect).expect("target on screen")
}

fn click(app: &mut App, pred: impl Fn(&Target) -> bool) {
    let t = app.hits.iter().rev().find(|h| pred(&h.target)).map(|h| h.target.clone()).expect("target on screen");
    let r = rect_of(app, |x| *x == t);
    let is_row = !matches!(t, Target::Tick(_)) && crate::app::TickRow::of(&t).is_some();
    click_at(app, r.x + if is_row { 3 } else { 1 }, r.y, MouseButton::Left);
}

fn row_named(app: &App, col: usize, name: &str) -> usize {
    app.levels[col].entries.iter().position(|e| e.name == name).unwrap()
}

fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn decodes_and_resamples() {
    let _g = serial();
    let root = library();
    let f = root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav");
    let mut src = crate::audio::test_open(&f, 48_000).unwrap();
    let mut frames = 0;
    while let Some(chunk) = src.next() {
        frames += chunk.len() / 2;
    }
    assert!((frames as i64 - 24_000).abs() < 1_200, "got {frames} frames");
}

#[test]
fn mouse_browsing_menus_theme_and_open_dialog() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let mut app = App::new(Some(music.clone()));
    render(&mut app);

    let names: Vec<_> = app.levels[0].entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["ambient", "hip-hop", "jazz", "punk", "r&b", "rock", "techno"]);

    let punk = row_named(&app, 0, "punk");
    click(&mut app, |t| *t == Target::Row { col: 0, row: punk });
    assert_eq!(app.levels.len(), 2);
    assert_eq!(app.levels[1].dir, music.join("punk"));
    render(&mut app);

    let hc = row_named(&app, 1, "hardcore");
    click(&mut app, |t| *t == Target::Row { col: 1, row: hc });
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col: 2, row: 0 });
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col: 3, row: 0 });
    let screen = render(&mut app);
    assert!(screen.contains("01 - Filler"), "{screen}");
    let panes: Vec<_> = app.hits.iter().filter_map(|h| match h.target { Target::Column(c) => Some(c), _ => None }).collect();
    assert_eq!(panes, [3, 4], "{screen}");
    assert!(!screen.contains("crust") && !screen.contains("techno"), "{screen}");
    let tracks: Vec<_> = app.levels[4].entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(tracks[1], "02 - I Don't Wanna Hear It.wav");
    assert_eq!(tracks[2], "10 - Seeing Red.wav");

    click(&mut app, |t| *t == Target::Crumb(music.join("punk")));
    assert_eq!(app.levels[app.focus].dir, music.join("punk"));
    render(&mut app);

    let focus = app.focus;
    let r = rect_of(&app, |t| *t == Target::Column(focus));
    app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: r.x + 2, row: r.y + 2, modifiers: KeyModifiers::NONE });

    click(&mut app, |t| *t == Target::Menu(0));
    assert!(app.menu.is_some());
    render(&mut app);
    let open_item = app.menu.as_ref().unwrap().items.iter().position(|i| i.action == Some(Action::OpenFolder)).unwrap();
    click(&mut app, |t| *t == Target::MenuItem(open_item));
    assert!(matches!(app.overlay, Some(Overlay::Open(_))));
    let screen = render(&mut app);
    assert!(screen.contains("File › Open Folder"), "{screen}");

    click(&mut app, |t| *t == Target::OpenPath);
    if let Some(Overlay::Open(d)) = app.overlay.as_mut() {
        d.path = music.join("tech").to_string_lossy().to_string();
    }
    key(&mut app, KeyCode::Tab);
    if let Some(Overlay::Open(d)) = &app.overlay {
        assert!(d.path.ends_with("techno/"), "completed to {}", d.path);
    }
    key(&mut app, KeyCode::Enter);
    render(&mut app);
    click(&mut app, |t| *t == Target::OpenButton(crate::app::OpenButton::Open));
    assert!(app.overlay.is_none());
    assert_eq!(app.levels[app.focus].dir, music.join("techno"));
    assert_eq!(app.settings.recent.first(), Some(&music.join("techno")));
    render(&mut app);

    let items = app.file_menu_items();
    assert!(items.iter().any(|i| i.action == Some(Action::OpenRecent(music.join("techno")))));

    click(&mut app, |t| *t == Target::Menu(2));
    render(&mut app);
    let ti = app.menu.as_ref().unwrap().items.iter().position(|it| it.action == Some(Action::Theme)).unwrap();
    click(&mut app, |t| *t == Target::MenuItem(ti));
    assert!(matches!(app.overlay, Some(Overlay::Theme(_))));
    for c in "gruvbox dark".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    let previewed = theme::THEMES[app.theme].name;
    assert!(previewed.to_lowercase().contains("gruvbox"), "previewing {previewed}");
    let screen = render(&mut app);
    assert!(screen.contains("Settings › Theme"));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.theme, previewed);

    app.run(Action::Theme);
    render(&mut app);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Esc);
    assert_eq!(theme::THEMES[app.theme].name, previewed);

    render(&mut app);
    let r = rect_of(&app, |t| matches!(t, Target::Row { .. }));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    assert!(app.menu.as_ref().is_some_and(|m| m.title_index.is_none()));
    key(&mut app, KeyCode::Esc);

    let again = App::new(None);
    assert_eq!(again.settings.theme, previewed);
    assert_eq!(again.settings.recent.first(), Some(&music.join("techno")));

    println!("{}", render(&mut app));
}

#[test]
fn every_theme_loads() {
    assert!(theme::THEMES.len() > 900);
    for t in theme::THEMES {
        let _ = t.palette();
    }
    assert!(theme::index_of("Dracula").is_some());
}

fn rms_through(curve: &crate::eq::Curve, freq: f32) -> f32 {
    let rate = 48_000u32;
    let mut proc = crate::eq::Processor::new(rate);
    proc.set_curve(curve);
    let mut sum = 0.0f64;
    let n = rate as usize;
    for i in 0..n {
        let x = (i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * 0.25;
        let (l, _) = proc.process(x, x);
        if i > n / 10 {
            sum += (l as f64) * (l as f64);
        }
    }
    (sum / (n as f64 * 0.9)).sqrt() as f32
}

#[test]
fn equalizer_shapes_the_sound() {
    use crate::eq::Curve;
    let flat = Curve::default();
    let mut boost = Curve::default();
    boost.set(6, 12.0);
    let db = |a: f32, b: f32| 20.0 * (a / b).log10();
    assert!((db(rms_through(&flat, 1000.0), rms_through(&flat, 62.0))).abs() < 0.1);
    let diff = db(rms_through(&boost, 1000.0), rms_through(&boost, 62.0));
    assert!(diff > 10.0 && diff < 13.0, "1 kHz vs 62 Hz: {diff:.1} dB");
    assert!(rms_through(&boost, 1000.0) <= 0.25 * 0.7072);
}

#[test]
fn equalizer_remembers_each_song_automatically() {
    let _g = serial();
    let root = library();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let song_a = album.join("01 - Filler.wav");
    let song_b = album.join("10 - Seeing Red.wav");
    let mut app = App::new(Some(root.join("Music")));

    render(&mut app);
    click(&mut app, |t| *t == Target::Menu(2));
    render(&mut app);
    let items = app.menu.as_ref().unwrap().items.clone();
    let i = items.iter().position(|it| it.action == Some(Action::Equalizer)).unwrap();
    click(&mut app, |t| *t == Target::MenuItem(i));
    assert!(matches!(app.overlay, Some(Overlay::Equalizer)));
    let screen = render(&mut app);
    assert!(screen.contains("Settings › Equalizer"));
    assert!(screen.contains("Nothing playing"));

    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![song_a.clone(), song_b.clone()]);
    app.status.index = 0;
    render(&mut app);

    let r = rect_of(&app, |t| *t == Target::EqBand(6));
    let top = app.eq_rect.y;
    click_at(&mut app, r.x + 1, top, MouseButton::Left);
    assert_eq!(app.eq_store.lock().unwrap().get(&song_a).gains[5], 20.0);

    let r4 = rect_of(&app, |t| *t == Target::EqBand(8));
    app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: r4.x + 1, row: r4.y + 2, modifiers: KeyModifiers::NONE });
    assert_eq!(app.eq_store.lock().unwrap().get(&song_a).gains[7], 0.5);
    let screen = render(&mut app);
    assert!(screen.contains("automatically saved & remembered for each song"), "{screen}");

    app.status.index = 1;
    assert!(app.eq_curve().is_flat());
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    let band_b = app.eq_band;
    let b = app.eq_store.lock().unwrap().get(&song_b);
    assert_eq!(b.get(band_b), 1.0);

    app.status.index = 0;
    assert_eq!(app.eq_curve().gains[5], 20.0);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());

    let store = crate::eq::Store::load();
    assert_eq!(store.get(&song_a).gains[5], 20.0);
    assert_eq!(store.get(&song_b).get(band_b), 1.0);
    app.run(Action::Equalizer);
    render(&mut app);
    click(&mut app, |t| *t == Target::EqReset);
    assert!(!app.eq_store.lock().unwrap().has(&song_a));
}

#[test]
#[ignore]
fn formats() {
    let dir = std::env::var("URCHIN_FORMAT_SAMPLES").expect("set URCHIN_FORMAT_SAMPLES");
    let mut files: Vec<_> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let listed = if crate::fsx::is_audio(&f) { "shown " } else { "HIDDEN" };
        match crate::audio::test_open(&f, 48_000) {
            Ok(mut src) => {
                let (mut frames, mut peak) = (0usize, 0f32);
                while let Some(chunk) = src.next() {
                    frames += chunk.len() / 2;
                    peak = chunk.iter().fold(peak, |p, s| p.max(s.abs()));
                    if frames > 48_000 * 30 { break; }
                }
                println!("{name:<22} {listed} PLAYS {:.2}s peak {peak:.3}", frames as f64 / 48_000.0);
            }
            Err(e) => println!("{name:<22} {listed} FAIL  {e}"),
        }
    }
}

#[test]
#[ignore]
fn decode_report() {
    use std::fmt::Write as _;
    let src = PathBuf::from(std::env::var("URCHIN_DECODE_FILES").expect("set URCHIN_DECODE_FILES"));
    let out = std::env::var("URCHIN_DECODE_REPORT").expect("set URCHIN_DECODE_REPORT");
    let mut files: Vec<PathBuf> = if src.is_dir() {
        fn walk(d: &Path, into: &mut Vec<PathBuf>) {
            for e in fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, into);
                } else if crate::fsx::is_audio(&p) {
                    into.push(p);
                }
            }
        }
        let mut v = Vec::new();
        walk(&src, &mut v);
        v
    } else {
        fs::read_to_string(&src).unwrap().lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()
    };
    files.sort();
    let hash = |h: &mut u64, l: &[f32], r: &[f32]| {
        for (a, b) in l.iter().zip(r) {
            for byte in a.to_bits().to_le_bytes().into_iter().chain(b.to_bits().to_le_bytes()) {
                *h = (*h ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    };
    let mut report = String::new();
    for f in &files {
        let name = if src.is_dir() { f.strip_prefix(&src).unwrap_or(f) } else { f.as_path() };
        let _ = writeln!(report, "== {}", name.display());
        let mut o = match crate::decode::open(f) {
            Ok(o) => o,
            Err(e) => {
                let _ = writeln!(report, "   open failed: {e}");
                continue;
            }
        };
        let _ = writeln!(report, "   rate {}  duration {:.6}", o.rate, o.duration);
        let _ = writeln!(report, "   meta {:?}", o.meta);
        let _ = writeln!(report, "   gain {:?}", o.gain);
        let _ = writeln!(report, "   tags {:?}", crate::library::read_tags(f));
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let (mut frames, mut h, mut lead) = (0usize, 0xcbf29ce484222325u64, None);
        loop {
            l.clear();
            r.clear();
            let more = o.backend.read(&mut l, &mut r);
            if lead.is_none() {
                if let Some(i) = l.iter().zip(&r).position(|(a, b)| a.abs() > 1e-4 || b.abs() > 1e-4) {
                    lead = Some(frames + i);
                }
            }
            hash(&mut h, &l, &r);
            frames += l.len();
            if !more {
                break;
            }
        }
        if let Ok(dir) = std::env::var("URCHIN_DECODE_DUMP") {
            let mut o = crate::decode::open(f).unwrap();
            let mut bytes = Vec::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            while o.backend.read(&mut l, &mut r) {}
            for (a, b) in l.iter().zip(&r) {
                bytes.extend_from_slice(&a.to_le_bytes());
                bytes.extend_from_slice(&b.to_le_bytes());
            }
            let flat = name.to_string_lossy().replace('/', "__");
            fs::write(Path::new(&dir).join(format!("{flat}.f32")), bytes).unwrap();
        }
        let _ = writeln!(
            report,
            "   decoded {frames} frames ({:.6} s), first sound at frame {:?}, samples {h:016x}",
            frames as f64 / o.rate as f64,
            lead
        );
        let _ = writeln!(report, "   fingerprint {:?}", crate::fingerprint::of(f));
        for at in [1.0, o.duration * 0.5, (o.duration - 2.0).max(0.0)] {
            let mut o = crate::decode::open(f).unwrap();
            let got = o.backend.seek(at);
            let (mut l, mut r, mut h) = (Vec::new(), Vec::new(), 0xcbf29ce484222325u64);
            while l.len() < 4096 && o.backend.read(&mut l, &mut r) {}
            if let Ok(dir) = std::env::var("URCHIN_DECODE_DUMP") {
                while l.len() < 4096 * 8 && o.backend.read(&mut l, &mut r) {}
                let bytes: Vec<u8> = l.iter().take(4096 * 8).flat_map(|a| a.to_le_bytes()).collect();
                let flat = name.to_string_lossy().replace('/', "__");
                fs::write(Path::new(&dir).join(format!("{flat}.seek{at:.3}.f32")), bytes).unwrap();
            }
            l.truncate(4096);
            r.truncate(4096);
            hash(&mut h, &l, &r);
            let _ = writeln!(report, "   seek {at:.3} -> {got:.6}, then {} frames {h:016x}", l.len());
        }
    }
    fs::write(&out, &report).unwrap();
    println!("{} files, report in {out}", files.len());
}

#[test]
fn now_playing_title_pulses_slowly() {
    use crate::ui::pulse;
    assert_eq!(pulse(0.0), 0.0);
    assert!((pulse(4.0) - 0.7).abs() < 1e-6);
    assert!(pulse(2.0) > 0.3 && pulse(2.0) < 0.4);
    assert!(pulse(8.0) < 1e-6);
}

#[test]
fn a_title_too_long_for_its_slot_crawls_slowly_right_to_left() {
    use crate::ui::marquee;
    use unicode_width::UnicodeWidthStr;
    let title = "A Title Long Enough To Need Cutting Short";
    assert_eq!(marquee("Short", 20, 40.0), "Short");
    assert_eq!(marquee(title, 10, 0.0), "A Title Lo");
    assert_eq!(marquee(title, 10, 2.9), "A Title Lo");
    assert_eq!(marquee(title, 10, 3.5), " Title Lon");
    assert_eq!(marquee(title, 10, 4.0), "Title Long");
    let lap = title.width() + 6;
    let end = 3.0 + (title.width() - 4) as f64 * 0.5;
    assert_eq!(marquee(title, 10, end), "hort      ");
    assert_eq!(marquee(title, 10, 3.0 + (lap - 3) as f64 * 0.5), "   A Title");
    let cycle = 3.0 + lap as f64 * 0.5;
    assert_eq!(marquee(title, 10, cycle + 1.0), "A Title Lo");
    for i in 0..200 {
        let t = i as f64 * 0.25;
        assert_eq!(marquee(title, 10, t).width(), 10);
        assert_eq!(marquee("東京の夜 — 坂本龍一 · アルバム", 9, t).width(), 9);
    }
}

#[test]
fn vlc_presets_per_song_and_tweaks_are_kept() {
    use crate::eq::{preset_curve, preset_name, Store, PRESET_COUNT};
    let _g = serial();
    let root = library();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let song_a = album.join("01 - Filler.wav");
    let song_b = album.join("10 - Seeing Red.wav");

    assert_eq!(PRESET_COUNT, 33);
    assert_eq!(preset_name(3), "Dance");
    assert_eq!(preset_curve(0).pre, 0.0);
    assert!(preset_curve(0).gains.iter().all(|g| *g == 0.0));
    let dance = preset_curve(3);
    assert_eq!(dance.pre, 0.0);
    assert_eq!(dance.gains[0], 9.6);
    assert_eq!(dance.gains[5], 0.0);
    assert_eq!(dance.gains[9], 0.0);
    assert!(dance.gains[1] < 9.6 && dance.gains[1] > 7.2);

    let mut app = App::new(Some(root.join("Music")));
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![song_a.clone(), song_b.clone()]);
    app.status.index = 0;
    app.run(Action::Equalizer);
    render(&mut app);

    click(&mut app, |t| *t == Target::EqPreset);
    let screen = render(&mut app);
    let i = app.menu.as_ref().unwrap().items.iter().position(|it| it.action == Some(Action::EqPreset(3))).unwrap();
    assert!(screen.contains("Full bass and treble"), "{screen}");
    click(&mut app, |t| *t == Target::MenuItem(i));
    assert_eq!(app.eq_curve(), dance);
    assert!(render(&mut app).contains("Preset: Dance ▾"));

    app.eq_band = 6;
    key(&mut app, KeyCode::Up);
    let tweaked = app.eq_curve();
    assert_eq!(tweaked.gains[5], 0.5);
    assert_eq!(tweaked.gains[0], dance.gains[0]);
    assert!(render(&mut app).contains("Preset: Dance (customized) ▾"));

    app.status.index = 1;
    assert_eq!(app.eq_curve(), dance);
    let order = crate::eq::preset_order();
    let at = order.iter().position(|&o| o == 3).unwrap();
    key(&mut app, KeyCode::Char(']'));
    assert_eq!(app.eq_curve(), preset_curve(order[at + 1]));
    key(&mut app, KeyCode::Char(']'));
    assert_eq!(app.eq_curve(), preset_curve(order[at + 2]));

    key(&mut app, KeyCode::Esc);
    let store = Store::load();
    assert_eq!(store.get(&song_a), tweaked);
    assert_eq!(store.get(&song_a).label(), "Dance (customized)");
    assert_eq!(store.get(&song_b).label(), "Drum & bass");
}

#[test]
fn equalizer_works_from_the_keyboard_without_arrow_keys() {
    use crate::eq::preset_curve;
    let _g = serial();
    let root = library();
    let song = root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav");
    let mut app = App::new(Some(root.join("Music")));
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![song]);
    app.status.index = 0;
    app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert!(matches!(app.overlay, Some(Overlay::Equalizer)));
    render(&mut app);

    for _ in app.eq_band..10 {
        key(&mut app, KeyCode::Tab);
    }
    assert_eq!(app.eq_band, 10);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.key_focus, Some(Target::EqPreset));
    assert!(render(&mut app).contains("▸ Preset: Flat ▾ ◂"));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.key_focus, Some(Target::EqReset));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.key_focus, Some(Target::EqToggle));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.key_focus, Some(Target::EqClose));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.key_focus, Some(Target::EqPreset));
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.key_focus, None);
    assert_eq!(app.eq_band, 10);
    key(&mut app, KeyCode::Tab);

    key(&mut app, KeyCode::Enter);
    assert!(app.menu.is_some());
    let ctrl = |app: &mut App, c: char| app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    ctrl(&mut app, 'n');
    ctrl(&mut app, 'n');
    ctrl(&mut app, 'p');
    for _ in 0..2 {
        key(&mut app, KeyCode::Char('j'));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.menu.is_none());
    assert_eq!(app.eq_curve(), preset_curve(crate::eq::preset_order()[3]));
    assert!(matches!(app.overlay, Some(Overlay::Equalizer)));

    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('l'));
    let on = app.settings.eq_enabled;
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.eq_enabled, !on);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
}

fn peak_rss_mb() -> f64 {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        let kb = status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok());
        if let Some(kb) = kb {
            return kb / 1024.0;
        }
    }
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let raw = u.ru_maxrss as f64;
    if cfg!(target_os = "macos") { raw / 1_048_576.0 } else { raw / 1024.0 }
}

#[cfg(target_os = "macos")]
const MEMORY_BUDGET_MB: f64 = 60.0;
#[cfg(not(target_os = "macos"))]
const MEMORY_BUDGET_MB: f64 = 50.0;

#[test]
#[ignore]
fn memory_probe() {
    if std::env::var("URCHIN_MEMORY_PROBE").is_err() {
        return;
    }
    let root = std::env::temp_dir().join(format!("urchin_mem_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let music = root.join("Music");
    for g in 0..20 {
        for a in 0..25 {
            for al in 0..4 {
                let dir = music.join(format!("genre {g}/artist {a}/album {al}"));
                fs::create_dir_all(&dir).unwrap();
                for t in 0..12 {
                    fs::write(dir.join(format!("{t:02} - song.mp3")), b"").unwrap();
                }
            }
        }
    }
    let album = root.join("real");
    fs::create_dir_all(&album).unwrap();
    let songs: Vec<PathBuf> = (0..4)
        .map(|i| {
            let f = album.join(format!("{i}.wav"));
            write_wav(&f, if i % 2 == 0 { 44_100 } else { 96_000 }, 20.0);
            f
        })
        .collect();
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));

    let mut app = App::new(Some(music.clone()));
    render(&mut app);
    for g in 0..20 {
        app.navigate_to(&music.join(format!("genre {g}")));
        render(&mut app);
        for a in (0..25).step_by(5) {
            app.navigate_to(&music.join(format!("genre {g}/artist {a}/album 0")));
            render(&mut app);
        }
    }
    for action in [Action::Theme, Action::OpenFolder, Action::Help, Action::About, Action::Equalizer] {
        app.run(action);
        render(&mut app);
        key(&mut app, KeyCode::Esc);
    }
    for s in &songs {
        let mut src = crate::audio::test_open(s, 48_000).unwrap();
        while src.next().is_some() {}
    }
    let _ = crate::eq::preset_curve(3);

    println!("URCHIN_PEAK_MB={:.1}", peak_rss_mb());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn memory_stays_within_budget() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["tests::memory_probe", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("URCHIN_MEMORY_PROBE", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let peak: f64 = text
        .lines()
        .find_map(|l| l.split("URCHIN_PEAK_MB=").nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("memory probe didn't report:\n{text}\n{}", String::from_utf8_lossy(&out.stderr)));
    println!("peak memory: {peak:.1} MB (budget {MEMORY_BUDGET_MB} MB)");
    assert!(peak < MEMORY_BUDGET_MB, "peak memory {peak:.1} MB is over the {MEMORY_BUDGET_MB} MB budget");
}

#[test]
fn one_pane_thirteen_seconds_into_a_song_then_back_to_two() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let album = music.join("punk/hardcore/Minor Threat/Complete Discography");
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    let mut app = App::new(Some(music));
    render(&mut app);

    app.navigate_to(&album);
    render(&mut app);
    let row = app.levels[app.focus].entries.iter().position(|e| e.path == song).unwrap();
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    click(&mut app, |t| *t == Target::Row { col, row });
    let panes = |app: &App| app.hits.iter().filter(|h| matches!(h.target, Target::Column(_))).count();

    app.status.plays = app.engine.plays_sent();
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![album.join("01 - Filler.wav"), song.clone()]);
    app.status.index = 1;
    app.status.position = 4.9;
    app.check_solo();
    render(&mut app);
    assert_eq!(panes(&app), 2);

    app.status.position = 13.0;
    app.check_solo();
    let screen = render(&mut app);
    assert!(app.solo);
    assert_eq!(panes(&app), 1, "{screen}");
    assert_eq!(app.current_dir(), album);
    assert_eq!(app.selected_entry().map(|e| e.path.clone()), Some(song));
    assert!(!screen.contains("Minor Threat ─"), "the parent pane is gone:\n{screen}");

    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('+'));
    assert!(app.solo);
    app.status.position = 30.0;
    app.check_solo();

    for code in [KeyCode::Char('j'), KeyCode::Down, KeyCode::Char('k'), KeyCode::Up, KeyCode::PageDown, KeyCode::End, KeyCode::Home] {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        render(&mut app);
        assert!(app.solo, "{code:?} left the one-pane view");
        assert_eq!(panes(&app), 1);
    }

    app.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    render(&mut app);
    assert!(!app.solo);
    assert_eq!(panes(&app), 2);

    app.solo = true;
    render(&mut app);
    click(&mut app, |t| *t == Target::Back);
    assert!(!app.solo);

    app.navigate_to(&album);
    render(&mut app);
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row: 0 });
    click(&mut app, |t| *t == Target::Row { col, row: 0 });
    app.status.position = 95.0;
    app.check_solo();
    render(&mut app);
    assert!(!app.solo, "switched to one pane straight after picking a new song");
    app.status.plays = app.engine.plays_sent();
    app.status.position = 13.2;
    app.check_solo();
    assert!(app.solo);
}

#[test]
fn one_pane_comes_back_thirteen_seconds_after_you_navigate_away_from_the_same_song() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let album = music.join("punk/hardcore/Minor Threat/Complete Discography");
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    let mut app = App::new(Some(music));
    app.settings.compact = true;
    render(&mut app);
    app.navigate_to(&album);
    render(&mut app);
    let row = app.levels[app.focus].entries.iter().position(|e| e.path == song).unwrap();
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    click(&mut app, |t| *t == Target::Row { col, row });
    app.status.plays = app.engine.plays_sent();
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![album.join("01 - Filler.wav"), song.clone()]);
    app.status.index = 1;
    app.status.position = 13.0;
    app.check_solo();
    assert!(app.solo);
    let toolbar_shown = |app: &mut App| {
        render(app);
        !app.toolbar_hidden()
    };
    assert!(!toolbar_shown(&mut app));

    app.status.position = 40.0;
    app.check_solo();
    key(&mut app, KeyCode::Left);
    app.check_solo();
    assert!(!app.solo);
    assert!(toolbar_shown(&mut app));

    app.status.position = 44.0;
    app.check_solo();
    assert!(!app.solo, "came back before 5 s");

    key(&mut app, KeyCode::Down);
    app.status.position = 48.0;
    app.check_solo();
    assert!(!app.solo, "the key press didn't restart the wait");

    app.status.position = 49.0;
    app.check_solo();
    assert!(app.solo);
    assert_eq!(app.current_dir(), album);
    assert_eq!(app.selected_entry().map(|e| e.path.clone()), Some(song.clone()));
    assert!(!toolbar_shown(&mut app));

    key(&mut app, KeyCode::Left);
    app.check_solo();
    assert!(!app.solo);
    app.status.position = 79.0;
    app.check_solo();
    assert!(app.solo);

    key(&mut app, KeyCode::Left);
    app.check_solo();
    assert!(!app.solo);
    app.status.plays += 1;
    app.status.position = 200.0;
    app.check_solo();
    assert!(!app.solo, "a restarted song isn't the same play");

    app.status.plays -= 1;
    app.status.position = 10.0;
    app.check_solo();
    key(&mut app, KeyCode::Left);
    app.check_solo();
    app.status.index = 0;
    app.status.position = 100.0;
    app.check_solo();
    assert!(!app.solo, "a different song isn't the one you left");

    app.status.index = 1;
    app.status.state = crate::audio::State::Paused;
    app.status.position = 300.0;
    app.check_solo();
    assert!(!app.solo);
}

#[test]
fn the_path_bar_arrows_climb_out_of_the_root_like_left_does() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let deep = root.join("Music/punk/hardcore");

    let mut keyed = App::new(Some(music.clone()));
    keyed.navigate_to(&deep);
    for _ in 0..40 {
        keyed.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }
    assert_eq!(keyed.current_dir(), Path::new("/"), "Left should reach the filesystem root");

    let mut app = App::new(Some(music.clone()));
    app.navigate_to(&deep);
    render(&mut app);
    let mut seen = vec![app.current_dir()];
    for _ in 0..40 {
        click(&mut app, |t| *t == Target::Back);
        render(&mut app);
        let at = app.current_dir();
        if *seen.last().unwrap() != at {
            assert!(!seen.contains(&at), "◀ went back to {} it had already left", at.display());
            seen.push(at);
        }
    }
    assert_eq!(app.current_dir(), Path::new("/"), "◀ should reach the filesystem root");
    assert!(seen.windows(2).all(|w| w[0].parent() == Some(w[1].as_path())), "{seen:?}");
    assert_eq!(seen[0], deep);

    assert!(!app.can_go_up());
    click(&mut app, |t| *t == Target::Back);
    assert_eq!(app.current_dir(), Path::new("/"));

    let mut app = App::new(Some(music.clone()));
    app.navigate_to(&deep);
    render(&mut app);
    click(&mut app, |t| *t == Target::Back);
    render(&mut app);
    assert_eq!(app.current_dir(), deep.parent().unwrap());
    click(&mut app, |t| *t == Target::Forward);
    assert_eq!(app.current_dir(), deep, "▶ should come back down to where ◀ started");

    let mut keyed = App::new(Some(music.clone()));
    keyed.navigate_to(&deep);
    render(&mut keyed);
    while keyed.key_focus != Some(Target::Back) {
        keyed.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    keyed.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(keyed.current_dir(), deep.parent().unwrap());
}

#[test]
fn the_crumbs_leading_ellipsis_jumps_to_the_folder_it_hides() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let deep = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");

    let mut app = App::new(Some(music.clone()));
    app.navigate_to(&deep);
    let screen = render_at(&mut app, 60, 30);
    assert!(screen.contains("… ›"), "the path should be truncated at this width\n{screen}");

    let (r, target) = app
        .hits
        .iter()
        .find(|h| h.rect.x == 6 && matches!(h.target, Target::Crumb(_)))
        .map(|h| (h.rect, h.target.clone()))
        .expect("… should be clickable");
    let Target::Crumb(hidden) = target else { unreachable!() };
    assert!(r.width >= 3, "… should be wide enough to click: {r:?}");

    let was = app.current_dir();
    assert!(was.starts_with(&hidden) && hidden != was, "{} is not above {}", hidden.display(), was.display());
    click_at(&mut app, r.x, r.y, MouseButton::Left);
    assert_eq!(app.current_dir(), hidden);

    let screen = render_at(&mut app, 60, 30);
    assert!(screen.contains(&*crate::app::short_name_cow(&hidden)), "{screen}");

    let mut app = App::new(Some(music));
    app.navigate_to(&deep);
    let screen = render_at(&mut app, 200, 30);
    assert!(!screen.contains('…'), "{screen}");
}

#[test]
fn standard_and_flipped_views() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let y_of = |app: &App, t: Target| app.hits.iter().find(|h| h.target == t).map(|h| h.rect.y).unwrap();

    assert!(!app.settings.flipped);
    let screen = render(&mut app);
    println!("{screen}");
    assert!(y_of(&app, Target::Seek) < 5, "seek bar near the top");
    assert_eq!(y_of(&app, Target::Back), 28, "path bar on the second-to-last row");
    let last_row: String = screen.lines().last().unwrap().to_string();
    assert!(last_row.trim().is_empty(), "the bottom row should be empty: {last_row:?}");
    assert!(y_of(&app, Target::Column(0)) > y_of(&app, Target::Seek));

    click(&mut app, |t| *t == Target::Menu(2));
    render(&mut app);
    let i = app.menu.as_ref().unwrap().items.iter().position(|it| it.action == Some(Action::SetFlipped(true))).unwrap();
    click(&mut app, |t| *t == Target::MenuItem(i));
    assert!(app.settings.flipped);
    let screen = render(&mut app);
    println!("{screen}");
    assert_eq!(y_of(&app, Target::Back), 1);
    assert!(y_of(&app, Target::Seek) > 25);

    assert!(crate::settings::Settings::load().flipped);
    app.run(Action::SetFlipped(false));
    assert!(!crate::settings::Settings::load().flipped);
}

#[test]
fn presets_are_alphabetical_and_never_clip() {
    use crate::eq::{preset_curve, preset_name, preset_order, Processor, PRESET_COUNT};
    let names: Vec<&str> = preset_order().into_iter().map(preset_name).collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(names, sorted);
    assert_eq!(names.len(), PRESET_COUNT);
    assert_eq!(names[0], "Alternative");
    assert_eq!(*names.last().unwrap(), "Vocal");
    for n in ["R&B", "Hip hop", "Metal", "Drum & bass", "Dance", "Flat"] {
        assert!(names.contains(&n), "{n} missing");
    }
    let rnb = preset_curve((0..PRESET_COUNT).find(|&i| preset_name(i) == "R&B").unwrap());
    assert_eq!(rnb.gains, [6.0, 6.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0, 3.0, 4.0]);

    let rate = 48_000u32;
    for id in 0..PRESET_COUNT {
        let mut p = Processor::new(rate);
        p.set_curve(&preset_curve(id));
        let mut peak = 0.0f32;
        let n = rate as usize * 4;
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let k = (1000f32).ln() / 4.0;
            let phase = std::f32::consts::TAU * 20.0 * ((k * t).exp() - 1.0) / k;
            let (l, _) = p.process(phase.sin(), phase.sin());
            if i > rate as usize / 10 {
                peak = peak.max(l.abs());
            }
        }
        assert!(peak <= 1.02, "{} peaks at {peak:.3} (clips)", preset_name(id));
    }
    let metal = (0..PRESET_COUNT).find(|&i| preset_name(i) == "Metal").unwrap();
    let mut p = Processor::new(rate);
    p.set_curve(&preset_curve(metal));
    assert!(p.peak_db() > 14.0, "combined peak {:.1} dB", p.peak_db());
}

fn noise_at(t: f32) -> f32 {
    let mut z = ((t as f64 * 48_000.0).round() as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32 - 0.5
}

fn synth_music(t: f32) -> f32 {
    use std::f32::consts::TAU;
    let beat = t % 0.5;
    let kick = 0.55 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 70.0 / 18.0 * (1.0 - (-18.0 * beat).exp()))).sin();
    let bass: f32 = (1..5).map(|k| 0.14 / k as f32 * (TAU * 55.0 * k as f32 * t).sin()).sum();
    let chord: f32 = [220.0f32, 277.2, 329.6]
        .iter()
        .flat_map(|&f| (1..4).map(move |k| 0.05 / k as f32 * (TAU * f * k as f32 * t).sin()))
        .sum();
    let vib = 880.0 + 6.0 * (TAU * 5.0 * t).sin();
    let lead = 0.05 * (TAU * vib * t).sin() + 0.03 * (TAU * 2.0 * vib * t).sin() + 0.02 * (TAU * 3.0 * vib * t).sin();
    let hat_t = t % 0.25;
    let hat = 0.18 * (-hat_t * 40.0).exp() * (noise_at(t) - noise_at(t - 1.0 / 48_000.0));
    (kick + bass + chord + lead + hat).clamp(-1.0, 1.0)
}

fn run_analyzer(
    an: &mut crate::spectrum::Analyzer,
    tap: &crate::spectrum::Tap,
    columns: usize,
    segments: u16,
    start: f32,
    secs: f32,
    signal: impl Fn(f32) -> f32,
    clock: &mut std::time::Instant,
) -> Vec<u16> {
    let rate = 48_000.0;
    let mut lit = Vec::new();
    let frames = (secs * 30.0) as usize;
    for f in 0..frames {
        let mut w = tap.writer();
        for i in 0..1600 {
            let t = start + (f * 1600 + i) as f32 / rate;
            w.push(signal(t));
        }
        drop(w);
        *clock += std::time::Duration::from_millis(33);
        lit = an.update(tap, columns, segments, *clock).to_vec();
    }
    lit
}

#[test]
fn analyzer_finds_bass_mids_and_treble_where_they_belong() {
    use crate::spectrum::{Analyzer, Tap};
    use std::f32::consts::TAU;
    let tap = Tap::new();
    tap.set_rate(48_000);
    let mut clock = std::time::Instant::now();
    let tone = |f: f32| move |t: f32| 0.3 * (TAU * f * t).sin();
    let tallest = |lit: &[u16]| lit.iter().enumerate().max_by_key(|(_, &v)| v).map(|(i, _)| i).unwrap();

    for (freq, expect_col) in [(60.0f32, 0.15f32), (1000.0, 0.55), (8000.0, 0.88)] {
        let mut an = Analyzer::new();
        let lit = run_analyzer(&mut an, &tap, 32, 10, 0.0, 0.6, tone(freq), &mut clock);
        let col = tallest(&lit) as f32 / 32.0;
        assert!((col - expect_col).abs() < 0.12, "{freq} Hz peaked at column {col:.2} of the width, expected ~{expect_col}: {lit:?}");
        let peak = lit[tallest(&lit)];
        assert!(peak >= 6, "{freq} Hz only lit {peak} of 10 segments");
        let far = if col < 0.5 { lit[31] } else { lit[0] };
        assert_eq!(far, 0, "{freq} Hz leaked to the far end: {lit:?}");
    }
}

#[test]
fn analyzer_rises_fast_falls_smoothly_and_goes_dark_in_silence() {
    use crate::spectrum::{Analyzer, Tap};
    use std::f32::consts::TAU;
    let tap = Tap::new();
    let mut an = Analyzer::new();
    let mut clock = std::time::Instant::now();
    let tone = |t: f32| 0.5 * (TAU * 250.0 * t).sin();
    let lit = run_analyzer(&mut an, &tap, 32, 10, 0.0, 0.1, tone, &mut clock);
    let col = (0..32).max_by_key(|&i| lit[i]).unwrap();
    assert!(lit[col] >= 8, "slow attack: {} segments after 0.1 s", lit[col]);
    let mut heights = vec![lit[col]];
    for k in 0..30 {
        let l = run_analyzer(&mut an, &tap, 32, 10, 1.0 + k as f32 / 30.0, 1.0 / 30.0, |_| 0.0, &mut clock);
        heights.push(l[col]);
    }
    let top = (0..heights.len()).max_by_key(|&i| (heights[i], std::cmp::Reverse(i))).unwrap();
    let falling = &heights[top..];
    assert!(falling.windows(2).all(|w| w[1] <= w[0]), "bars bounced while falling: {heights:?}");
    let steps = falling.windows(2).filter(|w| w[1] < w[0]).count();
    assert!(steps >= 4, "fell in {steps} jumps, should drop smoothly: {heights:?}");
    assert_eq!(*heights.last().unwrap(), 0, "still lit after 1 s of silence: {heights:?}");
    assert!(!an.is_active());
}

#[test]
fn analyzer_holds_steady_on_a_steady_sound() {
    use crate::spectrum::{Analyzer, Tap};
    let tap = Tap::new();
    let mut an = Analyzer::new();
    let mut clock = std::time::Instant::now();
    let steady = |t: f32| {
        0.2 * (std::f32::consts::TAU * 440.0 * t).sin() + 0.1 * (std::f32::consts::TAU * 3000.0 * t).sin() + 0.02 * noise_at(t)
    };
    run_analyzer(&mut an, &tap, 32, 10, 0.0, 1.0, steady, &mut clock);
    let mut history: Vec<Vec<u16>> = vec![an.update(&tap, 32, 10, clock).to_vec()];
    for k in 0..120 {
        history.push(run_analyzer(&mut an, &tap, 32, 10, 1.0 + k as f32 / 30.0, 1.0 / 30.0, steady, &mut clock));
    }
    for col in 0..32 {
        let heights: Vec<u16> = history.iter().map(|f| f[col]).collect();
        let changes: Vec<usize> = (1..heights.len()).filter(|&i| heights[i] != heights[i - 1]).collect();
        for pair in changes.windows(2) {
            assert!(pair[1] - pair[0] >= 10, "bar {col} flickered: {heights:?}");
        }
        assert!(changes.len() <= 2, "bar {col} kept moving on a steady sound: {heights:?}");
    }
    let peak_cols: Vec<usize> = (0..32).filter(|&c| history[0][c] >= 5).collect();
    assert!(!peak_cols.is_empty());
    for c in peak_cols {
        assert!(history.iter().all(|f| f[c] == history[0][c]), "tone bar {c} moved");
    }
}

#[test]
fn analyzer_shows_music_across_the_range() {
    use crate::spectrum::{Analyzer, Tap};
    let tap = Tap::new();
    let mut an = Analyzer::new();
    let mut clock = std::time::Instant::now();
    let lit = run_analyzer(&mut an, &tap, 32, 10, 0.0, 1.3, synth_music, &mut clock);
    println!("{lit:?}");
    assert!(lit[..8].iter().any(|&v| v >= 5), "bass: {lit:?}");
    assert!(lit[12..22].iter().any(|&v| v >= 3), "mids: {lit:?}");
    assert!(lit[24..].iter().any(|&v| v >= 1), "treble: {lit:?}");
    assert!(lit.iter().filter(|&&v| v == 10).count() < 8, "too many bars maxed: {lit:?}");
    assert!(an.levels().iter().all(|&l| (0.0..=1.0).contains(&l)));
}

#[test]
fn analyzer_sits_next_to_the_playback_panel_in_both_views() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    assert!(app.settings.analyzer, "on by default");
    let seek_y = |app: &App| app.hits.iter().find(|h| h.target == Target::Seek).unwrap().rect.y;
    let pane_y = |app: &App| app.hits.iter().find(|h| matches!(h.target, Target::Column(_))).unwrap().rect.y;
    let rows_with_segments = |screen: &str| -> Vec<usize> {
        screen.lines().enumerate().filter(|(_, l)| l.contains("▆▆ ▆▆")).map(|(i, _)| i).collect()
    };

    let screen = render(&mut app);
    let rows = rows_with_segments(&screen);
    assert_eq!(rows.len(), crate::ui::analyzer_rows(30, false) as usize - 1, "{screen}");
    assert!(rows[0] > seek_y(&app) as usize && *rows.last().unwrap() < pane_y(&app) as usize, "{screen}");

    app.run(Action::SetFlipped(true));
    let screen = render(&mut app);
    let rows = rows_with_segments(&screen);
    assert!(rows[0] > pane_y(&app) as usize && *rows.last().unwrap() < seek_y(&app) as usize, "{screen}");

    click(&mut app, |t| *t == Target::Menu(2));
    render(&mut app);
    let i = app.menu.as_ref().unwrap().items.iter().position(|it| it.action == Some(Action::ToggleVisual(Visual::Spectrum))).unwrap();
    assert_eq!(app.menu.as_ref().unwrap().items[i].checked, Some(true));
    click(&mut app, |t| *t == Target::MenuItem(i));
    let screen = render(&mut app);
    assert!(rows_with_segments(&screen).is_empty(), "{screen}");
    assert!(!crate::settings::Settings::load().analyzer);

    let labels: Vec<String> = app.menu_items(2).iter().map(|i| i.label.clone()).collect();
    assert!(!labels.iter().any(|l| l.contains("Color") || l.contains("Color")), "{labels:?}");
    app.run(Action::ToggleVisual(Visual::Spectrum));
    assert!(app.settings.analyzer);
    app.run(Action::SetFlipped(false));
}

#[test]
fn analyzer_resizes_and_stays_centerd() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    for (w, h) in [(80u16, 24u16), (110, 30), (160, 48), (220, 60)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let rows: Vec<u16> = (0..h).filter(|&y| (0..w).any(|x| buf[(x, y)].symbol() == "▆")).collect();
        assert_eq!(rows.len() as u16, crate::ui::analyzer_rows(h, false) - 1, "{w}x{h}");
        let y = rows[0];
        let xs: Vec<u16> = (0..w).filter(|&x| buf[(x, y)].symbol() == "▆").collect();
        let (left, right) = (xs[0], w - 1 - xs.last().unwrap());
        assert!(left.abs_diff(right) <= 1, "{w}x{h}: margins {left} and {right}");
        let gaps: Vec<u16> = xs.windows(2).map(|p| p[1] - p[0]).filter(|&d| d > 1).collect();
        assert!(gaps.iter().all(|&g| g == 2), "{w}x{h}: uneven gaps {gaps:?}");
    }
    assert_eq!(crate::ui::analyzer_rows(16, false), 0);
    assert_eq!(crate::ui::analyzer_rows(16, true), 0);
}

#[test]
#[ignore]
fn analyzer_preview() {
    let Ok(path) = std::env::var("URCHIN_PREVIEW") else { return };
    let _g = serial();
    let root = library();
    for (flipped, name, theme) in [
        (false, "default_theme", None),
        (false, "dracula", Some("Dracula")),
        (false, "gruvbox_light", Some("Gruvbox Light")),
        (true, "nord_flipped", Some("Nord")),
        (false, "solarized", Some("Solarized Dark Higher Contrast")),
    ] {
        let mut app = App::new(Some(root.join("Music")));
        if let Some(t) = theme {
            app.theme = crate::theme::index_of(t).unwrap_or_else(|| panic!("no theme {t}"));
        }
        app.settings.flipped = flipped;
        app.status.state = crate::audio::State::Playing;
        let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
        app.status.queue = std::sync::Arc::new(vec![album.join("02 - I Don't Wanna Hear It.wav")]);
        app.status.meta.title = "I Don't Wanna Hear It".into();
        app.status.meta.artist = "Minor Threat".into();
        app.status.position = 31.0;
        app.status.duration = 51.0;
        app.navigate_to(&album);
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let tap = app.tap.clone();
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let cols = 39;
        let rows = crate::ui::analyzer_rows(40, flipped) - 1;
        run_analyzer(app.visuals.spectrum(), &tap, cols, rows, 0.0, 1.3, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                let hex = |col: ratatui::style::Color| match col {
                    ratatui::style::Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
                    _ => "000000".into(),
                };
                out.push_str(&format!("{}\t{}\t{}\n", c.symbol().replace('\t', " "), hex(c.fg), hex(c.bg)));
            }
            out.push_str("--\n");
        }
        fs::write(format!("{path}/{name}.cells"), out).unwrap();
    }
}

#[test]
fn analyzer_works_with_every_theme() {
    use crate::theme::{contrast_of, luminance_of, Shade, THEMES};
    for t in THEMES {
        let a = t.palette().analyzer;
        assert!(luminance_of(a.glass) <= 0.03, "{}: display window not dark", t.name);
        let (main, high, top) = (a.lit(Shade::Main), a.lit(Shade::High), a.lit(Shade::Top));
        assert!(contrast_of(main, a.glass) >= 3.5, "{}: segments too faint", t.name);
        assert!(luminance_of(main) > luminance_of(high) && luminance_of(high) > luminance_of(top), "{}: shades not getting darker", t.name);
        assert!(contrast_of(main, high) >= 1.3 && contrast_of(high, top) >= 1.3, "{}: shades too alike", t.name);
        assert!(contrast_of(top, a.glass) >= 1.85, "{}: darkest segment barely shows", t.name);
        for shade in [Shade::Main, Shade::High, Shade::Top] {
            assert!(contrast_of(a.lit(shade), a.unlit) >= 1.7, "{}: {shade:?} lit and unlit look alike ({:.2})", t.name, contrast_of(a.lit(shade), a.unlit));
        }
        assert!(contrast_of(a.unlit, a.glass) < 1.8, "{}: unlit segments too bright", t.name);
    }
    assert!(THEMES.len() > 900);
}

#[test]
fn a_preset_picked_for_one_song_applies_to_its_folder_only() {
    use crate::eq::{preset_curve, preset_name, Source, Store, PRESET_COUNT};
    let _g = serial();
    let root = library();
    let rnb = root.join("Music/r&b");
    let ne = rnb.join("New Edition");
    let live = ne.join("Live");
    let other = rnb.join("Bell Biv DeVoe");
    for d in [&ne, &live, &other] {
        fs::create_dir_all(d).unwrap();
    }
    let song = |dir: &Path, name: &str| {
        let p = dir.join(format!("{name}.wav"));
        write_wav(&p, 44_100, 0.2);
        p
    };
    let (a, b, c, f) = (song(&ne, "a"), song(&ne, "b"), song(&ne, "c"), song(&ne, "f"));
    let d = song(&live, "d");
    let e = song(&other, "e");
    let id = |name: &str| (0..PRESET_COUNT).find(|&i| preset_name(i) == name).unwrap();
    let (rnb_id, hiphop_id, flat_id) = (id("R&B"), id("Hip hop"), id("Flat"));

    let mut app = App::new(Some(root.join("Music")));
    let play = |app: &mut App, s: &Path| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![s.to_path_buf()]);
        app.status.index = 0;
    };
    let source = |app: &App, s: &Path| app.eq_store.lock().unwrap().source(s);
    let curve = |app: &App, s: &Path| app.eq_store.lock().unwrap().get(s);

    play(&mut app, &c);
    app.run(Action::Equalizer);
    key(&mut app, KeyCode::Up);
    let c_own = curve(&app, &c);
    assert_eq!(source(&app, &c), Source::Own);

    play(&mut app, &a);
    app.run(Action::EqPreset(rnb_id));
    assert_eq!(curve(&app, &a), preset_curve(rnb_id));
    for s in [&b, &f] {
        assert_eq!(source(&app, s), Source::Folder(rnb_id as u8));
        assert_eq!(curve(&app, s), preset_curve(rnb_id));
    }
    assert_eq!(curve(&app, &c), c_own);
    assert_eq!(source(&app, &d), Source::None);
    assert_eq!(source(&app, &e), Source::None);
    let toast = app.toast.as_ref().unwrap().0.clone();
    assert_eq!(toast, "R&B for this song and 2 more in New Edition (1 keeps their own EQ)");

    play(&mut app, &b);
    assert!(render(&mut app).contains("This folder's preset: R&B"));
    key(&mut app, KeyCode::Up);
    assert_eq!(source(&app, &b), Source::Own);
    assert_eq!(curve(&app, &b).label(), "R&B (customized)");
    assert_eq!(source(&app, &f), Source::Folder(rnb_id as u8));

    play(&mut app, &f);
    app.run(Action::EqPreset(hiphop_id));
    assert_eq!(curve(&app, &a), preset_curve(rnb_id));
    assert_eq!(curve(&app, &b).label(), "R&B (customized)");
    assert_eq!(curve(&app, &c), c_own);
    assert_eq!(app.eq_store.lock().unwrap().folder_preset_for(&a), Some(hiphop_id as u8));

    play(&mut app, &a);
    click(&mut app, |t| *t == Target::EqReset);
    assert_eq!(source(&app, &a), Source::Folder(hiphop_id as u8));
    assert!(app.toast.as_ref().unwrap().0.contains("follows its folder's preset again: Hip hop"));

    key(&mut app, KeyCode::Esc);
    let store = Store::load();
    assert_eq!(store.source(&a), Source::Folder(hiphop_id as u8));
    assert_eq!(store.get(&c), c_own);
    assert_eq!(store.source(&d), Source::None);

    play(&mut app, &f);
    app.run(Action::EqPreset(flat_id));
    assert_eq!(source(&app, &a), Source::None);
    assert!(curve(&app, &a).is_flat());
    assert_eq!(curve(&app, &c), c_own);
}

fn analyzer_label_row(buf: &ratatui::buffer::Buffer) -> (u16, String) {
    let (w, h) = (buf.area.width, buf.area.height);
    let last_seg = (0..h).filter(|&y| (0..w).any(|x| buf[(x, y)].symbol() == "▆")).max().unwrap();
    let y = last_seg + 1;
    (y, (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect())
}

#[test]
fn analyzer_has_frequency_labels_under_the_right_bars() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let labels = ["20 Hz", "60 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"];
    for flipped in [false, true] {
        app.settings.flipped = flipped;
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let (y, row) = analyzer_label_row(&buf);
        let mut at = 0;
        for l in labels {
            let pos = row[at..].find(l).unwrap_or_else(|| panic!("{l} missing or out of order: {row:?}")) + at;
            at = pos + l.len();
        }
        let bars: Vec<u16> = (0..120).filter(|&x| buf[(x, y - 1)].symbol() == "▆").collect();
        assert_eq!(row.find("20 Hz").unwrap() as u16, bars[0], "{row:?}");
        assert_eq!((row.find("16 kHz").unwrap() + 6) as u16, bars.last().unwrap() + 1, "{row:?}");
        let n = bars.len() / 2;
        for (freq, l) in [(1000.0f32, "1 kHz"), (250.0, "250 Hz"), (4000.0, "4 kHz")] {
            let col = crate::spectrum::column_of(freq, n, 48_000).unwrap();
            let bar_x = bars[col * 2] as i32;
            let mid = row.find(l).unwrap() as i32 + l.len() as i32 / 2;
            assert!((mid - bar_x).abs() <= 2, "{l} centerd at {mid}, its bar is at {bar_x}: {row:?}");
        }
    }
}

fn labels_on(row: &str) -> Vec<(String, usize)> {
    let mut words = Vec::new();
    let mut x = 0;
    for word in row.split(' ') {
        if !word.is_empty() {
            words.push((x, word));
        }
        x += word.len() + 1;
    }
    words
        .chunks(2)
        .map(|pair| {
            let [(x, number), (unit_x, unit)] = pair else { panic!("a label lost its unit: {row:?}") };
            assert_eq!(*unit_x, x + number.len() + 1, "label split apart: {row:?}");
            (format!("{number} {unit}"), *x)
        })
        .collect()
}

#[test]
fn analyzer_labels_sit_under_their_bars_at_every_width() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let all = ["20 Hz", "60 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"];
    let freqs = [20.0f32, 60.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
    for w in 30u16..=200 {
        let mut term = Terminal::new(TestBackend::new(w, 40)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let (y, row) = analyzer_label_row(&buf);
        let found = labels_on(&row);
        let mut rest = all.iter();
        for (label, _) in &found {
            assert!(rest.any(|l| l == label), "{w} columns: {label:?} out of place: {row:?}");
        }
        for must in ["20 Hz", "1 kHz", "16 kHz"] {
            assert!(found.iter().any(|(l, _)| l == must), "{w} columns: {must} missing: {row:?}");
        }
        if w >= 67 {
            assert_eq!(found.len(), 10, "{w} columns: only {} labels: {row:?}", found.len());
        }
        let cells: Vec<u16> = (0..w).filter(|&x| buf[(x, y - 1)].symbol() == "▆").collect();
        let cw = cells.windows(2).take_while(|p| p[1] == p[0] + 1).count() + 1;
        let (left, right) = (cells[0] as usize, *cells.last().unwrap() as usize + 1);
        for (label, x) in &found {
            let i = all.iter().position(|l| l == label).unwrap();
            let col = crate::spectrum::column_of(freqs[i], cells.len() / cw, 48_000).unwrap();
            let bar = cells[col * cw] as usize;
            assert!(*x < bar + cw && x + label.len() > bar, "{w} columns: {label} isn't under its bar at {bar}: {row:?}");
        }
        assert_eq!(found[0], ("20 Hz".to_string(), left), "{w} columns: {row:?}");
        let (last, x) = found.last().unwrap();
        assert_eq!((last.as_str(), x + last.len()), ("16 kHz", right), "{w} columns: {row:?}");
    }
}

#[test]
fn analyzer_bars_follow_the_theme() {
    use crate::theme::{contrast_of, Shade, THEMES};
    let _g = serial();
    let mut distinct = std::collections::HashSet::new();
    for t in THEMES {
        let p = t.palette();
        let low = p.analyzer.lit(Shade::Main);
        if contrast_of(p.accent, p.analyzer.glass) >= 4.0 {
            assert_eq!(low, p.accent, "{}: bars don't use the theme's accent", t.name);
        }
        distinct.insert(format!("{low:?}"));
    }
    assert!(distinct.len() > 300, "only {} different bar colors across {} themes", distinct.len(), THEMES.len());
    for t in THEMES {
        let p = t.palette();
        assert!(contrast_of(p.analyzer.label, p.analyzer.glass) >= 3.5, "{}: labels too faint", t.name);
    }
}

#[test]
fn analyzer_bars_get_a_darker_tip_wherever_they_reach() {
    use crate::theme::Shade;
    for lit in [1u16, 2, 3, 7, 13] {
        assert_eq!(Shade::of(lit - 1, lit), Some(Shade::Top), "{lit} lit");
        if lit >= 2 {
            assert_eq!(Shade::of(lit - 2, lit), Some(Shade::High), "{lit} lit");
        }
        assert!((0..lit.saturating_sub(2)).all(|s| Shade::of(s, lit) == Some(Shade::Main)), "{lit} lit");
        assert!((lit..13).all(|s| Shade::of(s, lit).is_none()), "{lit} lit");
    }
    assert!((0..13).all(|s| Shade::of(s, 0).is_none()));

    let _g = serial();
    let root = library();
    for theme in ["Dracula", "Gruvbox Light"] {
        let mut app = App::new(Some(root.join("Music")));
        app.theme = crate::theme::index_of(theme).unwrap();
        let colors = app.palette().analyzer;
        let (w, h) = (120u16, 40u16);
        let segs = crate::ui::analyzer_rows(h, false) - 1;
        let tap = app.tap.clone();
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        run_analyzer(app.visuals.spectrum(), &tap, 39, segs, 0.0, 1.0, |t| (synth_music(t) * 2.0).clamp(-1.0, 1.0), &mut clock);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let rows: Vec<u16> = (0..h).filter(|&y| (0..w).any(|x| buf[(x, y)].symbol() == "▆")).collect();
        let mut heights = std::collections::HashSet::new();
        for x in (0..w).filter(|&x| buf[(x, rows[0])].symbol() == "▆") {
            let column: Vec<_> = rows.iter().rev().map(|&y| buf[(x, y)].fg).collect();
            let on = column.iter().take_while(|&&c| c != colors.unlit).count() as u16;
            for (s, &fg) in column.iter().enumerate() {
                let want = match Shade::of(s as u16, on) {
                    Some(shade) => colors.lit(shade),
                    None => colors.unlit,
                };
                assert_eq!(fg, want, "{theme}: bar at x={x} with {on} lit, segment {s}");
            }
            heights.insert(on);
        }
        assert!(heights.len() >= 4, "{theme}: bar heights {heights:?}");
    }
}

#[test]
fn layout_fills_the_window_in_both_views() {
    use crate::ui::{Layout, TRANSPORT_H};
    use ratatui::layout::Rect;
    for flipped in [false, true] {
        for analyzer in [false, true] {
            for h in 11u16..=80 {
                let area = Rect { x: 0, y: 0, width: 100, height: h };
                let l = Layout::new(area, flipped, analyzer);
                let parts = if flipped {
                    vec![l.toolbar, l.folders, l.analyzer, l.transport]
                } else {
                    vec![l.transport, l.analyzer, l.folders, l.toolbar]
                };
                let mut y = 1;
                for r in &parts {
                    assert_eq!(r.y, y, "{h} rows, flipped {flipped}: {parts:?}");
                    y += r.height;
                }
                let end = if flipped { h } else { h - 1 };
                assert_eq!(y, end, "{h} rows, flipped {flipped}: {parts:?}");
                assert_eq!(l.transport.height, TRANSPORT_H);
                assert!(l.folders.height >= 3, "{h} rows: folders squeezed to {}", l.folders.height);
                if !analyzer {
                    assert_eq!(l.analyzer.height, 0);
                }
                assert_eq!(l.content.height, h - 1 - TRANSPORT_H);
                assert!(l.content.y >= 1 && l.content.y + l.content.height <= h);
                assert!(l.content.y > l.transport.y || l.content.y + l.content.height <= l.transport.y);
                assert_eq!(l.free.height, l.content.height - l.analyzer.height);
            }
        }
    }
}

fn quarter_second_songs(name: &str, n: usize) -> Vec<PathBuf> {
    let dir = std::env::temp_dir().join(format!("urchin_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    (0..n)
        .map(|i| {
            let song = dir.join(format!("{i}.wav"));
            write_wav(&song, 48_000, 0.25);
            song
        })
        .collect()
}

#[test]
fn repeat_plays_the_song_again_and_repeat_all_goes_round() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("repeat", 3);

    let mut p = TestPlayer::new(songs.clone(), 1, Repeat::Off);
    assert_eq!(p.listen(5.0), [1, 2]);
    assert!(!p.playing());

    let mut p = TestPlayer::new(songs.clone(), 1, Repeat::One);
    assert_eq!(p.listen(1.125), [1, 1, 1, 1, 1]);
    assert!(p.playing());

    let mut p = TestPlayer::new(songs.clone(), 1, Repeat::All);
    assert_eq!(p.listen(1.125), [1, 2, 0, 1, 2]);

    let mut p = TestPlayer::new(songs.clone(), 0, Repeat::One);
    p.next();
    assert_eq!(p.listen(0.625), [1, 1, 1]);
    let mut p = TestPlayer::new(songs.clone(), 2, Repeat::All);
    p.next();
    assert_eq!(p.listen(0.1), [0]);
    p.previous();
    assert_eq!(p.listen(0.1), [2]);
    let mut p = TestPlayer::new(songs, 0, Repeat::Off);
    p.previous();
    assert_eq!(p.listen(0.1), [0]);
}

#[test]
fn play_next_goes_behind_the_song_playing_in_the_order_added_and_add_to_queue_goes_last() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_add", 6);
    let mut p = TestPlayer::new(songs[..3].to_vec(), 0, Repeat::Off);
    assert_eq!(p.listen(0.05), [0]);
    p.enqueue(vec![songs[3].clone()], true);
    p.enqueue(vec![songs[4].clone()], true);
    p.enqueue(vec![songs[5].clone()], false);
    assert_eq!(p.queue(), [&songs[0], &songs[3], &songs[4], &songs[1], &songs[2], &songs[5]].map(|s| s.clone()));
    assert_eq!(p.listen(5.0), [0, 1, 2, 3, 4, 5]);

    let mut p = TestPlayer::new(songs[..2].to_vec(), 0, Repeat::Off);
    p.listen(0.05);
    p.enqueue(vec![songs[2].clone(), songs[3].clone()], true);
    assert_eq!(p.listen(0.3), [0, 1]);
    p.enqueue(vec![songs[4].clone()], true);
    assert_eq!(p.queue(), [&songs[0], &songs[2], &songs[3], &songs[4], &songs[1]].map(|s| s.clone()));
}

#[test]
fn adding_songs_when_the_queue_had_run_out_or_stopped_carries_on() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_end", 4);
    let mut p = TestPlayer::new(songs[..2].to_vec(), 1, Repeat::Off);
    assert_eq!(p.listen(0.2), [1]);
    p.enqueue(vec![songs[2].clone()], false);
    assert_eq!(p.listen(1.0), [1, 2]);

    let mut p = TestPlayer::new(songs[..1].to_vec(), 0, Repeat::Off);
    p.listen(1.0);
    assert!(!p.playing());
    p.enqueue(vec![songs[3].clone()], false);
    assert!(p.playing());
    assert_eq!(p.listen(1.0), [1]);
}

#[test]
fn removing_songs_from_the_queue() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_remove", 4);
    let mut p = TestPlayer::new(songs[..3].to_vec(), 0, Repeat::Off);
    assert_eq!(p.listen(0.2), [0]);
    p.remove(1);
    assert_eq!(p.queue(), [&songs[0], &songs[2]].map(|s| s.clone()));
    assert_eq!(p.listen(1.0), [0, 1]);

    let mut p = TestPlayer::new(songs[..3].to_vec(), 1, Repeat::Off);
    p.listen(0.05);
    p.remove(1);
    assert_eq!(p.queue(), [&songs[0], &songs[2]].map(|s| s.clone()));
    assert_eq!(p.listen(0.1), [1]);

    let mut p = TestPlayer::new(songs[..3].to_vec(), 2, Repeat::Off);
    p.listen(0.05);
    p.remove(2);
    assert!(!p.playing());
    let mut p = TestPlayer::new(songs[..3].to_vec(), 2, Repeat::All);
    p.listen(0.05);
    p.remove(2);
    assert!(p.playing());
    assert_eq!(p.listen(0.1), [0]);

    let mut p = TestPlayer::new(songs[..3].to_vec(), 2, Repeat::Off);
    p.listen(0.1);
    p.remove(0);
    assert_eq!(p.status().index, 1);
    assert!(p.playing());
    let mut p = TestPlayer::new(songs[..4].to_vec(), 0, Repeat::Off);
    p.listen(0.1);
    p.remove(3);
    assert_eq!(p.listen(2.0), [0, 1, 2]);
}

#[test]
fn moving_songs_in_the_queue_and_jumping_to_one() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_move", 3);
    let mut p = TestPlayer::new(songs.clone(), 0, Repeat::Off);
    assert_eq!(p.listen(0.2), [0]);
    p.move_song(2, 1);
    assert_eq!(p.queue(), [&songs[0], &songs[2], &songs[1]].map(|s| s.clone()));
    assert_eq!(p.listen(1.0), [0, 1, 2]);

    let mut p = TestPlayer::new(songs.clone(), 1, Repeat::Off);
    p.listen(0.1);
    let before = p.position().unwrap();
    p.move_song(1, 0);
    assert_eq!(p.queue(), [&songs[1], &songs[0], &songs[2]].map(|s| s.clone()));
    assert_eq!(p.status().index, 0);
    assert!(p.position().unwrap() >= before);
    assert_eq!(p.listen(1.0), [0, 1, 2]);

    let mut p = TestPlayer::new(songs, 0, Repeat::Off);
    p.listen(0.05);
    p.jump(2);
    assert_eq!(p.listen(0.1), [2]);
}

#[test]
fn clearing_the_queue_keeps_only_the_song_playing() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_clear", 5);

    let mut p = TestPlayer::new(songs.clone(), 1, Repeat::Off);
    assert_eq!(p.listen(0.2), [1]);
    p.clear_queue();
    assert_eq!(p.queue(), [songs[1].clone()]);
    assert_eq!(p.status().index, 0);
    assert_eq!(p.listen(1.0), [0]);
    assert!(!p.playing());

    let mut p = TestPlayer::new(songs.clone(), 2, Repeat::Off);
    p.listen(0.05);
    p.toggle_pause();
    p.clear_queue();
    assert_eq!(p.queue(), [songs[2].clone()]);
    assert!(p.playing());

    p.toggle_pause();
    p.enqueue(vec![songs[4].clone()], false);
    assert_eq!(p.listen(1.0), [0, 1]);

    let mut p = TestPlayer::new(songs, 4, Repeat::Off);
    p.listen(1.0);
    assert!(!p.playing());
    p.clear_queue();
    assert!(p.queue().is_empty());
}

#[test]
fn shuffling_the_queue_mixes_only_the_songs_still_to_come() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("queue_shuffle", 8);

    let mut p = TestPlayer::new(songs.clone(), 2, Repeat::Off);
    assert_eq!(p.listen(0.2), [2]);
    p.shuffle_upcoming(12345);
    let q = p.queue();
    assert_eq!(q[..3], songs[..3], "the songs played and the one playing stay put");
    assert_ne!(q[3..], songs[3..], "the rest are mixed");
    let mut rest = q[3..].to_vec();
    rest.sort();
    assert_eq!(rest, songs[3..], "and nothing is lost or doubled");
    assert_eq!(p.status().index, 2);
    assert_eq!(p.listen(3.0), [2, 3, 4, 5, 6, 7]);

    let mut again = TestPlayer::new(songs.clone(), 2, Repeat::Off);
    again.shuffle_upcoming(12345);
    assert_eq!(again.queue(), q);

    let mut p = TestPlayer::new(songs[..3].to_vec(), 1, Repeat::Off);
    p.listen(0.05);
    p.shuffle_upcoming(1);
    assert_eq!(p.queue(), songs[..3]);

    let mut p = TestPlayer::new(songs[..6].to_vec(), 5, Repeat::Off);
    p.listen(1.0);
    assert!(!p.playing());
    p.shuffle_upcoming(99);
    let mut all = p.queue();
    assert_ne!(all, songs[..6]);
    all.sort();
    assert_eq!(all, songs[..6]);

    let mut p = TestPlayer::new(songs[..3].to_vec(), 0, Repeat::Off);
    p.listen(0.05);
    p.enqueue(vec![songs[3].clone(), songs[4].clone()], true);
    p.shuffle_upcoming(7);
    p.enqueue(vec![songs[5].clone()], true);
    assert_eq!(p.queue()[1], songs[5], "a new play next song goes right after the one playing");
}

#[test]
fn changing_repeat_in_a_songs_last_moments_still_counts() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("repeat_late", 3);
    let mut p = TestPlayer::new(songs.clone(), 0, Repeat::Off);
    assert_eq!(p.listen(0.2), [0]);
    p.set_repeat(Repeat::One);
    assert_eq!(p.listen(0.425), [0, 0, 0]);

    let mut p = TestPlayer::new(songs.clone(), 0, Repeat::One);
    assert_eq!(p.listen(0.2), [0]);
    p.set_repeat(Repeat::Off);
    assert_eq!(p.listen(0.425), [0, 1, 2]);

    let mut p = TestPlayer::new(songs, 2, Repeat::Off);
    assert_eq!(p.listen(0.2), [2]);
    p.set_repeat(Repeat::All);
    assert_eq!(p.listen(0.425), [2, 0, 1]);
}

#[test]
fn repeat_never_gets_stuck_on_songs_that_cant_play() {
    use crate::audio::{Repeat, TestPlayer};
    let songs = quarter_second_songs("repeat_broken", 1);
    let dir = songs[0].parent().unwrap();
    let empty = dir.join("empty.wav");
    write_wav(&empty, 48_000, 0.0);
    let broken = dir.join("broken.wav");
    fs::write(&broken, "not audio").unwrap();

    let mut p = TestPlayer::new(vec![empty.clone(), songs[0].clone()], 0, Repeat::One);
    assert_eq!(p.listen(0.625), [1, 1, 1]);
    for repeat in [Repeat::One, Repeat::All] {
        let mut p = TestPlayer::new(vec![empty.clone(), broken.clone()], 0, repeat);
        assert_eq!(p.listen(1.0), [] as [usize; 0]);
        assert!(!p.playing(), "{repeat:?}");
    }
}

#[test]
fn repeat_and_repeat_all_are_in_the_playback_menu_and_remembered() {
    use crate::audio::Repeat;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let repeat_items = |app: &App| -> Vec<(String, Option<bool>)> {
        app.menu_items(1).into_iter().filter(|i| i.label.starts_with("Repeat")).map(|i| (i.label, i.checked)).collect()
    };
    let shown = |checked: [bool; 2]| vec![("Repeat".to_string(), Some(checked[0])), ("Repeat All".to_string(), Some(checked[1]))];
    assert_eq!(repeat_items(&app), shown([false, false]));
    assert_eq!(app.settings.repeat, Repeat::Off);

    render(&mut app);
    click(&mut app, |t| *t == Target::Menu(1));
    let screen = render(&mut app);
    assert!(screen.contains("  Repeat All"), "{screen}");
    let i = app.menu.as_ref().unwrap().items.iter().position(|m| m.label == "Repeat").unwrap();
    click(&mut app, |t| *t == Target::MenuItem(i));
    assert_eq!(app.settings.repeat, Repeat::One);
    assert_eq!(repeat_items(&app), shown([true, false]));
    click(&mut app, |t| *t == Target::Menu(1));
    let screen = render(&mut app);
    let lines: Vec<&str> = screen.lines().filter(|l| l.contains("Repeat")).collect();
    assert!(lines.len() == 2 && lines[0].contains("✓ Repeat") && lines[1].contains("  Repeat All"), "{screen}");

    key(&mut app, KeyCode::Esc);
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT));
    for _ in 0..6 {
        key(&mut app, KeyCode::Char('k'));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.repeat, Repeat::All);
    assert_eq!(repeat_items(&app), shown([false, true]));

    let mut app = App::new(Some(root.join("Music")));
    assert_eq!(app.settings.repeat, Repeat::All);
    app.run(Action::ToggleRepeat(Repeat::All));
    assert_eq!(repeat_items(&app), shown([false, false]));
    assert_eq!(App::new(Some(root.join("Music"))).settings.repeat, Repeat::Off);
}

#[cfg(test)]
mod audit_fix_tests {
    use crate::fsx;
    use std::fs;

    #[test]
    fn tilde_only_matches_whole_path_components() {
        let h = fsx::home().to_string_lossy().to_string();
        assert_eq!(fsx::contract_tilde(&h), "~");
        assert_eq!(fsx::contract_tilde(&format!("{h}/music")), "~/music");
        assert_eq!(fsx::contract_tilde(&format!("{h}2/music")), format!("{h}2/music"));
        assert_eq!(fsx::expand_tilde("~someone/x"), "~someone/x");
        assert_eq!(fsx::expand_tilde("~/x"), format!("{h}/x"));
    }

    #[cfg(unix)]
    #[test]
    fn collect_tracks_survives_symlink_loops() {
        let d = std::env::temp_dir().join(format!("urchin_loop_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("a")).unwrap();
        fs::write(d.join("a/one.mp3"), b"x").unwrap();
        std::os::unix::fs::symlink(&d, d.join("a/back")).unwrap();
        let t = fsx::collect_tracks(&d);
        assert_eq!(t.len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_json_is_kept_as_bad() {
        let d = std::env::temp_dir().join(format!("urchin_bad_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let f = d.join("s.json");
        fs::write(&f, b"{ not json").unwrap();
        let v: Vec<u32> = crate::settings::read_json(&f);
        assert!(v.is_empty());
        assert!(d.join("s.json.bad").exists());
        assert!(!f.exists());
        let _ = fs::remove_dir_all(&d);
    }
}

mod ring_tests {
    use crate::ring::Ring;

    fn frames(from: usize, count: usize) -> Vec<f32> {
        (from..from + count).flat_map(|i| [i as f32, -(i as f32)]).collect()
    }

    #[test]
    fn frames_come_out_in_order_across_the_wrap_and_nothing_overfills() {
        let r = Ring::new(8);
        assert_eq!(r.capacity(), 8);
        let a: Vec<f32> = (0..20).map(|i| i as f32).collect();
        assert_eq!(r.push(&a), 8, "only 8 fit");
        assert_eq!(r.len(), 8);
        {
            let mut rd = r.reader().unwrap();
            assert_eq!(rd.pop(), Some((0.0, 1.0)));
            assert_eq!(rd.pop(), Some((2.0, 3.0)));
        }
        assert_eq!(r.len(), 6);
        assert_eq!(r.push(&a[16..]), 2, "two more fit now");
        assert_eq!(r.push(&a[..2]), 0, "and then it is full");
        let mut rd = r.reader().unwrap();
        let mut got = Vec::new();
        while let Some(f) = rd.pop() {
            got.push(f);
        }
        let want: Vec<(f32, f32)> = (2..10).map(|i| (2.0 * i as f32, 2.0 * i as f32 + 1.0)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_half_frame_is_never_written() {
        let r = Ring::new(8);
        assert_eq!(r.push(&[1.0, 2.0, 3.0]), 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn clear_and_truncate_drop_what_is_waiting() {
        let r = Ring::new(16);
        r.push(&frames(0, 6));
        r.exclusive(|x| {
            assert_eq!(x.len(), 6);
            x.truncate(4);
            assert_eq!(x.len(), 4);
            x.truncate(10);
            assert_eq!(x.len(), 4);
        });
        {
            let mut rd = r.reader().unwrap();
            assert_eq!(rd.pop(), Some((0.0, -0.0)));
        }
        assert_eq!(r.len(), 3);
        r.push(&frames(100, 1));
        let mut rd = r.reader().unwrap();
        let got: Vec<f32> = std::iter::from_fn(|| rd.pop()).map(|f| f.0).collect();
        assert_eq!(got, [1.0, 2.0, 3.0, 100.0]);
        drop(rd);
        r.push(&frames(0, 5));
        r.clear();
        assert!(r.is_empty());
        assert!(r.reader().unwrap().pop().is_none());
    }

    #[test]
    fn the_callback_is_never_made_to_wait() {
        let r = Ring::new(8);
        r.push(&frames(0, 2));
        r.exclusive(|_| assert!(r.reader().is_none(), "a busy gate gives the callback nothing, at once"));
        assert!(r.reader().is_some());
    }

    #[test]
    fn every_frame_crosses_between_threads_in_order() {
        const TOTAL: usize = 400_000;
        let r = Ring::new(1024);
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut next = 0;
                while next < TOTAL {
                    let n = (1 + (next * 7919) % 300).min(TOTAL - next);
                    let sent = r.push(&frames(next, n));
                    next += sent;
                    if sent < n {
                        std::thread::yield_now();
                    }
                }
            });
            let mut expect = 0usize;
            while expect < TOTAL {
                let Some(mut rd) = r.reader() else { continue };
                while let Some((l, rr)) = rd.pop() {
                    assert_eq!((l, rr), (expect as f32, -(expect as f32)), "frame {expect}");
                    expect += 1;
                }
                drop(rd);
                std::thread::yield_now();
            }
        });
    }

    #[test]
    fn clears_and_cuts_under_a_running_callback_never_tear_a_frame_or_go_backwards() {
        const TOTAL: usize = 300_000;
        let r = Ring::new(512);
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut next = 0usize;
                let (mut round, mut since_cut, mut since_clear) = (0usize, 0, 0);
                while next < TOTAL {
                    let n = (1 + ((round + 1) * 31) % 200).min(TOTAL - next);
                    let pushed = r.push(&frames(next, n));
                    if pushed == 0 {
                        std::thread::yield_now();
                        continue;
                    }
                    next += pushed;
                    round += 1;
                    since_cut += 1;
                    since_clear += 1;
                    if since_cut == 17 {
                        since_cut = 0;
                        next -= r.exclusive(|x| {
                            let before = x.len();
                            x.truncate(before / 2);
                            before - x.len()
                        });
                    }
                    if since_clear == 53 {
                        since_clear = 0;
                        r.clear();
                    }
                }
                done.store(true, std::sync::atomic::Ordering::Release);
            });
            let mut last: Option<usize> = None;
            let mut seen = 0usize;
            loop {
                let finished = done.load(std::sync::atomic::Ordering::Acquire);
                if let Some(mut rd) = r.reader() {
                    while let Some((l, rr)) = rd.pop() {
                        assert_eq!(l, -rr, "a frame with its two halves from different writes");
                        let n = l as usize;
                        assert!(last < Some(n), "frame {n} came after {last:?}");
                        last = Some(n);
                        seen += 1;
                    }
                }
                if finished && r.is_empty() {
                    break;
                }
                std::thread::yield_now();
            }
            assert!(seen > TOTAL / 4, "only {seen} of {TOTAL} frames got through");
        });
    }
}

#[test]
fn seeking_by_a_step_counts_every_press_even_before_the_screen_catches_up() {
    use crate::audio::{Repeat, State, TestPlayer};
    let dir = std::env::temp_dir().join(format!("urchin_seekby_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let long = dir.join("long.wav");
    write_wav(&long, 48_000, 10.0);
    let mut p = TestPlayer::new(vec![long], 0, Repeat::Off);
    p.listen(0.1);
    let at = p.position().unwrap();

    for _ in 0..3 {
        p.seek_by(1.0);
    }
    let pos = p.position().unwrap();
    assert!(pos > at + 2.9 && pos < at + 3.05, "{at} -> {pos}");
    p.seek_by(-100.0);
    assert!(p.position().unwrap() < 0.03);

    p.listen(0.3);
    p.toggle_pause();
    assert_eq!(p.status().state, State::Paused);
    p.restart();
    let st = p.status();
    assert_eq!(st.state, State::Playing);
    assert!(st.position < 0.03, "{}", st.position);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn losing_the_output_holds_the_place_and_a_new_one_carries_on_from_it() {
    use crate::audio::{Repeat, State, TestPlayer};
    let dir = std::env::temp_dir().join(format!("urchin_lost_out_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_wav(&a, 44_100, 3.0);
    write_wav(&b, 48_000, 3.0);
    let mut p = TestPlayer::new(vec![a, b], 0, Repeat::Off);
    assert_eq!(p.listen(0.5), [0]);
    let before = p.position().unwrap();
    assert!((before - 0.5).abs() < 0.03, "{before}");

    p.lose_output();
    let st = p.status();
    assert_eq!(st.state, State::Paused);
    assert!(st.error.is_none(), "no message: {:?}", st.error);
    assert_eq!(st.index, 0);
    assert!((st.position - before).abs() < 0.03, "{} vs {before}", st.position);
    assert_eq!(p.listen(0.3), [] as [usize; 0], "nothing plays, and nothing moves on");
    assert!((p.status().position - before).abs() < 0.03);

    p.regain_output(96_000);
    let st = p.status();
    assert_eq!(st.state, State::Playing);
    assert!(st.error.is_none());
    assert!((st.position - before).abs() < 0.03, "{} vs {before}", st.position);
    assert_eq!(p.listen(0.5), [0]);
    let later = p.position().unwrap();
    assert!((later - (before + 0.5)).abs() < 0.04, "{before} -> {later}");

    p.toggle_pause();
    p.lose_output();
    p.regain_output(48_000);
    assert_eq!(p.status().state, State::Paused);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_engine_says_when_it_has_dealt_with_what_it_was_sent() {
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};
    let _g = serial();
    let root = library();
    let song = root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav");
    let store = Arc::new(StdMutex::new(crate::eq::Store::default()));
    let engine = crate::audio::Engine::start(0.0, store, false);
    assert_eq!(engine.play(vec![song.clone()], 0), 1);
    assert_eq!(engine.play(vec![song], 0), 2);

    let wait = |engine: &crate::audio::Engine| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let st = engine.status();
            if engine.caught_up(&st) {
                return st;
            }
            assert!(Instant::now() < deadline, "the engine never caught up");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let st = wait(&engine);
    assert_eq!(st.plays, 2, "both starts were dealt with");
    assert_eq!(st.acked, 2);

    engine.set_repeat(crate::audio::Repeat::One);
    assert!(!engine.caught_up(&st));
    let st = wait(&engine);
    assert_eq!(st.acked, 3);
    assert_eq!(st.plays, 2, "only Play counts as a start");
}

#[test]
fn playing_an_empty_folder_does_not_arm_the_one_pane_view_for_the_song_already_playing() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let mut app = App::new(Some(music.clone()));
    render(&mut app);
    app.navigate_to(&music.join("punk/crust"));
    render(&mut app);
    app.run(Action::PlaySelection);
    app.status.state = crate::audio::State::Playing;
    app.status.plays = 99;
    app.status.position = 60.0;
    app.status.queue = std::sync::Arc::new(vec![music.join("punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav")]);
    app.check_solo();
    assert!(!app.solo, "nothing was started, so there is nothing to switch to");
}

#[test]
#[ignore]
fn real_output_stays_paused_after_the_device_is_lost_until_play() {
    use crate::audio::{Engine, State, Status};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};
    let _g = serial();
    let dir = std::env::temp_dir().join(format!("urchin_real_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let song = dir.join("long.wav");
    write_wav(&song, 44_100, 30.0);
    std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
    let engine = Engine::start(0.0, Arc::new(StdMutex::new(crate::eq::Store::default())), false);
    engine.play(vec![song], 0);

    let until = |secs: u64, what: &str, ok: &dyn Fn(&Status) -> bool| -> Status {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let st = engine.status();
            if ok(&st) {
                return st;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {st:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let st = until(5, "playback to start", &|s| s.state != State::Stopped || s.error.is_some());
    if st.state == State::Stopped {
        println!("no sound card here: {:?}", st.error);
        return;
    }
    let st = until(10, "a second of music", &|s| s.position >= 1.0);
    assert!(!st.device.is_empty(), "the output's name is published");
    let at = st.position;

    engine.lose_output_for_test();
    let held = until(3, "the loss to show", &|s| s.state == State::Paused && s.device.is_empty());
    assert!(held.error.is_none(), "no message: {:?}", held.error);
    assert_eq!(held.state, State::Paused, "{held:?}");
    assert!(held.position >= at - 0.05 && held.position < at + 0.5, "{at} -> {}", held.position);
    engine.media_toggle_pause();
    std::thread::sleep(Duration::from_secs(3));
    let still = engine.status();
    assert_eq!(still.state, State::Paused, "{still:?}");
    assert!(still.device.is_empty() && (still.position - held.position).abs() < 0.01, "{still:?}");
    engine.toggle_pause();
    let back = until(15, "the output to open on Play", &|s| s.state == State::Playing && s.error.is_none());
    assert!(back.position >= held.position - 0.1, "carried on from {} not {}", back.position, held.position);
    let later = until(10, "music to flow again", &|s| s.position >= back.position + 0.5);
    assert!(!later.device.is_empty());
    println!(
        "real output {:?}: 1 s reached at {at:.2}; lost at {:.2}; back at {:.2}; {:.2} s later",
        later.device, held.position, back.position, later.position
    );
    engine.stop_now();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn real_output_is_closed_when_idle_and_opened_again_by_the_next_play() {
    use crate::audio::{Engine, State, Status};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};
    let _g = serial();
    let dir = std::env::temp_dir().join(format!("urchin_idle_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let song = dir.join("short.wav");
    write_wav(&song, 44_100, 1.0);
    std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
    let engine = Engine::start(0.0, Arc::new(StdMutex::new(crate::eq::Store::default())), false);
    engine.release_idle_output_after_for_test(Duration::from_millis(300));

    let until = |secs: u64, what: &str, ok: &dyn Fn(&Status) -> bool| -> Status {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let st = engine.status();
            if ok(&st) {
                return st;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {st:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    engine.play(vec![song.clone()], 0);
    let st = until(5, "playback to start", &|s| s.state != State::Stopped || s.error.is_some());
    if st.state == State::Stopped {
        println!("no sound card here: {:?}", st.error);
        return;
    }
    until(5, "sound", &|s| s.state == State::Playing && s.position >= 0.3);
    until(10, "the song to end", &|s| s.state == State::Stopped);

    let opened = engine.outputs_opened_for_test();
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(engine.outputs_opened_for_test(), opened);

    engine.play(vec![song], 0);
    until(5, "sound after the output was closed", &|s| s.state == State::Playing && s.position >= 0.3);
    assert_eq!(engine.outputs_opened_for_test(), opened + 1, "the output had been closed, so Play opened one");
    engine.stop_now();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_screen_redraws_calmly_while_nothing_moves_and_soon_after_a_command() {
    use std::time::{Duration, Instant};
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.tick();
        if app.engine.caught_up(&app.status) {
            break;
        }
        assert!(Instant::now() < deadline, "the engine never caught up");
        std::thread::sleep(Duration::from_millis(5));
    }
    let every = |app: &App| app.frame_interval();

    app.settings.analyzer = true;
    assert_eq!(every(&app), Duration::from_millis(500));
    app.settings.analyzer = false;
    assert_eq!(every(&app), Duration::from_millis(500));
    app.status.state = crate::audio::State::Playing;
    assert_eq!(every(&app), Duration::from_millis(250));
    app.settings.analyzer = true;
    assert_eq!(every(&app), Duration::from_millis(33));
    app.settings.analyzer = false;
    app.engine.set_repeat(crate::audio::Repeat::One);
    assert!(!app.engine.caught_up(&app.status));
    assert_eq!(every(&app), Duration::from_millis(20));
    let _ = fs::remove_dir_all(&root);
}

fn slider_cells(app: &mut App, window: u16) -> Vec<ratatui::buffer::Cell> {
    let buf = draw_at(app, window, 30);
    let r = app.volume_rect;
    (r.x..r.x + r.width).map(|x| buf[(x, r.y)].clone()).collect()
}

fn slider_symbols(app: &mut App, window: u16) -> Vec<String> {
    slider_cells(app, window).iter().map(|c| c.symbol().to_string()).collect()
}

fn volume_percent(app: &App) -> u32 {
    (app.engine.volume() * 100.0).round() as u32
}

fn tick_cells(bar: &[String]) -> Vec<usize> {
    (0..bar.len()).filter(|&i| bar[i] == "▏").collect()
}

#[test]
fn the_volume_slider_has_a_tick_at_every_five_percent_in_every_window() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    app.engine.set_volume(0.0);

    let bar = slider_symbols(&mut app, 120);
    assert_eq!(bar.len(), 40);
    assert_eq!(tick_cells(&bar), (0..20).map(|k| k * 2).collect::<Vec<usize>>(), "{bar:?}");
    assert!(bar.iter().all(|s| s == "▏" || s == " "), "nothing is lit at 0%: {bar:?}");

    for window in 53..=200u16 {
        let bar = slider_symbols(&mut app, window);
        assert_eq!(bar.len(), if window >= 103 { 40 } else { 20 }, "{window} columns");
        assert_eq!(tick_cells(&bar).len(), 20, "{window} columns: {bar:?}");
    }
    let bar = slider_symbols(&mut app, 80);
    assert_eq!(tick_cells(&bar), (0..20).collect::<Vec<usize>>(), "{bar:?}");
    for window in 30..53u16 {
        let bar = slider_symbols(&mut app, window);
        assert_eq!(bar.len(), 10, "{window} columns");
        assert_eq!(tick_cells(&bar), (0..10).collect::<Vec<usize>>(), "{window} columns");
    }

    let pal = app.palette();
    app.engine.set_volume(0.5);
    let buf = draw_at(&mut app, 120, 30);
    let r = app.volume_rect;
    let at = |i: u16| buf[(r.x + i, r.y)].clone();
    assert!((0..20).all(|i| matches!(at(i).symbol(), " " | "▏")), "no block characters in the fill");
    assert!((0..19).all(|i| at(i).bg == pal.accent), "the fill is the accent all the way");
    assert_eq!(at(19).bg, pal.lead, "the leading bar is brighter");
    assert_eq!(at(20).symbol(), "▏", "the fill ends on the 50% tick");
    assert!((20..40).all(|i| at(i).bg == pal.raised), "the bare track");
    assert_eq!(at(1).symbol(), " ");
    assert_eq!(at(2).symbol(), "▏");
    assert_eq!(at(2).bg, pal.accent, "a lit tick sits on the fill color");
    assert_eq!(at(2).fg, pal.accent_fg);
    assert_ne!(at(2).fg, at(2).bg, "and can be seen");
    assert_eq!(at(22).symbol(), "▏");
    assert_eq!(at(22).bg, at(21).bg, "a tick past the fill sits on the bare track");
    assert_ne!(at(22).fg, at(22).bg);
    assert_ne!(at(22).bg, at(2).bg, "so the lit side and the bare side differ");

    app.engine.set_volume(0.52);
    let bar = slider_cells(&mut app, 120);
    assert_eq!(bar[20].symbol(), "▊");
    assert_eq!((bar[20].fg, bar[20].bg), (pal.lead, pal.raised), "the end of the fill is the brighter bar");
    assert_eq!(bar[19].bg, pal.accent, "and the cell before it is plain fill again");
    assert_eq!(bar[21].symbol(), " ");
    assert_eq!(bar[22].symbol(), "▏");

    for window in [120u16, 80] {
        let mut last = Vec::new();
        for p in 0..=100 {
            app.engine.set_volume(p as f32 / 100.0);
            let now: Vec<_> = slider_cells(&mut app, window).iter().map(|c| (c.symbol().to_string(), c.fg, c.bg)).collect();
            assert_ne!(now, last, "{window} columns: {p}% looks the same as {}%", p.max(1) - 1);
            last = now;
        }
    }

    app.engine.set_volume(0.0);
    draw_at(&mut app, 120, 30);
    let r = app.volume_rect;
    assert_eq!(r.width, 40);
    click_at(&mut app, r.x + 19, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 50);
    click_at(&mut app, r.x + 29, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 75);
    click_at(&mut app, r.x + 1, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 5);
    click_at(&mut app, r.x + 39, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 100);
    click_at(&mut app, r.x - 2, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 0);
    draw_at(&mut app, 80, 30);
    let r = app.volume_rect;
    assert_eq!(r.width, 20);
    click_at(&mut app, r.x + 9, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 50);
    click_at(&mut app, r.x + 19, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 100);
    click_at(&mut app, r.x, r.y, MouseButton::Left);
    assert_eq!(volume_percent(&app), 5);

    app.engine.set_volume(0.5);
    for _ in 0..7 {
        key(&mut app, KeyCode::Char('+'));
    }
    assert_eq!(volume_percent(&app), 57);
    for _ in 0..7 {
        key(&mut app, KeyCode::Char('-'));
    }
    assert_eq!(volume_percent(&app), 50);
    assert!((app.engine.volume() - 0.5).abs() < 1e-6, "{}", app.engine.volume());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn only_the_leading_bar_of_the_volume_slider_is_brighter() {
    use crate::theme::luminance_of;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let pal = app.palette();
    assert_ne!(pal.lead, pal.accent);
    assert!(luminance_of(pal.lead) > luminance_of(pal.accent), "brighter, not darker");

    app.engine.set_volume(0.67);
    let bar = slider_cells(&mut app, 85);
    assert_eq!(bar.len(), 20);
    for (i, c) in bar.iter().enumerate().take(13) {
        assert_eq!((c.symbol(), c.fg, c.bg), ("▏", pal.accent_fg, pal.accent), "bar {i}");
    }
    assert_eq!((bar[13].symbol(), bar[13].fg, bar[13].bg), ("▍", pal.lead, pal.raised));
    for (i, c) in bar.iter().enumerate().skip(14) {
        assert_eq!((c.symbol(), c.fg, c.bg), ("▏", pal.dim, pal.raised), "bare cell {i}");
    }

    for window in [120u16, 85, 45] {
        for percent in 0..=100u32 {
            app.engine.set_volume(percent as f32 / 100.0);
            let bar = slider_cells(&mut app, window);
            let is_lead = |c: &ratatui::buffer::Cell| c.bg == pal.lead || (c.fg == pal.lead && c.bg == pal.raised);
            let is_plain = |c: &ratatui::buffer::Cell| c.bg == pal.accent;
            let leading: Vec<usize> = (0..bar.len()).filter(|&i| is_lead(&bar[i])).collect();
            let plain = bar.iter().filter(|c| is_plain(c)).count();
            if percent == 0 {
                assert!(leading.is_empty() && plain == 0, "{window} columns: something lit at 0%");
                continue;
            }
            assert_eq!(leading.len(), 1, "{window} columns, {percent}%: {leading:?}");
            let end = leading[0];
            assert!((0..end).all(|i| is_plain(&bar[i])), "{window} columns, {percent}%: plain bars before the leading one");
            assert_eq!(plain, end, "{window} columns, {percent}%");
            assert!((end + 1..bar.len()).all(|i| bar[i].bg == pal.raised && bar[i].fg != pal.lead), "{window} columns, {percent}%");
            for c in &bar[..end] {
                assert!(matches!(c.symbol(), " " | "▏"), "{window} columns, {percent}%: {}", c.symbol());
                if c.symbol() == "▏" {
                    assert_eq!(c.fg, pal.accent_fg);
                }
            }
            if end > 0 {
                assert_eq!((bar[0].symbol(), bar[0].fg, bar[0].bg), ("▏", pal.accent_fg, pal.accent), "{window} columns, {percent}%: the far-left bar");
            }
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_volume_sliders_leading_bar_stands_out_in_every_theme() {
    use crate::theme::{contrast_of, luminance_of, THEMES};
    let _g = serial();
    for t in THEMES {
        let p = t.palette();
        let dark = luminance_of(p.bg) < luminance_of(p.fg);
        if dark {
            assert!(luminance_of(p.lead) >= luminance_of(p.accent), "{}: not brighter", t.name);
        } else {
            assert!(luminance_of(p.lead) <= luminance_of(p.accent), "{}: not deeper", t.name);
        }
        assert!(contrast_of(p.lead, p.raised) >= contrast_of(p.accent, p.raised), "{}: the leading bar fades into the track", t.name);
        assert!(contrast_of(p.lead_fg, p.lead) >= 2.5, "{}: tick unreadable on the leading bar", t.name);
    }
    for name in ["Dracula", "Gilded-Forest-Dark"] {
        let p = THEMES[theme::index_of(name).unwrap()].palette();
        assert!(contrast_of(p.lead, p.accent) >= 1.3, "{name}: the leading bar barely differs");
    }
}

#[test]
fn the_song_title_gives_way_to_the_slider_not_the_other_way_round() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    app.engine.set_volume(0.5);
    for window in [60u16, 80, 90, 100, 102, 103, 120, 160] {
        let screen = render_at(&mut app, window, 30);
        let bar = app.volume_rect;
        assert_eq!(bar.width, if window >= 103 { 40 } else { 20 }, "{window} columns");
        assert!(screen.contains("vol"), "{window} columns:\n{screen}");
        assert!(screen.contains(" 50%"), "{window} columns:\n{screen}");
        if window >= 80 {
            assert!(screen.contains("double-click a folder"), "{window} columns:\n{screen}");
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_one_pane_view_holds_in_a_folder_that_has_subfolders() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let album = music.join("punk/hardcore/Minor Threat/Complete Discography");
    for sub in ["Bonus Disc", "Live"] {
        let dir = album.join(sub);
        fs::create_dir_all(&dir).unwrap();
        write_wav(&dir.join("01 - Extra.wav"), 44_100, 0.5);
    }
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    let mut app = App::new(Some(music));
    render(&mut app);
    app.navigate_to(&album);
    render(&mut app);
    let row = app.levels[app.focus].entries.iter().position(|e| e.path == song).unwrap();
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    click(&mut app, |t| *t == Target::Row { col, row });
    app.status.plays = app.engine.plays_sent();
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![album.join("01 - Filler.wav"), song]);
    app.status.index = 1;
    app.status.position = 13.5;
    app.check_solo();
    let panes = |app: &App| app.hits.iter().filter(|h| matches!(h.target, Target::Column(_))).count();
    let screen = render(&mut app);
    assert!(app.solo, "{screen}");
    assert_eq!(panes(&app), 1);

    let mut landed_on_a_folder = false;
    let keys = [
        KeyCode::Home, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::End,
        KeyCode::Up, KeyCode::PageUp, KeyCode::PageDown, KeyCode::Char('k'), KeyCode::Char('j'), KeyCode::Home,
    ];
    for code in keys {
        key(&mut app, code);
        let screen = render(&mut app);
        landed_on_a_folder |= app.selected_entry().is_some_and(|e| e.kind == crate::fsx::Kind::Folder);
        assert!(app.solo, "{code:?} left the one-pane view:\n{screen}");
        assert_eq!(panes(&app), 1, "{code:?} showed two panes:\n{screen}");
        assert_eq!(app.levels[app.focus].dir, album, "{code:?}");
        assert!(screen.contains("Complete Discography"), "{code:?}:\n{screen}");
        assert!(screen.contains("Bonus Disc"), "the folders are rows of the one pane:\n{screen}");
        assert!(!screen.contains("Extra"), "the folder under the cursor must not open beside it:\n{screen}");
    }
    assert!(landed_on_a_folder, "the walk never reached a subfolder, so it proved nothing");

    let pane = rect_of(&app, |t| matches!(t, Target::Column(_)));
    for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
        app.on_mouse(MouseEvent { kind, column: pane.x + 3, row: pane.y + 3, modifiers: KeyModifiers::NONE });
        render(&mut app);
        assert!(app.solo);
        assert_eq!(panes(&app), 1);
    }
    let _ = fs::remove_dir_all(&root);
}

fn listening_in_a_folder(seconds: f64) -> (App, PathBuf, PathBuf) {
    let root = library();
    let music = root.join("Music");
    let album = music.join("punk/hardcore/Minor Threat/Complete Discography");
    for sub in ["Bonus Disc", "Live"] {
        let dir = album.join(sub);
        fs::create_dir_all(&dir).unwrap();
        write_wav(&dir.join("01 - Extra.wav"), 44_100, 0.5);
    }
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    let mut app = App::new(Some(music));
    render(&mut app);
    app.navigate_to(&album);
    render(&mut app);
    let row = app.levels[app.focus].entries.iter().position(|e| e.path == song).unwrap();
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    click(&mut app, |t| *t == Target::Row { col, row });
    app.status.plays = app.engine.plays_sent();
    app.status.state = crate::audio::State::Playing;
    app.status.queue = std::sync::Arc::new(vec![album.join("01 - Filler.wav"), song]);
    app.status.index = 1;
    app.status.position = seconds;
    app.check_solo();
    render(&mut app);
    (app, root, album)
}

fn panes_on_screen(app: &App) -> usize {
    app.hits.iter().filter(|h| matches!(h.target, Target::Column(_))).count()
}

#[test]
fn arrow_keys_never_bring_the_other_folders_back_while_you_listen() {
    let _g = serial();
    let (mut app, root, album) = listening_in_a_folder(20.0);
    assert!(app.solo);
    for code in [KeyCode::F(1), KeyCode::Esc, KeyCode::F(10), KeyCode::Esc, KeyCode::Char('x'), KeyCode::Char('v'), KeyCode::Char('m')] {
        key(&mut app, code);
        render(&mut app);
        assert!(app.solo, "{code:?} switched the one-pane view off");
    }
    app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Esc);
    app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Esc);
    render(&mut app);
    assert!(app.solo, "opening and closing a dialog switched the one-pane view off");
    for code in [KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::End, KeyCode::Up] {
        key(&mut app, code);
        let screen = render(&mut app);
        assert_eq!(panes_on_screen(&app), 1, "{code:?} showed two panes:\n{screen}");
        assert_eq!(app.levels[app.focus].dir, album);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn up_and_down_in_the_playing_folder_stay_in_one_pane_even_before_5_seconds() {
    let _g = serial();
    let (mut app, root, album) = listening_in_a_folder(2.0);
    assert!(!app.solo);
    assert_eq!(panes_on_screen(&app), 2);
    let mut landed_on_a_folder = false;
    for code in [KeyCode::Home, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::End, KeyCode::Char('k')] {
        key(&mut app, code);
        let screen = render(&mut app);
        landed_on_a_folder |= app.selected_entry().is_some_and(|e| e.kind == crate::fsx::Kind::Folder);
        assert_eq!(panes_on_screen(&app), 1, "{code:?} showed two panes:\n{screen}");
        assert!(!screen.contains("Extra"), "the folder under the cursor opened beside it:\n{screen}");
        assert_eq!(app.levels[app.focus].dir, album);
    }
    assert!(landed_on_a_folder);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn up_and_down_still_browse_normally_when_the_music_is_in_another_folder() {
    let _g = serial();
    let (mut app, root, album) = listening_in_a_folder(20.0);
    key(&mut app, KeyCode::Left);
    render(&mut app);
    assert!(!app.solo);
    assert_eq!(panes_on_screen(&app), 2);
    assert_ne!(app.current_dir(), album);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Up);
    render(&mut app);
    assert_eq!(panes_on_screen(&app), 2, "browsing another folder keeps parent and child side by side");
    let _ = fs::remove_dir_all(&root);
}

fn write_tagged_mp3(path: &Path, title: &str, artist: &str, album: &str, art: usize) {
    fn frame(id: &str, mut body: Vec<u8>) -> Vec<u8> {
        let mut f = id.as_bytes().to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.append(&mut body);
        f
    }
    let text = |s: &str| {
        let mut b = vec![0u8];
        b.extend_from_slice(s.as_bytes());
        b
    };
    let mut tag = Vec::new();
    tag.extend(frame("TIT2", text(title)));
    if art > 0 {
        let mut b = vec![0u8];
        b.extend_from_slice(b"image/png\0");
        b.push(3);
        b.push(0);
        b.resize(b.len() + art, 0x5a);
        tag.extend(frame("APIC", b));
    }
    tag.extend(frame("TPE1", text(artist)));
    tag.extend(frame("TALB", text(album)));
    let n = tag.len();
    let mut out = b"ID3\x03\x00\x00".to_vec();
    out.extend_from_slice(&[(n >> 21 & 0x7f) as u8, (n >> 14 & 0x7f) as u8, (n >> 7 & 0x7f) as u8, (n & 0x7f) as u8]);
    out.extend(tag);
    for _ in 0..40 {
        out.extend_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        out.resize(out.len() + 413, 0u8);
    }
    fs::write(path, out).unwrap();
}

#[test]
fn the_tags_at_the_start_of_an_mp3_are_read_with_or_without_a_picture() {
    let dir = std::env::temp_dir().join(format!("urchin_id3_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, art) in [("plain.mp3", 0), ("with_cover.mp3", 300_000)] {
        let path = dir.join(name);
        write_tagged_mp3(&path, "Seeing Red", "Minor Threat", "Complete Discography", art);
        let meta = crate::decode::open(&path).unwrap().meta;
        assert_eq!(meta.title, "Seeing Red", "{name}");
        assert_eq!(meta.artist, "Minor Threat", "{name}");
        assert_eq!(meta.album, "Complete Discography", "{name}");
    }
    let _ = fs::remove_dir_all(&dir);
}

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::spectrum::Tap;
use crate::viz::Trace;

fn run_trace(
    trace: &mut dyn crate::visuals::Picture,
    tap: &Tap,
    (columns, rows): (usize, u16),
    seed: u64,
    start: f32,
    secs: f32,
    signal: impl Fn(f32) -> f32,
    clock: &mut std::time::Instant,
) {
    for f in 0..(secs * 30.0) as usize {
        let mut w = tap.writer();
        for i in 0..1600 {
            w.push(signal(start + (f * 1600 + i) as f32 / 48_000.0));
        }
        drop(w);
        *clock += std::time::Duration::from_millis(33);
        trace.update(tap, columns, rows, seed, *clock);
    }
}

fn run_trace_silent(trace: &mut dyn crate::visuals::Picture, tap: &Tap, (columns, rows): (usize, u16), seed: u64, secs: f32, clock: &mut std::time::Instant) {
    for _ in 0..(secs * 30.0) as usize {
        *clock += std::time::Duration::from_millis(33);
        trace.update(tap, columns, rows, seed, *clock);
    }
}

fn painted(trace: &mut Trace, theme: usize, (columns, rows): (usize, u16)) -> Buffer {
    trace.use_theme(theme, || theme::THEMES[theme].trace_colors());
    let area = Rect::new(0, 0, columns as u16, rows);
    let mut buf = Buffer::empty(area);
    trace.paint(&mut buf, area);
    buf
}

fn glyphs(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()).collect()
}

fn dracula() -> usize {
    theme::index_of("Dracula").unwrap()
}

#[test]
fn trace_is_a_function_of_the_audio_and_nothing_else() {
    let size = (85, 13);
    let music = |t: f32| (synth_music(t) * 2.2).clamp(-1.0, 1.0);
    let picture = |signal: &dyn Fn(f32) -> f32, seed: u64| {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        run_trace(&mut trace, &tap, size, seed, 0.0, 3.0, signal, &mut clock);
        painted(&mut trace, dracula(), size)
    };
    let a = picture(&music, 42);
    assert_eq!(a, picture(&music, 42));
    assert_ne!(a, picture(&music, 43));
    let other = |t: f32| 0.5 * (std::f32::consts::TAU * 330.0 * t).sin() + 0.4 * (std::f32::consts::TAU * 3100.0 * t).sin();
    assert_ne!(glyphs(&a), glyphs(&picture(&other, 42)));
    assert!(glyphs(&a).iter().any(|row| row.contains('|')), "{:?}", glyphs(&a));
}

#[test]
fn trace_puts_bass_on_the_left_and_treble_on_the_right() {
    use std::f32::consts::TAU;
    let size = (90usize, 15u16);
    let height = |signal: &dyn Fn(f32) -> f32| {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        run_trace(&mut trace, &tap, size, 1, 0.0, 2.0, signal, &mut clock);
        let amps = trace.amps().to_vec();
        let tallest = |i: usize| amps[i * 30..(i + 1) * 30].iter().copied().fold(0.0f32, f32::max);
        (tallest(0), tallest(1), tallest(2))
    };
    let bass = height(&|t| 0.5 * (TAU * 60.0 * t).sin() + 0.3 * (TAU * 90.0 * t).sin());
    let treble = height(&|t| 0.5 * (TAU * 6000.0 * t).sin() + 0.3 * (TAU * 9000.0 * t).sin());
    assert!(bass.0 > 0.3 && bass.0 > 2.0 * bass.2, "bass: {bass:?}");
    assert!(treble.2 > 0.3 && treble.2 > 2.0 * treble.0, "treble: {treble:?}");
    let mid = height(&|t| 0.5 * (TAU * 800.0 * t).sin() + 0.3 * (TAU * 1100.0 * t).sin());
    assert!(mid.1 > 0.3 && mid.1 > 2.0 * mid.0 && mid.1 > 2.0 * mid.2, "mid: {mid:?}");
}

#[test]
fn trace_follows_how_loud_the_music_is() {
    let size = (85, 13);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mean = |t: &Trace| t.amps().iter().sum::<f32>() / t.amps().len() as f32;
    run_trace(&mut trace, &tap, size, 1, 0.0, 4.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
    let loud = mean(&trace);
    run_trace(&mut trace, &tap, size, 1, 4.0, 0.7, |t| synth_music(t) * 0.22, &mut clock);
    let quiet = mean(&trace);
    assert!(loud > 0.12, "loud music should fill the window: {loud}");
    assert!(quiet < 0.6 * loud, "quiet {quiet} against loud {loud}");
    run_trace(&mut trace, &tap, size, 1, 4.7, 0.7, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
    assert!(mean(&trace) > 1.5 * quiet, "{} against {quiet}", mean(&trace));
}

#[test]
fn trace_adapts_to_quiet_masters_and_loud_ones_alike() {
    let size = (85, 13);
    let fill = |gain: f32| {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        run_trace(&mut trace, &tap, size, 1, 0.0, 20.0, |t| (synth_music(t) * gain).clamp(-1.0, 1.0), &mut clock);
        trace.amps().iter().sum::<f32>() / trace.amps().len() as f32
    };
    let (loud, quiet) = (fill(2.2), fill(0.4));
    assert!(quiet > 0.55 * loud && quiet < 1.8 * loud, "loud {loud}, quiet {quiet}");
}

#[test]
fn trace_flares_on_a_kick_and_not_on_a_steady_tone() {
    use std::f32::consts::TAU;
    let size = (85, 13);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    run_trace(&mut trace, &tap, size, 1, 0.0, 2.0, |t| 0.2 * (TAU * 220.0 * t).sin() + 0.1 * (TAU * 330.0 * t).sin(), &mut clock);
    assert!(trace.flash() < 0.05, "flare on a steady tone: {}", trace.flash());
    let mut seen = 0.0f32;
    for f in 0..30 {
        let mut w = tap.writer();
        for i in 0..1600 {
            let t = 2.0 + (f * 1600 + i) as f32 / 48_000.0;
            let beat = t % 0.5;
            let kick = 0.9 * (-beat * 12.0).exp() * (TAU * (50.0 * beat + 70.0 / 18.0 * (1.0 - (-18.0 * beat).exp()))).sin();
            w.push((kick + 0.2 * (TAU * 220.0 * t).sin()).clamp(-1.0, 1.0));
        }
        drop(w);
        clock += std::time::Duration::from_millis(33);
        trace.update(&tap, size.0, size.1, 1, clock);
        seen = seen.max(trace.flash());
    }
    assert!(seen > 0.5, "no flare on kicks: {seen}");
}

fn trace_music(t: f32) -> f32 {
    (synth_music(t) * 2.2).clamp(-1.0, 1.0)
}

fn distance_from(buf: &Buffer, flat: ratatui::style::Color) -> f32 {
    use ratatui::style::Color;
    let channels = |c: Color| match c {
        Color::Rgb(r, g, b) => [r as f32, g as f32, b as f32],
        other => panic!("not a plain color: {other:?}"),
    };
    let apart = |c: Color| channels(c).iter().zip(channels(flat)).map(|(x, y)| (x - y).abs()).sum::<f32>();
    buf.content.iter().map(|c| if c.symbol() == " " { apart(c.bg) } else { apart(c.bg).max(apart(c.fg)) }).sum()
}

#[test]
fn trace_fades_into_one_solid_color_when_the_music_stops() {
    let size = (85usize, 12u16);
    let flat = theme::THEMES[dracula()].trace_colors().at(crate::viz::BG_MIN);
    for silence_arrives in [true, false] {
        let how = if silence_arrives { "silence still arriving" } else { "nothing arriving" };
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for f in 0..90usize {
            run_trace(&mut trace, &tap, size, 1, f as f32 / 30.0, 1.0 / 30.0, trace_music, &mut clock);
            let _ = painted(&mut trace, dracula(), size);
        }
        let playing = painted(&mut trace, dracula(), size);
        assert!(glyphs(&playing).iter().any(|row| row.contains('|')), "{how}: nothing to fade");
        let start = distance_from(&playing, flat);
        assert!(start > 0.0);

        let mut left = Vec::new();
        for f in 0..150usize {
            if silence_arrives {
                run_trace(&mut trace, &tap, size, 1, 3.0 + f as f32 / 30.0, 1.0 / 30.0, |_| 0.0, &mut clock);
            } else {
                run_trace_silent(&mut trace, &tap, size, 1, 1.0 / 30.0, &mut clock);
            }
            left.push(distance_from(&painted(&mut trace, dracula(), size), flat) / start);
        }
        println!("{how}: {:?}", left.iter().step_by(5).map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
        assert!(left[14] > 0.7, "{how}: only {:.2} of the picture left after half a second", left[14]);
        assert!(left[29] > 0.45, "{how}: only {:.2} of the picture left after a second", left[29]);
        assert!(left[59] < 0.3, "{how}: {:.2} of the picture left after two seconds", left[59]);
        for f in 7..left.len() {
            assert!(left[f] <= left[f - 1] + 0.003, "{how}: the picture came back a little at frame {f}: {:.3} after {:.3}", left[f], left[f - 1]);
            assert!(left[f - 1] - left[f] < 0.08, "{how}: the picture dropped by {:.3} in one frame at frame {f}", left[f - 1] - left[f]);
        }
        assert_eq!(left[120], 0.0, "{how}: not yet flat after four seconds");
        assert!(!trace.is_active(), "{how}: still moving");
        let end = painted(&mut trace, dracula(), size);
        assert!(end.content.iter().all(|c| c.symbol() == " " && c.fg == flat && c.bg == flat), "{how}: not one solid color");
    }
}

#[test]
fn trace_comes_back_quickly_when_the_music_starts_again() {
    let size = (85usize, 12u16);
    let flat = theme::THEMES[dracula()].trace_colors().at(crate::viz::BG_MIN);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    run_trace(&mut trace, &tap, size, 1, 0.0, 3.0, trace_music, &mut clock);
    let playing = distance_from(&painted(&mut trace, dracula(), size), flat);
    run_trace(&mut trace, &tap, size, 1, 3.0, 4.0, |_| 0.0, &mut clock);
    assert_eq!(distance_from(&painted(&mut trace, dracula(), size), flat), 0.0);
    let mut last = 0.0;
    for f in 0..15usize {
        run_trace(&mut trace, &tap, size, 1, 7.0 + f as f32 / 30.0, 1.0 / 30.0, trace_music, &mut clock);
        assert!(trace.presence() >= last, "frame {f}: going away while the music plays");
        last = trace.presence();
    }
    assert_eq!(trace.presence(), 1.0, "not all back after half a second");
    assert!(distance_from(&painted(&mut trace, dracula(), size), flat) > 0.4 * playing, "the picture is not back");

    run_trace(&mut trace, &tap, size, 1, 7.5, 1.4, |_| 0.0, &mut clock);
    let half = trace.presence();
    assert!(half > 0.1 && half < 0.9, "{half}");
    run_trace(&mut trace, &tap, size, 1, 8.9, 0.5, trace_music, &mut clock);
    assert_eq!(trace.presence(), 1.0);
}

#[test]
fn trace_does_not_fade_for_a_gap_between_two_sounds() {
    let size = (85usize, 12u16);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let with_a_rest = |t: f32| if (3.0..3.09).contains(&t) { 0.0 } else { trace_music(t) };
    run_trace(&mut trace, &tap, size, 1, 0.0, 6.0, with_a_rest, &mut clock);
    assert_eq!(trace.presence(), 1.0);
    run_trace(&mut trace, &tap, size, 1, 6.0, 0.5, |_| 0.0, &mut clock);
    assert!(trace.presence() < 1.0);
}

#[test]
fn trace_has_no_line_through_the_middle() {
    use crate::theme::luminance_of;
    for size in [(60usize, 9u16), (85, 12), (85, 13), (120, 14), (200, 21)] {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for second in 0..4 {
            run_trace(&mut trace, &tap, size, 5, 1.0 + second as f32 * 2.0, 2.0, trace_music, &mut clock);
            let buf = painted(&mut trace, dracula(), size);
            let (columns, line) = (size.0 as u16, ((size.1 - 1) / 2) as usize);
            let rows = glyphs(&buf);
            let needles = |y: usize| rows[y].matches('|').count();
            let light = |y: usize| (0..columns).map(|x| luminance_of(if buf[(x, y as u16)].symbol() == " " { buf[(x, y as u16)].bg } else { buf[(x, y as u16)].fg })).sum::<f32>();
            let at = format!("{}x{} at {}s", size.0, size.1, 3 + 2 * second);

            let beside = needles(line - 1).max(needles(line + 1));
            assert!(needles(line) >= columns as usize / 6, "{at}: only {} needles on the middle row", needles(line));
            assert!(needles(line) >= beside, "{at}: fewer needles on the middle row ({}) than beside it ({beside})", needles(line));
            assert!(needles(line) <= beside + beside / 4 + 3, "{at}: {} needles on the middle row against {beside} beside it", needles(line));

            for side in [-1i32, 1] {
                let row = |d: i32| (line as i32 + side * d) as usize;
                let (middle, next, after) = (light(line), light(row(1)), light(row(2)));
                assert!(middle <= 1.4 * next, "{at}: the middle row stands out as a bar on side {side}: {middle:.1} against {next:.1}");
                assert!(middle - next <= (next - after) * 1.05 + 0.5, "{at}: the light does not fall away smoothly from the middle on side {side}: {middle:.1} {next:.1} {after:.1}");
            }
            for x in 0..columns {
                let lum = |y: usize| {
                    let c = &buf[(x, y as u16)];
                    luminance_of(if c.symbol() == " " { c.bg } else { c.fg })
                };
                assert!(lum(line) <= 1.5 * lum(line - 1).max(lum(line + 1)) + 0.01, "{at}: column {x}: the middle row stands out ({} against {})", lum(line), lum(line - 1).max(lum(line + 1)));
            }
        }
    }
}

#[test]
fn trace_is_ascii_only() {
    for (columns, rows) in [(30usize, 5u16), (85, 13), (85, 12), (120, 14), (220, 21), (400, 30)] {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut seen = std::collections::BTreeSet::new();
        for step in 0..3 {
            run_trace(&mut trace, &tap, (columns, rows), 9, step as f32 * 2.0, 2.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            let buf = painted(&mut trace, dracula(), (columns, rows));
            for y in 0..rows {
                for x in 0..columns as u16 {
                    let s = buf[(x, y)].symbol();
                    assert!(s.len() == 1 && s.is_ascii(), "{columns}x{rows}: {s:?} at {x},{y}");
                    seen.extend(s.chars());
                }
            }
        }
        assert!(seen.iter().all(|c| " .:'|".contains(*c)), "{columns}x{rows}: {seen:?}");
        assert!(seen.contains(&'|'), "{columns}x{rows}: {seen:?}");
        if rows >= 9 {
            assert!(seen.len() >= 3, "{columns}x{rows}: only {seen:?}");
        }
    }
}

#[test]
fn trace_never_draws_the_marks_that_look_ugly() {
    let loud = |t: f32| (synth_music(t) * 6.0).clamp(-1.0, 1.0);
    let noise = |t: f32| (noise_at(t) * 2.0).clamp(-1.0, 1.0);
    let square = |t: f32| if (t * 110.0).fract() < 0.5 { 0.9 } else { -0.9 };
    let signals: [(&str, &dyn Fn(f32) -> f32); 3] = [("loud music", &loud), ("noise", &noise), ("square wave", &square)];
    for (name, signal) in signals {
        for (columns, rows) in [(30usize, 5u16), (85, 12), (85, 13), (120, 14), (220, 21), (400, 30)] {
            let (tap, mut trace) = (Tap::new(), Trace::new());
            let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
            for step in 0..4 {
                run_trace(&mut trace, &tap, (columns, rows), 3, step as f32 * 1.5, 1.5, signal, &mut clock);
                let buf = painted(&mut trace, dracula(), (columns, rows));
                for row in glyphs(&buf) {
                    assert!(!row.chars().any(|c| "#=+-*@".contains(c)), "{name} at {columns}x{rows}: {row}");
                }
            }
        }
    }
}

#[test]
fn trace_has_the_same_picture_in_every_theme_and_only_the_colors_change() {
    use crate::theme::{luminance_of, THEMES};
    use ratatui::style::Color;
    let size = (85usize, 13u16);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    run_trace(&mut trace, &tap, size, 5, 0.0, 3.0, trace_music, &mut clock);
    let reference = painted(&mut trace, dracula(), size);
    let (quiet_tap, mut quiet) = (Tap::new(), Trace::new());
    run_trace_silent(&mut quiet, &quiet_tap, size, 5, 1.0, &mut clock);
    let line = (size.1 - 1) / 2;
    let mut looks = std::collections::BTreeSet::new();
    for (i, t) in THEMES.iter().enumerate() {
        let ramp = t.trace_colors();
        let flat = ramp.at(crate::viz::BG_MIN);
        let (floor, ceiling) = (luminance_of(flat) - 0.012, luminance_of(ramp.at(crate::viz::BG_MAX)) + 0.002);

        let buf = painted(&mut trace, i, size);
        assert_eq!(glyphs(&buf), glyphs(&reference), "{}: the picture depends on the theme", t.name);
        looks.insert(format!("{:?}", [(10, line), (20, line), (40, line), (40, 3), (40, 2)].map(|(x, y)| (buf[(x, y)].fg, buf[(x, y)].bg))));
        for y in 0..size.1 {
            for x in 0..size.0 as u16 {
                let (fg, bg) = (buf[(x, y)].fg, buf[(x, y)].bg);
                assert!(matches!(fg, Color::Rgb(..)) && matches!(bg, Color::Rgb(..)), "{}: not a plain color", t.name);
                let l = luminance_of(bg);
                assert!(l >= floor && l <= ceiling, "{}: background {bg:?} at {x},{y} is off the flat-color-to-glow range", t.name);
            }
        }

        let calm = painted(&mut quiet, i, size);
        for y in 0..size.1 {
            for x in 0..size.0 as u16 {
                let c = &calm[(x, y)];
                assert!(c.symbol() == " " && c.fg == flat && c.bg == flat, "{}: {x},{y} is not the solid color: {c:?}", t.name);
            }
        }
    }
    assert!(looks.len() > 400, "only {} different looks across {} themes", looks.len(), THEMES.len());
}

#[test]
fn trace_ignores_redraws_that_come_between_frames() {
    let size = (85, 13);
    let run = |extra: bool| {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for f in 0..90usize {
            let mut w = tap.writer();
            for i in 0..1600 {
                w.push((synth_music((f * 1600 + i) as f32 / 48_000.0) * 2.2).clamp(-1.0, 1.0));
            }
            drop(w);
            clock += std::time::Duration::from_millis(33);
            trace.update(&tap, size.0, size.1, 1, clock);
            if extra {
                for ms in [5u64, 10, 16] {
                    trace.update(&tap, size.0, size.1, 1, clock + std::time::Duration::from_millis(ms));
                }
            }
        }
        painted(&mut trace, dracula(), size)
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn trace_copes_with_any_window_size() {
    let tap = Tap::new();
    let mut w = tap.writer();
    for i in 0..20_000 {
        w.push((synth_music(i as f32 / 48_000.0) * 2.2).clamp(-1.0, 1.0));
    }
    drop(w);
    let mut trace = Trace::new();
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    for columns in [1usize, 2, 3, 7, 30, 64, 85, 119, 301, 640] {
        for rows in [1u16, 2, 3, 4, 5, 6, 9, 12, 13, 31, 60] {
            for _ in 0..4 {
                clock += std::time::Duration::from_millis(33);
                trace.update(&tap, columns, rows, 3, clock);
            }
            trace.use_theme(0, || theme::THEMES[0].trace_colors());
            let mut buf = Buffer::empty(Rect::new(0, 0, columns as u16 + 6, rows + 4));
            trace.paint(&mut buf, Rect::new(3, 2, columns as u16, rows));
            for y in 0..rows + 4 {
                for x in 0..columns as u16 + 6 {
                    let inside = (3..3 + columns as u16).contains(&x) && (2..2 + rows).contains(&y);
                    assert!(inside || buf[(x, y)] == ratatui::buffer::Cell::default(), "{columns}x{rows}: cell {x},{y} outside the slot was drawn on");
                }
            }
        }
    }
}

#[test]
fn trace_cells_all_get_painted() {
    let size = (60usize, 11u16);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    run_trace(&mut trace, &tap, size, 1, 0.0, 2.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
    trace.use_theme(dracula(), || theme::THEMES[dracula()].trace_colors());
    let area = Rect::new(0, 0, size.0 as u16, size.1);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, ratatui::style::Style::default().fg(ratatui::style::Color::Rgb(1, 2, 3)).bg(ratatui::style::Color::Rgb(4, 5, 6)));
    for c in &mut buf.content {
        c.set_char('Z');
    }
    trace.paint(&mut buf, area);
    for y in 0..size.1 {
        for x in 0..size.0 as u16 {
            let c = &buf[(x, y)];
            assert!(c.symbol() != "Z" && c.bg != ratatui::style::Color::Rgb(4, 5, 6) && c.fg != ratatui::style::Color::Rgb(1, 2, 3), "cell {x},{y} left as it was");
        }
    }
}

#[test]
fn view_menu_chooses_between_the_spectrum_analyzer_and_the_trace() {
    use crate::settings::Settings;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let item = |app: &App, action: Action| app.menu_items(2).into_iter().find(|i| i.action == Some(action.clone())).unwrap();
    let screen = |app: &mut App| render_at(app, 110, 30);
    let has_bars = |s: &str| s.contains("▆▆ ▆▆");

    assert_eq!(app.settings.visual, crate::settings::Visual::Spectrum);
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Spectrum)).checked, Some(true));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Trace)).checked, Some(false));
    assert!(has_bars(&screen(&mut app)));

    app.run(Action::ToggleVisual(Visual::Trace));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Spectrum)).checked, Some(false));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Trace)).checked, Some(true));
    let shown = screen(&mut app);
    assert!(!has_bars(&shown) && !shown.contains('▆'), "{shown}");
    assert_eq!(Settings::load().visual, crate::settings::Visual::Trace);
    assert!(Settings::load().analyzer);

    app.run(Action::ToggleVisual(Visual::Trace));
    assert!(!app.settings.analyzer);
    assert!(!screen(&mut app).contains('▆'));
    app.run(Action::ToggleVisual(Visual::Spectrum));
    assert!(app.settings.analyzer && has_bars(&screen(&mut app)));
    assert_eq!(Settings::load().visual, crate::settings::Visual::Spectrum);

    let mut odd = serde_json::to_value(&app.settings).unwrap();
    odd["visual"] = "something_from_the_future".into();
    assert_eq!(serde_json::from_value::<Settings>(odd).unwrap().visual, Visual::Spectrum);
    for (old, visual) in [("squares", Visual::Lines), ("Synthwave_Urchin", Visual::Synthwave), ("bokeh light", Visual::BokehLight)] {
        assert_eq!(serde_json::from_value::<Settings>(serde_json::json!({ "visual": old })).unwrap().visual, visual);
    }
    for visual in [Visual::Trace, Visual::ClubLights] {
        assert_eq!(Visual::from_name(visual.name()), visual);
    }
    let old: Settings = serde_json::from_str(r#"{"theme":"Nord","analyzer":true}"#).unwrap();
    assert_eq!(old.visual, Visual::Spectrum);
    assert_eq!(old.theme, "Nord");
}

#[test]
fn the_screen_keeps_animating_for_the_trace_and_rests_when_it_is_still() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.tick();
        if app.engine.caught_up(&app.status) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the engine never caught up");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.run(Action::ToggleVisual(Visual::Trace));
    let quick = std::time::Duration::from_millis(33);
    assert_eq!(app.frame_interval(), std::time::Duration::from_millis(500));
    let tap = app.tap.clone();
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    run_trace(app.visuals.picture(Visual::Trace), &tap, (85, 13), 1, 0.0, 1.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
    assert_eq!(app.frame_interval(), quick);
    run_trace_silent(app.visuals.picture(Visual::Trace), &tap, (85, 13), 1, 4.0, &mut clock);
    assert_eq!(app.frame_interval(), std::time::Duration::from_millis(500));
    app.run(Action::ToggleVisual(Visual::Spectrum));
    run_trace(app.visuals.picture(Visual::Trace), &tap, (85, 13), 1, 0.0, 1.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
    assert!(app.visuals.picture(Visual::Trace).is_active());
    assert_eq!(app.frame_interval(), std::time::Duration::from_millis(500));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_same_song_always_gets_the_same_trace_seed() {
    let _g = serial();
    let root = library();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let mut app = App::new(Some(root.join("Music")));
    assert_eq!(app.trace_seed(), 0);
    app.status.state = crate::audio::State::Playing;
    app.status.duration = 0.5;
    app.status.queue = std::sync::Arc::new(vec![album.join("02 - I Don't Wanna Hear It.wav")]);
    let first = app.trace_seed();
    assert_ne!(first, 0);
    assert_eq!(first, app.trace_seed());
    app.status.queue = std::sync::Arc::new(vec![album.join("10 - Seeing Red.wav")]);
    app.status.index = 0;
    let second = app.trace_seed();
    let copy = root.join("a_copy.wav");
    fs::copy(album.join("02 - I Don't Wanna Hear It.wav"), &copy).unwrap();
    app.status.queue = std::sync::Arc::new(vec![copy]);
    assert_eq!(app.trace_seed(), first);
    assert_eq!(first, crate::viz::song_seed(fs::metadata(album.join("02 - I Don't Wanna Hear It.wav")).unwrap().len(), 500));
    app.status.state = crate::audio::State::Stopped;
    assert_eq!(app.trace_seed(), first);
    let _ = second;
}

#[test]
fn the_trace_slot_has_the_analyzers_size_and_takes_nothing_from_the_folders() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let pane_y = |app: &App| app.hits.iter().find(|h| matches!(h.target, Target::Column(_))).unwrap().rect.y;
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    for (w, h) in [(80u16, 24u16), (110, 30), (160, 48)] {
        app.settings.visual = Visual::Spectrum;
        draw_at(&mut app, w, h);
        let with_spectrum = pane_y(&app);
        app.settings.visual = Visual::Trace;
        draw_at(&mut app, w, h);
        assert_eq!(pane_y(&app), with_spectrum, "{w}x{h}: the folders moved");
        let buf = draw_at(&mut app, w, h);
        let rows = ui::analyzer_rows(h, false);
        let top = 1 + ui::TRANSPORT_H;
        let flat = theme::THEMES[app.theme].trace_colors().at(crate::viz::BG_MIN);
        for y in top..top + rows {
            for x in 0..w {
                let c = &buf[(x, y)];
                assert!(c.symbol() == " " && c.fg == flat && c.bg == flat, "{w}x{h}: {x},{y} of the slot is not the solid color: {c:?}");
            }
        }
        let tap = app.tap.clone();
        run_trace(app.visuals.picture(Visual::Trace), &tap, (w as usize, rows), 0, 0.0, 3.0, trace_music, &mut clock);
        let buf = draw_at(&mut app, w, h);
        let middle = top + (rows - 1) / 2;
        let needles = |y: u16| (0..w).filter(|&x| buf[(x, y)].symbol() == "|").count();
        assert!(needles(middle) > w as usize / 8, "{w}x{h}: only {} needles on the middle row of the slot", needles(middle));
        assert!(needles(middle) >= needles(middle - 1), "{w}x{h}: the middle row has fewer needles than the one above it");
        run_trace(app.visuals.picture(Visual::Trace), &tap, (w as usize, rows), 0, 3.0, 4.0, |_| 0.0, &mut clock);
        let buf = draw_at(&mut app, w, h);
        assert!((top..top + rows).all(|y| (0..w).all(|x| buf[(x, y)].symbol() == " " && buf[(x, y)].bg == flat)), "{w}x{h}: the slot did not settle to the solid color");
    }
}

#[test]
#[ignore]
fn trace_text_dump() {
    for (columns, rows) in [(83usize, 5u16), (83, 9), (83, 13), (100, 21)] {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for second in 0..4 {
            run_trace(&mut trace, &tap, (columns, rows), 7, second as f32 * 2.0, 2.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            println!("--- {columns}x{rows} at {}s", second * 2 + 2);
            for row in glyphs(&painted(&mut trace, dracula(), (columns, rows))) {
                println!("{row}");
            }
        }
    }
}

#[test]
#[ignore]
fn trace_parity_dump() {
    use std::io::Write;
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (columns, rows) = (env("TRACE_COLS", "83").parse::<usize>().unwrap(), env("TRACE_ROWS", "13").parse::<u16>().unwrap());
    let seed: u64 = env("TRACE_SEED", "7").parse().unwrap();
    let theme = theme::index_of(&env("TRACE_THEME", "Dracula")).unwrap();
    let out = env("TRACE_OUT", "/tmp/trace_parity");
    fs::create_dir_all(&out).unwrap();
    let bytes = fs::read(env("TRACE_AUDIO", "")).unwrap();
    let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();

    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut amps = fs::File::create(format!("{out}/rust_amps.csv")).unwrap();
    let mut flash = fs::File::create(format!("{out}/rust_flash.csv")).unwrap();
    let frames = audio.len() / 1600;
    for f in 0..frames {
        let mut w = tap.writer();
        for &s in &audio[f * 1600..(f + 1) * 1600] {
            w.push(s);
        }
        drop(w);
        clock += std::time::Duration::from_secs_f64(1.0 / 30.0);
        trace.update(&tap, columns, rows, seed, clock);
        let row: Vec<String> = trace.amps().iter().map(|a| format!("{a:.5}")).collect();
        writeln!(amps, "{}", row.join(",")).unwrap();
        writeln!(flash, "{:.5}", trace.flash()).unwrap();
        if f % 30 == 29 {
            let pic = painted(&mut trace, theme, (columns, rows));
            fs::write(format!("{out}/rust_frame_{f:04}.txt"), glyphs(&pic).join("\n")).unwrap();
        }
    }
    println!("{frames} frames of {columns}x{rows} written to {out}");
}

#[test]
#[ignore]
fn trace_cost_report() {
    use ratatui::backend::CrosstermBackend;
    use ratatui::{TerminalOptions, Viewport};
    use std::cell::Cell;
    use std::io::Write;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    struct Counter(Rc<Cell<usize>>);
    impl Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.set(self.0.get() + b.len());
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let _g = serial();
    let root = library();
    let frames = 150usize;
    let sizes: Vec<(u16, u16)> = std::env::var("TRACE_COST_SIZES")
        .unwrap_or_else(|_| "80x24,120x40,200x60".into())
        .split(',')
        .map(|s| {
            let (w, h) = s.split_once('x').unwrap();
            (w.parse().unwrap(), h.parse().unwrap())
        })
        .collect();
    let wanted = std::env::var("TRACE_COST_VISUALS").unwrap_or_else(|_| "off,spectrum,trace".into());
    for (w, h) in sizes {
        println!("--- window {w}x{h}, analyzer slot {} rows", ui::analyzer_rows(h, false));
        println!("{:<10} {:>12} {:>12} {:>12} {:>14} {:>12}", "visual", "bytes/frame", "p95 bytes", "cells/frame", "draw us (mean)", "KB/s at 30");
        let mut base_bytes = 0.0;
        for visual in ["off", "spectrum", "trace"] {
            if !wanted.split(',').any(|v| v == visual) {
                continue;
            }
            let mut app = App::new(Some(root.join("Music")));
            app.settings.analyzer = visual != "off";
            app.settings.visual = if visual == "trace" { Visual::Trace } else { Visual::Spectrum };
            app.status.state = crate::audio::State::Playing;
            app.status.duration = 200.0;
            let sent = Rc::new(Cell::new(0usize));
            let backend = CrosstermBackend::new(Counter(sent.clone()));
            let mut term = Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Fixed(ratatui::layout::Rect::new(0, 0, w, h)) }).unwrap();
            let mut previous: Option<ratatui::buffer::Buffer> = None;
            let (mut per_frame, mut cells, mut spent) = (Vec::new(), Vec::new(), Duration::ZERO);
            for f in 0..frames + 30 {
                let mut wr = app.tap.writer();
                for i in 0..1600 {
                    wr.push((synth_music((f * 1600 + i) as f32 / 48_000.0) * 2.2).clamp(-1.0, 1.0));
                }
                drop(wr);
                std::thread::sleep(Duration::from_millis(33));
                let before = sent.get();
                let t0 = Instant::now();
                let mut snapshot = None;
                term.draw(|fr| {
                    ui::draw(fr, &mut app);
                    snapshot = Some(fr.buffer_mut().clone());
                })
                .unwrap();
                let took = t0.elapsed();
                let snapshot = snapshot.unwrap();
                if f >= 30 {
                    per_frame.push(sent.get() - before);
                    cells.push(previous.as_ref().map_or(0, |p| p.diff(&snapshot).len()));
                    spent += took;
                }
                previous = Some(snapshot);
            }
            let mean = per_frame.iter().sum::<usize>() as f64 / frames as f64;
            let mut sorted = per_frame.clone();
            sorted.sort_unstable();
            let p95 = sorted[frames * 95 / 100];
            let cells_mean = cells.iter().sum::<usize>() as f64 / frames as f64;
            if visual == "off" {
                base_bytes = mean;
            }
            println!(
                "{:<10} {:>12.0} {:>12} {:>12.0} {:>14.0} {:>12.1}   (visual alone: {:+.0} bytes/frame)",
                visual,
                mean,
                p95,
                cells_mean,
                spent.as_secs_f64() * 1e6 / frames as f64,
                mean * 30.0 / 1024.0,
                mean - base_bytes
            );
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore]
fn trace_churn_report() {
    for (columns, rows) in [(80usize, 9u16), (120, 14), (200, 21)] {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut prev: Option<Buffer> = None;
        let (mut glyph, mut fg_only, mut bg_only, mut both, mut frames) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let (mut line_cells, mut needle_cells, mut empty_cells) = (0usize, 0usize, 0usize);
        let line = (rows as usize - 1) / 2;
        for f in 0..240 {
            run_trace(&mut trace, &tap, (columns, rows), 3, f as f32 / 30.0, 1.0 / 30.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            let now = painted(&mut trace, dracula(), (columns, rows));
            if let (Some(p), true) = (&prev, f >= 60) {
                frames += 1;
                for y in 0..rows {
                    for x in 0..columns as u16 {
                        let (a, b) = (&p[(x, y)], &now[(x, y)]);
                        let (g, fg, bg) = (a.symbol() != b.symbol(), a.fg != b.fg, a.bg != b.bg);
                        let fg = fg && b.symbol() != " ";
                        if g || fg || bg {
                            if g { glyph += 1 } else if fg && bg { both += 1 } else if fg { fg_only += 1 } else { bg_only += 1 }
                            if y as usize == line { line_cells += 1 } else if b.symbol() != " " || a.symbol() != " " { needle_cells += 1 } else { empty_cells += 1 }
                        }
                    }
                }
            }
            prev = Some(now);
        }
        let n = frames as f64;
        let total = (glyph + fg_only + bg_only + both) as f64 / n;
        println!("{columns}x{rows} ({} cells): {total:.0} changed cells per frame", columns * rows as usize);
        println!("   glyph changed {:.0}, color only: fg {:.0}, bg {:.0}, both {:.0}", glyph as f64 / n, fg_only as f64 / n, bg_only as f64 / n, both as f64 / n);
        println!("   on the middle row {:.0}, on or at the end of a needle {:.0}, blank cells whose glow changed {:.0}", line_cells as f64 / n, needle_cells as f64 / n, empty_cells as f64 / n);
    }
}

#[test]
#[ignore]
fn trace_cells_dump() {
    use ratatui::style::Color;
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (columns, rows) = (env("TRACE_COLS", "83").parse::<usize>().unwrap(), env("TRACE_ROWS", "13").parse::<u16>().unwrap());
    let seed: u64 = env("TRACE_SEED", "7").parse().unwrap();
    let out = env("TRACE_OUT", "/tmp/trace_cells");
    fs::create_dir_all(&out).unwrap();
    let bytes = fs::read(env("TRACE_AUDIO", "")).unwrap();
    let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let names: Vec<String> = env("TRACE_THEMES", "Dracula").split(',').map(|s| s.trim().to_string()).collect();
    let moments: Vec<f32> = env("TRACE_AT", "5.0").split(',').map(|s| s.trim().parse().unwrap()).collect();
    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        other => panic!("not a plain color: {other:?}"),
    };

    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let frames = audio.len() / 1600;
    for f in 0..frames {
        let mut w = tap.writer();
        for &s in &audio[f * 1600..(f + 1) * 1600] {
            w.push(s);
        }
        drop(w);
        clock += std::time::Duration::from_secs_f64(1.0 / 30.0);
        trace.update(&tap, columns, rows, seed, clock);
        let first = theme::index_of(&names[0]).unwrap();
        let _ = painted(&mut trace, first, (columns, rows));
        let now = (f + 1) as f32 / 30.0;
        if let Ok(each) = std::env::var("TRACE_CYCLE") {
            let index = (f as f32 / 30.0 / each.parse::<f32>().unwrap()) as usize % names.len();
            let pic = painted(&mut trace, theme::index_of(&names[index]).unwrap(), (columns, rows));
            let mut text = format!("{columns} {rows}\n");
            for y in 0..rows {
                for x in 0..columns as u16 {
                    let c = &pic[(x, y)];
                    text.push_str(c.symbol());
                    text.push_str(&hex(c.fg));
                    text.push_str(&hex(c.bg));
                }
                text.push('\n');
            }
            fs::write(format!("{out}/frame_{f:05}.cells"), text).unwrap();
            let mut names_file = fs::OpenOptions::new().create(true).append(true).open(format!("{out}/captions.txt")).unwrap();
            std::io::Write::write_all(&mut names_file, format!("{}\n", names[index]).as_bytes()).unwrap();
            continue;
        }
        for &m in &moments {
            if (now - m).abs() < 0.5 / 30.0 {
                for name in &names {
                    let Some(index) = theme::index_of(name) else { panic!("no theme called {name}") };
                    let pic = painted(&mut trace, index, (columns, rows));
                    let mut text = format!("{columns} {rows}\n");
                    for y in 0..rows {
                        for x in 0..columns as u16 {
                            let c = &pic[(x, y)];
                            text.push_str(c.symbol());
                            text.push_str(&hex(c.fg));
                            text.push_str(&hex(c.bg));
                        }
                        text.push('\n');
                    }
                    let file = format!("{out}/{}_{:.2}s.cells", name.replace(' ', "_"), m);
                    fs::write(file, text).unwrap();
                }
            }
        }
    }
}

#[test]
fn trace_repaints_the_same_picture_until_the_music_moves_it_on() {
    let size = (85, 13);
    let (tap, mut trace) = (Tap::new(), Trace::new());
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
    for f in 0..90usize {
        run_trace(&mut trace, &tap, size, 1, f as f32 / 30.0, 1.0 / 30.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
        let first = painted(&mut trace, dracula(), size);
        for _ in 0..3 {
            assert_eq!(painted(&mut trace, dracula(), size), first, "frame {f}: a repaint changed the picture");
        }
    }
}

#[test]
fn trace_changes_a_modest_share_of_the_cells_from_one_frame_to_the_next() {
    for size in [(80usize, 9u16), (120, 14), (200, 21)] {
        let (tap, mut trace) = (Tap::new(), Trace::new());
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut prev, mut changed, mut frames) = (None::<Buffer>, 0usize, 0usize);
        for f in 0..240usize {
            run_trace(&mut trace, &tap, size, 3, f as f32 / 30.0, 1.0 / 30.0, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);
            let now = painted(&mut trace, dracula(), size);
            if let (Some(p), true) = (&prev, f >= 60) {
                changed += p.diff(&now).len();
                frames += 1;
            }
            prev = Some(now);
        }
        let share = changed as f64 / frames as f64 / (size.0 * size.1 as usize) as f64;
        assert!(share < 0.20, "{}x{}: {:.1}% of the cells change in a frame", size.0, size.1, share * 100.0);
    }
}

fn playlist_files(root: &Path) -> PathBuf {
    let lists = root.join("lists");
    fs::create_dir_all(&lists).unwrap();
    let song = root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav");
    for n in ["one", "two", "three"] {
        fs::write(lists.join(format!("{n}.m3u")), format!("#EXTM3U\n#EXTINF:1,x\n{}\n", song.display())).unwrap();
    }
    lists
}

fn open_import_at(app: &mut App, dir: &Path) {
    app.run(Action::ImportPlaylist);
    assert!(matches!(&app.overlay, Some(Overlay::Open(d)) if d.mode == crate::app::OpenMode::ImportPlaylists));
    render(app);
    click(app, |t| *t == Target::OpenPath);
    if let Some(Overlay::Open(d)) = app.overlay.as_mut() {
        d.path = dir.to_string_lossy().to_string();
    }
    key(app, KeyCode::Enter);
    render(app);
}

fn checked_names(app: &App) -> Vec<String> {
    match &app.overlay {
        Some(Overlay::Open(d)) => d.checked.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn import_playlist_check_marks_enter_and_double_click() {
    let _g = serial();
    let root = library();
    let lists = playlist_files(&root);
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    let items = app.file_menu_items();
    assert_eq!(items[0].action, Some(Action::OpenFolder));
    assert_eq!(items[1].action, Some(Action::NewPlaylist));
    assert_eq!(items[1].label, "New Playlist…");
    assert_eq!(items[2].action, Some(Action::ImportPlaylist));
    assert_eq!(items[2].label, "Add New Playlist…");

    open_import_at(&mut app, &lists);
    let screen = render(&mut app);
    assert!(screen.contains("File › Add New Playlist"), "{screen}");
    assert!(screen.contains("one.m3u") && screen.contains("three.m3u") && screen.contains("two.m3u"), "{screen}");

    click(&mut app, |t| *t == Target::OpenRow(1));
    assert_eq!(checked_names(&app), ["one.m3u"]);
    assert!(render(&mut app).contains("✓ one.m3u"));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(checked_names(&app), ["one.m3u", "three.m3u"]);
    if let Some(Overlay::Open(d)) = &app.overlay {
        assert_eq!(d.selected, 2, "Space must not move down");
    }
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(checked_names(&app), ["one.m3u"]);
    key(&mut app, KeyCode::Char(' '));

    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    let library = crate::playlists::dir();
    let names: Vec<String> = crate::playlists::list_in(&library).iter().map(|p| crate::playlists::name_of(p)).collect();
    assert_eq!(names, ["one", "three"]);
    let view = app.playlists.as_ref().expect("the Playlists view opens");
    assert_eq!(view.names, ["one", "three"]);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("imported 2 playlists")));

    app.playlists = None;
    open_import_at(&mut app, &lists);
    click(&mut app, |t| *t == Target::OpenRow(1));
    let r = rect_of(&app, |t| *t == Target::OpenRow(3));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Left);
    click_at(&mut app, r.x + 1, r.y, MouseButton::Left);
    assert!(app.overlay.is_none());
    let names: Vec<String> = crate::playlists::list_in(&library).iter().map(|p| crate::playlists::name_of(p)).collect();
    assert_eq!(names, ["one", "three", "two"]);
    assert_eq!(app.playlists.as_ref().unwrap().names[app.playlists.as_ref().unwrap().selected], "two");

    app.playlists = None;
    open_import_at(&mut app, &lists);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(crate::playlists::list_in(&library).iter().any(|p| crate::playlists::name_of(p) == "one_2"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn playback_menu_has_playlists_on_top_and_the_view_plays() {
    let _g = serial();
    let root = library();
    let lists = playlist_files(&root);
    let library = crate::playlists::dir();
    for n in ["one", "two"] {
        crate::playlists::import_into(&lists.join(format!("{n}.m3u")), &library).unwrap();
    }
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    let items = app.menu_items(1);
    assert_eq!(items[0].action, Some(Action::Playlists));
    assert_eq!(items[1].action, Some(Action::Queue));
    assert_eq!(items[2].action, Some(Action::Search));
    assert_eq!(items[3].action, Some(Action::Library));
    assert!(items[4].is_separator());

    click(&mut app, |t| *t == Target::Menu(1));
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(0));
    let screen = render(&mut app);
    assert!(app.playlists.is_some());
    assert!(screen.contains("one") && screen.contains("two") && screen.contains("01 - Filler"), "{screen}");

    let plays = app.engine.plays_sent();
    key(&mut app, KeyCode::Right);
    assert!(app.playlists.as_ref().unwrap().on_songs);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.engine.plays_sent(), plays + 1);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.playlists.as_ref().unwrap().selected, 1);
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::PlaylistRow(0));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Left);
    click_at(&mut app, r.x + 1, r.y, MouseButton::Left);
    assert_eq!(app.engine.plays_sent(), plays + 2);

    key(&mut app, KeyCode::Esc);
    assert!(app.playlists.is_none());
    app.run(Action::Playlists);
    assert!(app.playlists.is_some());
    assert_eq!(app.menu_items(1)[0].checked, Some(true));
    app.run(Action::Playlists);
    assert!(app.playlists.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn new_playlist_asks_for_a_name_and_makes_it() {
    let _g = serial();
    let root = library();
    let lib = crate::playlists::dir();
    assert!(lib.ends_with("config/urchin/playlists"), "{lib:?}");
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    let items = app.file_menu_items();
    assert_eq!(items[1].label, "New Playlist…");
    assert_eq!(items[1].action, Some(Action::NewPlaylist));

    app.run(Action::Playlists);
    let screen = render(&mut app);
    assert!(screen.contains("No Playlists Yet") && screen.contains("File › Add New Playlist"), "{screen}");
    app.run(Action::Playlists);

    app.run(Action::NewPlaylist);
    let screen = render(&mut app);
    assert!(screen.contains("File › New Playlist") && screen.contains("Create"), "{screen}");
    key(&mut app, KeyCode::Enter);
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(..))));
    assert!(!lib.exists() || crate::playlists::list_in(&lib).is_empty());

    for c in "Mix q!x".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    assert!(render(&mut app).contains("Mix q▌"));
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert!(!app.quit, "typing q in the box must not quit");
    assert_eq!(fs::read_to_string(lib.join("Mix q.m3u8")).unwrap(), "#EXTM3U\n");
    let v = app.playlists.as_ref().expect("the Playlists view shows the new playlist");
    assert_eq!(v.names, ["Mix q"]);

    app.run(Action::NewPlaylist);
    for c in "Mix q".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    render(&mut app);
    click(&mut app, |t| *t == Target::PlaylistNameOk);
    assert!(app.overlay.is_none());
    assert!(lib.join("Mix q_2.m3u8").is_file());
    app.run(Action::NewPlaylist);
    key(&mut app, KeyCode::Char('z'));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(crate::playlists::list_in(&lib).len(), 2);

    app.run(Action::NewPlaylist);
    render(&mut app);
    click(&mut app, |t| *t == Target::PlaylistNameCancel);
    assert!(app.overlay.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn right_click_a_playlist_to_rename_or_delete_it() {
    let _g = serial();
    let root = library();
    let lib = crate::playlists::dir();
    let a = crate::playlists::create_in("alpha", &lib).unwrap();
    let b = crate::playlists::create_in("beta", &lib).unwrap();
    let mut app = App::new(Some(root.join("Music")));
    app.run(Action::Playlists);
    render(&mut app);
    let labels = |app: &App| -> Vec<String> { app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect() };

    let r = rect_of(&app, |t| *t == Target::PlaylistRow(0));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "", "Rename", "Delete", "", crate::reveal::label(), "Copy Path"]);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(4));
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(n, NameFor::Rename(p))) if n == "alpha" && *p == a));
    let screen = render(&mut app);
    assert!(screen.contains("Rename Playlist") && screen.contains("alpha▌") && screen.contains("Rename"), "{screen}");

    for _ in 0..5 {
        key(&mut app, KeyCode::Backspace);
    }
    for c in "beta".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(..))));
    assert!(a.is_file() && b.is_file());

    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    for c in "gamma".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert!(!a.exists() && lib.join("gamma.m3u8").is_file());
    let v = app.playlists.as_ref().unwrap();
    assert_eq!(v.names, ["beta", "gamma"]);
    assert_eq!(v.names[v.selected], "gamma");

    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::PlaylistRow(1));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(5));
    assert!(!lib.join("gamma.m3u8").exists());
    let v = app.playlists.as_ref().unwrap();
    assert_eq!(v.names, ["beta"]);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "deleted gamma"));

    fs::write(&b, format!("#EXTM3U\n{}\n", root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav").display())).unwrap();
    app.run(Action::Playlists);
    app.run(Action::Playlists);
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::PlaylistSong(0));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "", crate::reveal::label(), "Copy Path"]);

    key(&mut app, KeyCode::Esc);
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::PlaylistRow(0));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(5));
    let screen = render(&mut app);
    assert!(screen.contains("No Playlists Yet"), "{screen}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn playlists_work_with_the_keyboard_alone() {
    let _g = serial();
    let root = library();
    let lib = crate::playlists::dir();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);
    render(&mut app);
    let labels = |app: &App| -> Vec<String> { app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect() };
    let type_text = |app: &mut App, t: &str| {
        for c in t.chars() {
            key(app, KeyCode::Char(c));
        }
    };

    key(&mut app, KeyCode::F(10));
    render(&mut app);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.menu.as_ref().unwrap().hover, Some(1));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(n, NameFor::New)) if n.is_empty()));
    type_text(&mut app, "mm one");
    key(&mut app, KeyCode::Enter);
    assert!(lib.join("mm one.m3u8").is_file());
    assert!(app.playlists.is_some());

    key(&mut app, KeyCode::F(2));
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(n, NameFor::Rename(_))) if n == "mm one"));
    for _ in 0..3 {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "two");
    key(&mut app, KeyCode::Enter);
    assert!(lib.join("mm two.m3u8").is_file() && !lib.join("mm one.m3u8").exists());

    render(&mut app);
    key(&mut app, KeyCode::Char('m'));
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "", "Rename", "Delete", "", crate::reveal::label(), "Copy Path"]);
    assert_eq!(app.menu.as_ref().unwrap().hover, Some(0));
    key(&mut app, KeyCode::Down);
    assert_eq!(app.menu.as_ref().unwrap().hover, Some(1));
    key(&mut app, KeyCode::Esc);
    assert!(app.menu.is_none() && app.playlists.is_some());

    key(&mut app, KeyCode::Delete);
    assert!(!lib.join("mm two.m3u8").exists());
    assert!(render(&mut app).contains("No Playlists Yet"));
    key(&mut app, KeyCode::Esc);
    assert!(app.playlists.is_none());

    crate::playlists::create_in("kb", &lib).unwrap();
    let row = row_named(&app, app.focus, "02 - I Don't Wanna Hear It.wav");
    render(&mut app);
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    render(&mut app);
    key(&mut app, KeyCode::Char('m'));
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "Add to Playlist", "", "Move Up", "Move Down", "", crate::reveal::label(), "Copy Path"]);
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(labels(&app), ["kb"]);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    let text = fs::read_to_string(lib.join("kb.m3u8")).unwrap();
    assert!(text.contains("02 - I Don't Wanna Hear It.wav"), "{text}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn right_click_a_song_to_add_it_to_a_playlist() {
    let _g = serial();
    let root = library();
    let lists = playlist_files(&root);
    let library = crate::playlists::dir();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);
    render(&mut app);
    let labels = |app: &App| -> Vec<String> { app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect() };
    let right_click_row = |app: &mut App, name: &str| {
        let row = row_named(app, app.focus, name);
        render(app);
        let col = app.focus;
        let r = rect_of(app, |t| *t == Target::Row { col, row });
        click_at(app, r.x + 1, r.y, MouseButton::Right);
    };

    right_click_row(&mut app, "02 - I Don't Wanna Hear It.wav");
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "Add to Playlist", "", "Move Up", "Move Down", "", crate::reveal::label(), "Copy Path"]);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(3));
    assert_eq!(labels(&app), ["No Playlists Yet"]);
    key(&mut app, KeyCode::Esc);

    app.navigate_to(&root.join("Music/punk"));
    right_click_row(&mut app, "hardcore");
    assert_eq!(labels(&app), ["Shuffle", "Play Next", "Add to Queue", "Open", "Pin", "", crate::reveal::label(), "Copy Path"]);
    key(&mut app, KeyCode::Esc);
    app.navigate_to(&album);

    crate::playlists::import_into(&lists.join("one.m3u"), &library).unwrap();
    let one = library.join("one.m3u8");
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    right_click_row(&mut app, "02 - I Don't Wanna Hear It.wav");
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(3));
    assert_eq!(labels(&app), ["one"]);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(0));
    assert!(app.menu.is_none());
    let text = fs::read_to_string(&one).unwrap();
    assert!(text.trim_end().ends_with(&song.display().to_string()), "{text}");
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("added") && m.contains("one")));

    right_click_row(&mut app, "02 - I Don't Wanna Hear It.wav");
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(3));
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(0));
    assert_eq!(fs::read_to_string(&one).unwrap(), text);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("already in one")));

    let third = row_named(&app, app.focus, "10 - Seeing Red.wav");
    let col = app.focus;
    app.hits.clear();
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col, row: third });
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(labels(&app), ["one"]);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(fs::read_to_string(&one).unwrap().contains("10 - Seeing Red.wav"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn synthwave_keeps_its_colors_whichever_theme_is_in_use() {
    use crate::settings::Settings;
    use ratatui::style::Color;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let item = |app: &App, action: Action| app.menu_items(2).into_iter().find(|i| i.action == Some(action.clone())).unwrap();
    let nord = theme::index_of("Nord").unwrap();
    app.theme = nord;
    app.settings.theme = "Nord".into();
    let rgb = |c: u32| Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);

    assert_eq!(item(&app, Action::ToggleVisual(Visual::Synthwave)).label, "Synthwave");
    app.run(Action::ToggleVisual(Visual::Synthwave));
    assert_eq!((app.theme, app.settings.visual), (nord, Visual::Synthwave));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Synthwave)).checked, Some(true));
    assert_eq!(Settings::load().theme, "Nord");
    assert_eq!(Settings::load().visual, Visual::Synthwave);

    let own = rgb(theme::synthwave_colors().ground);
    let mut seen = Vec::new();
    for name in ["Nord", "Dracula", "Solarized Dark Higher Contrast"] {
        let i = theme::index_of(name).unwrap();
        app.theme = i;
        app.settings.theme = name.into();
        let buf = draw_at(&mut app, 110, 34);
        let flat = buf.content.iter().filter(|cell| cell.bg == own).count();
        assert!(flat > 400, "{name}: only {flat} cells of Synthwave's own ground color");
        assert!(buf.content.iter().any(|cell| cell.bg != own), "{name}: the window should keep the theme's colors");
        assert_eq!(app.theme, i);
        seen.push(flat);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "Synthwave changed with the theme: {seen:?}");

    app.run(Action::ToggleVisual(Visual::Synthwave));
    assert!(!app.settings.analyzer);
    app.run(Action::ToggleVisual(Visual::Synthwave));
    app.run(Action::ToggleVisual(Visual::Trace));
    app.run(Action::ToggleVisual(Visual::Bokeh));
    assert_eq!(theme::THEMES[app.theme].name, Settings::load().theme);

    let old: Settings = serde_json::from_str(r#"{"theme":"Nord","visual":"synthwave","theme_before_synthwave":"Nord"}"#).unwrap();
    assert_eq!(old.visual, Visual::Synthwave);
    let own_name: Settings = serde_json::from_str(r#"{"visual":"synthwave_urchin"}"#).unwrap();
    assert_eq!(own_name.visual, Visual::Synthwave);
}

#[test]
fn synthmoonwave_keeps_its_colors_whichever_theme_is_in_use() {
    use crate::settings::Settings;
    use ratatui::style::Color;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let item = |app: &App, action: Action| app.menu_items(2).into_iter().find(|i| i.action == Some(action.clone())).unwrap();
    let nord = theme::index_of("Nord").unwrap();
    app.theme = nord;
    app.settings.theme = "Nord".into();
    let rgb = |c: u32| Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);

    assert_eq!(item(&app, Action::ToggleVisual(Visual::SynthMoonWave)).label, "SynthMoonWave");
    app.run(Action::ToggleVisual(Visual::SynthMoonWave));
    assert_eq!((app.theme, app.settings.visual), (nord, Visual::SynthMoonWave));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::SynthMoonWave)).checked, Some(true));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Synthwave)).checked, Some(false));
    assert_eq!(Settings::load().visual, Visual::SynthMoonWave);
    assert_eq!(Settings::load().theme, "Nord");

    let own = rgb(theme::synthmoonwave_colors().ground);
    assert_ne!(own, rgb(theme::synthwave_colors().ground));
    let mut seen = Vec::new();
    for name in ["Nord", "Dracula", "Solarized Dark Higher Contrast"] {
        let i = theme::index_of(name).unwrap();
        app.theme = i;
        app.settings.theme = name.into();
        let buf = draw_at(&mut app, 110, 34);
        let flat = buf.content.iter().filter(|cell| cell.bg == own).count();
        assert!(flat > 400, "{name}: only {flat} cells of SynthMoonWave's own ground color");
        seen.push(flat);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "SynthMoonWave changed with the theme: {seen:?}");

    app.run(Action::ToggleVisual(Visual::Synthwave));
    let buf = draw_at(&mut app, 110, 34);
    assert!(buf.content.iter().filter(|cell| cell.bg == rgb(theme::synthwave_colors().ground)).count() > 400);
    assert_eq!(Visual::from_name("synthmoonwave"), Visual::SynthMoonWave);
    assert_eq!(Visual::SynthMoonWave.name(), "synthmoonwave");
}

#[test]
#[ignore]
fn synthmoonwave_cells_dump() {
    use ratatui::style::{Color, Modifier};
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (w, h) = (env("MW_COLS", "90").parse::<u16>().unwrap(), env("MW_ROWS", "30").parse::<u16>().unwrap());
    let out = env("MW_OUT", "/tmp/synthmoonwave_cells");
    let visual = Visual::from_name(&env("MW_VISUAL", "synthmoonwave"));
    let theme_name = env("MW_THEME", "Gilded-Forest-Dark");
    let secs: f32 = env("MW_SECS", "6").parse().unwrap();
    let stills: Vec<f32> = env("MW_STILLS", "").split(',').filter(|s| !s.trim().is_empty()).map(|s| s.trim().parse().unwrap()).collect();
    let _g = serial();
    let root = std::env::temp_dir().join(format!("urchin_moonwave_render_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    std::env::set_var("HOME", &root);
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let album = root.join("Music/Electronic/The Midnight/Endless Summer");
    fs::create_dir_all(&album).unwrap();
    for song in ["Endless Summer", "Sunset", "Vampires", "Gloria"] {
        write_wav(&album.join(format!("The Midnight - {song}.wav")), 44_100, 0.25);
    }
    fs::create_dir_all(&out).unwrap();
    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        other => panic!("not a plain color: {other:?}"),
    };
    let dump = |buf: &ratatui::buffer::Buffer, file: String| {
        let mut text = format!("{} {}\n", buf.area.width, buf.area.height);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                assert_eq!(c.symbol().chars().count(), 1, "cell {x},{y} holds {:?}", c.symbol());
                text.push_str(c.symbol());
                text.push_str(&hex(c.fg));
                text.push_str(&hex(c.bg));
                text.push(if c.modifier.contains(Modifier::BOLD) { '1' } else { '0' });
            }
            text.push('\n');
        }
        fs::write(file, text).unwrap();
    };
    let playing = |app: &mut App| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![album.join("The Midnight - Vampires.wav")]);
        app.status.meta.title = "Vampires".into();
        app.status.meta.artist = "The Midnight".into();
        app.status.position = 71.0;
        app.status.duration = 342.0;
        app.status.error = None;
        app.toast = None;
    };
    let index = theme::index_of(&theme_name).or_else(|| theme::index_of(&theme_name.replace('-', " "))).unwrap_or_else(|| panic!("no theme called {theme_name}"));
    let mut app = App::new(Some(root.join("Music")));
    app.theme = index;
    app.tap.set_rate(48_000);
    app.navigate_to(&album);
    app.run(Action::ToggleVisual(visual));
    playing(&mut app);
    draw_at(&mut app, w, h);
    let (cols, rows) = (app.analyzer_rect.width as usize, app.analyzer_rect.height);
    let tap = app.tap.clone();
    let song: Option<Vec<f32>> = std::env::var("MW_SONG").ok().map(|p| fs::read(p).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect());
    let music = |t: f32| match &song {
        Some(s) => s.get((t * 48_000.0) as usize).copied().unwrap_or(0.0),
        None => (synth_music(t) * 2.2).clamp(-1.0, 1.0),
    };
    let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    let mut audio: Vec<f32> = Vec::new();
    let frames = (secs * 30.0) as usize;
    let mut next = 0;
    for f in 0..frames {
        let t = f as f32 / 30.0;
        let mut wr = tap.writer();
        for i in 0..1600 {
            let v = music(t + i as f32 / 48_000.0);
            wr.push(v);
            audio.push(v);
        }
        drop(wr);
        clock += std::time::Duration::from_millis(33);
        let seed = app.trace_seed();
        app.visuals.picture(visual).update(&tap, cols, rows, seed, clock);
        playing(&mut app);
        if stills.is_empty() {
            let buf = draw_at(&mut app, w, h);
            dump(&buf, format!("{out}/frame_{f:05}.window"));
        } else if next < stills.len() && t + 1e-3 >= stills[next] {
            let buf = draw_at(&mut app, w, h);
            dump(&buf, format!("{out}/still_{next}.window"));
            next += 1;
        }
    }
    let bytes: Vec<u8> = audio.iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(format!("{out}/song.f32"), bytes).unwrap();
    println!("slot {cols}x{rows}, {frames} frames, seed {}", app.trace_seed());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn galaxy_keeps_its_colors_whichever_theme_is_in_use() {
    use crate::settings::Settings;
    use ratatui::style::Color;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let item = |app: &App, action: Action| app.menu_items(2).into_iter().find(|i| i.action == Some(action.clone())).unwrap();
    let nord = theme::index_of("Nord").unwrap();
    app.theme = nord;
    app.settings.theme = "Nord".into();
    let rgb = |c: u32| Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);

    assert_eq!(item(&app, Action::ToggleVisual(Visual::Galaxy)).label, "Galaxy");
    app.run(Action::ToggleVisual(Visual::Galaxy));
    assert_eq!((app.theme, app.settings.visual), (nord, Visual::Galaxy));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::Galaxy)).checked, Some(true));
    assert_eq!(Settings::load().theme, "Nord");
    assert_eq!(Settings::load().visual, Visual::Galaxy);

    let own = rgb(theme::galaxy_colors().ground);
    let mut seen = Vec::new();
    for name in ["Nord", "Dracula", "Solarized Dark Higher Contrast"] {
        let i = theme::index_of(name).unwrap();
        app.theme = i;
        app.settings.theme = name.into();
        let buf = draw_at(&mut app, 110, 34);
        let flat = buf.content.iter().filter(|cell| cell.bg == own).count();
        assert!(flat > 400, "{name}: only {flat} cells of Galaxy's own ground color");
        assert!(buf.content.iter().any(|cell| cell.bg != own), "{name}: the window should keep the theme's colors");
        assert_eq!(app.theme, i);
        seen.push(flat);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "Galaxy changed with the theme: {seen:?}");

    app.run(Action::ToggleVisual(Visual::Galaxy));
    assert!(!app.settings.analyzer);
    app.run(Action::ToggleVisual(Visual::Bokeh));
    assert_eq!(theme::THEMES[app.theme].name, Settings::load().theme);
    for name in ["galaxy", "galaxy_urchin"] {
        let saved: Settings = serde_json::from_str(&format!(r#"{{"visual":"{name}"}}"#)).unwrap();
        assert_eq!(saved.visual, Visual::Galaxy);
    }
}

#[test]
fn club_lights_keeps_its_colors_whichever_theme_is_in_use() {
    use crate::settings::Settings;
    use ratatui::style::Color;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let item = |app: &App, action: Action| app.menu_items(2).into_iter().find(|i| i.action == Some(action.clone())).unwrap();
    let nord = theme::index_of("Nord").unwrap();
    app.theme = nord;
    app.settings.theme = "Nord".into();
    let own = {
        let c = theme::club_lights_colors().ground;
        Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
    };

    assert_eq!(item(&app, Action::ToggleVisual(Visual::ClubLights)).label, "Club Lights");
    app.run(Action::ToggleVisual(Visual::ClubLights));
    assert_eq!((app.theme, app.settings.visual), (nord, Visual::ClubLights));
    assert_eq!(item(&app, Action::ToggleVisual(Visual::ClubLights)).checked, Some(true));
    assert_eq!(Settings::load().theme, "Nord");
    assert_eq!(Settings::load().visual, Visual::ClubLights);

    let mut seen = Vec::new();
    for name in ["Nord", "Dracula", "Solarized Dark Higher Contrast"] {
        let i = theme::index_of(name).unwrap();
        app.theme = i;
        app.settings.theme = name.into();
        let buf = draw_at(&mut app, 110, 34);
        let flat = buf.content.iter().filter(|cell| cell.bg == own).count();
        assert!(flat > 400, "{name}: only {flat} cells of Club Lights' own ground color");
        assert!(buf.content.iter().any(|cell| cell.bg != own), "{name}: the window should keep the theme's colors");
        assert_eq!(app.theme, i);
        seen.push(flat);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "Club Lights changed with the theme: {seen:?}");

    app.run(Action::ToggleVisual(Visual::ClubLights));
    assert!(!app.settings.analyzer);
    app.run(Action::ToggleVisual(Visual::Bokeh));
    assert_eq!(theme::THEMES[app.theme].name, Settings::load().theme);
    for name in ["club_lights", "Club Lights"] {
        let saved: Settings = serde_json::from_str(&format!(r#"{{"visual":"{name}"}}"#)).unwrap();
        assert_eq!(saved.visual, Visual::ClubLights);
    }
}

#[test]
fn settings_menu_has_a_visualizers_heading_and_short_names() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let items = app.menu_items(2);
    let at = items.iter().position(|i| i.label == "Bokeh").expect("Bokeh item");
    assert_eq!(items[at - 1].label, "[ Visualizers ]");
    assert!(items[at - 1].heading && items[at - 1].action.is_none() && !items[at - 1].is_separator());
    assert!(items[at - 2].is_separator());
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    for l in ["Bokeh", "Lines", "Stereo", "Trace"] {
        assert!(labels.contains(&l), "{labels:?}");
    }
    assert!(!labels.iter().any(|l| l.ends_with("Visualizer")), "{labels:?}");
    let visualizers: Vec<&str> = items[at..].iter().take_while(|i| !i.is_separator()).map(|i| i.label.as_str()).collect();
    let mut sorted = visualizers.clone();
    sorted.sort_by_key(|l| l.to_lowercase());
    assert_eq!(visualizers, sorted);
    assert_eq!(visualizers, ["Bokeh", "Bokeh Light", "Club Lights", "Galaxy", "Lines", "Oscilloscope", "Spectroscope", "Stereo", "SynthMoonWave", "Synthwave", "Trace", "Vectorscope"]);

    render(&mut app);
    click(&mut app, |t| *t == Target::Menu(2));
    let screen = render(&mut app);
    let line = screen.lines().find(|l| l.contains("[ Visualizers ]")).expect("heading drawn");
    let bokeh = screen.lines().find(|l| l.contains("Bokeh")).unwrap();
    assert!(line.find("[ Visualizers ]").unwrap() > bokeh.find("Bokeh").unwrap() - 2, "{screen}");
    app.menu.as_mut().unwrap().hover = Some(at - 1);
    render(&mut app);
    assert!(!app.hits.iter().any(|h| h.target == Target::MenuItem(at - 1)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_fresh_install_starts_on_stereo_and_remembers_the_last_visualizer() {
    let _g = serial();
    let root = library();
    let app = App::new(Some(root.join("Music")));
    assert!(app.settings.analyzer);
    assert_eq!(app.settings.visual, crate::settings::Visual::Spectrum);
    drop(app);

    let mut app = App::new(Some(root.join("Music")));
    app.run(Action::ToggleVisual(Visual::Lines));
    drop(app);
    let app = App::new(Some(root.join("Music")));
    assert!(app.settings.analyzer);
    assert_eq!(app.settings.visual, crate::settings::Visual::Lines);
    drop(app);

    let mut app = App::new(Some(root.join("Music")));
    app.run(Action::ToggleVisual(Visual::Lines));
    assert!(!app.settings.analyzer);
    drop(app);
    let mut app = App::new(Some(root.join("Music")));
    assert!(!app.settings.analyzer);
    assert!(app.menu_items(2).iter().all(|i| i.checked != Some(true) || !["Bokeh", "Bokeh Light", "Lines", "Stereo", "Trace", "Galaxy", "Synthwave", "SynthMoonWave", "Club Lights", "Oscilloscope", "Vectorscope", "Spectroscope"].contains(&i.label.as_str())));

    app.run(Action::ToggleVisual(Visual::Trace));
    drop(app);
    let app = App::new(Some(root.join("Music")));
    assert!(app.settings.analyzer);
    assert_eq!(app.settings.visual, crate::settings::Visual::Trace);
    let _ = fs::remove_dir_all(&root);
}

fn wait_for_queue(app: &mut App, n: usize) {
    for _ in 0..600 {
        app.tick();
        if app.status.queue.len() == n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the engine never showed a queue of {n}: {:?}", app.status.queue);
}

fn load_queue(app: &mut App, songs: &[PathBuf]) {
    use crate::audio::State;
    app.engine.play(songs.to_vec(), 0);
    wait_for_queue(app, songs.len());
    app.engine.stop();
    for _ in 0..600 {
        app.tick();
        if app.status.state == State::Stopped && app.engine.caught_up(&app.status) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("playback never stopped: {:?}", app.status.state);
}

fn queue_order(app: &App) -> Vec<String> {
    app.status.queue.iter().map(|p| p.file_name().unwrap().to_string_lossy()[..2].to_string()).collect()
}

fn album_songs(root: &Path) -> (PathBuf, Vec<PathBuf>) {
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let songs = ["01 - Filler.wav", "02 - I Don't Wanna Hear It.wav", "10 - Seeing Red.wav"].iter().map(|n| album.join(n)).collect();
    (album, songs)
}

#[test]
fn the_queue_view_lists_the_queue_and_the_keyboard_edits_it() {
    let _g = serial();
    let root = library();
    let (album, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);
    load_queue(&mut app, &songs);
    render(&mut app);

    key(&mut app, KeyCode::Char('W'));
    assert!(app.queue_view.is_some() && app.playlists.is_none());
    let screen = render(&mut app);
    assert!(screen.contains("Queue · 3 songs") && screen.contains("01 - Filler") && screen.contains("10 - Seeing Red"), "{screen}");
    assert_eq!(app.menu_items(1)[1].checked, Some(true));
    assert_eq!(queue_order(&app), ["01", "02", "10"]);

    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('J'));
    assert_eq!(queue_order(&app), ["01", "10", "02"]);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 2);
    key(&mut app, KeyCode::Char('K'));
    assert_eq!(queue_order(&app), ["01", "02", "10"]);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 1);
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(queue_order(&app), ["01", "10", "02"]);
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(queue_order(&app), ["01", "10", "02"]);

    key(&mut app, KeyCode::Delete);
    assert_eq!(queue_order(&app), ["01", "10"]);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 1);

    key(&mut app, KeyCode::Char('Q'));
    key(&mut app, KeyCode::Right);
    assert_eq!(queue_order(&app), ["01", "10"]);

    key(&mut app, KeyCode::Esc);
    assert!(app.queue_view.is_none());
    let row = row_named(&app, app.focus, "02 - I Don't Wanna Hear It.wav");
    let col = app.focus;
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col, row });
    key(&mut app, KeyCode::Char('Q'));
    assert_eq!(queue_order(&app), ["01", "10", "02"]);
    app.navigate_to(album.parent().unwrap());
    render(&mut app);
    let row = row_named(&app, app.focus, "Complete Discography");
    let col = app.focus;
    click(&mut app, |t| *t == Target::Row { col, row });
    key(&mut app, KeyCode::Char('Q'));
    assert_eq!(app.status.queue.len(), 6);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_queue_view_works_with_the_mouse() {
    let _g = serial();
    let root = library();
    let (album, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);

    render(&mut app);
    click(&mut app, |t| *t == Target::Menu(1));
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(1));
    let screen = render(&mut app);
    assert!(app.queue_view.is_some());
    assert!(screen.contains("The queue is empty") && screen.contains("Add to Queue"), "{screen}");
    click(&mut app, |t| *t == Target::Back);
    assert!(app.queue_view.is_none());
    app.run(Action::Playlists);
    app.run(Action::Queue);
    assert!(app.playlists.is_none() && app.queue_view.is_some());
    app.run(Action::Playlists);
    assert!(app.queue_view.is_none() && app.playlists.is_some());
    app.run(Action::Playlists);

    load_queue(&mut app, &songs);
    app.run(Action::Queue);
    render(&mut app);

    let r = rect_of(&app, |t| *t == Target::QueueRow(1));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(
        labels,
        ["Play", "", "Move Up", "Move Down", "Remove", "", "Add to Playlist", crate::reveal::label(), "Copy Path", "", "Shuffle Queue", "Clear Queue", "Save Queue as Playlist…"]
    );
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(4));
    assert_eq!(queue_order(&app), ["01", "10"]);

    render(&mut app);
    let (a, b) = (rect_of(&app, |t| *t == Target::QueueRow(0)), rect_of(&app, |t| *t == Target::QueueRow(1)));
    let ev = |kind, r: ratatui::layout::Rect| MouseEvent { kind, column: r.x + 2, row: r.y, modifiers: KeyModifiers::NONE };
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), a));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), b));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), b));
    assert_eq!(queue_order(&app), ["10", "01"]);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 1);
    assert!(app.drag.is_none());

    render(&mut app);
    let a = rect_of(&app, |t| *t == Target::QueueRow(0));
    click_at(&mut app, a.x + 2, a.y, MouseButton::Left);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 0);
    assert_eq!(queue_order(&app), ["10", "01"]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_queue_scrolls_and_keeps_the_highlight_in_view() {
    let _g = serial();
    let root = library();
    let dir = root.join("many");
    fs::create_dir_all(&dir).unwrap();
    let songs: Vec<PathBuf> = (0..60)
        .map(|i| {
            let p = dir.join(format!("{i:02} - song.wav"));
            write_wav(&p, 8_000, 0.1);
            p
        })
        .collect();
    let mut app = App::new(Some(root.join("Music")));
    load_queue(&mut app, &songs);
    app.run(Action::Queue);
    let screen = render(&mut app);
    assert!(screen.contains("Queue · 60 songs") && screen.contains("00 - song") && !screen.contains("59 - song"), "{screen}");
    key(&mut app, KeyCode::End);
    let screen = render(&mut app);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 59);
    assert!(screen.contains("59 - song") && !screen.contains("00 - song"), "{screen}");
    let r = rect_of(&app, |t| matches!(t, Target::QueueRow(_)));
    for _ in 0..10 {
        app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: r.x + 2, row: r.y, modifiers: KeyModifiers::NONE });
    }
    let screen = render(&mut app);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 59);
    assert!(screen.contains("25 - song") && !screen.contains("59 - song"), "{screen}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_queue_can_be_shuffled_cleared_and_saved_as_a_playlist() {
    let _g = serial();
    let root = library();
    let (album, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);

    app.run(Action::SaveQueue);
    assert!(app.overlay.is_none());
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "the queue is empty"));
    app.run(Action::ClearQueue);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "the queue is already empty"));
    app.run(Action::ShuffleQueue);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "no songs are left to shuffle"));

    let labels: Vec<String> = app.menu_items(1).iter().rev().take(3).map(|i| i.label.clone()).collect();
    assert_eq!(labels, ["Save Queue as Playlist…", "Clear Queue", "Shuffle Queue"]);

    load_queue(&mut app, &songs);
    app.run(Action::Queue);
    let mut orders = std::collections::HashSet::new();
    for _ in 0..12 {
        key(&mut app, KeyCode::Char('z'));
        let order = queue_order(&app);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, ["01", "02", "10"]);
        orders.insert(order);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    assert!(orders.len() > 1, "twelve shuffles gave one order: {orders:?}");
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "shuffled 3 songs"));

    let order_now = app.status.queue.clone();
    key(&mut app, KeyCode::Char('S'));
    let screen = render(&mut app);
    assert!(matches!(&app.overlay, Some(Overlay::PlaylistName(n, NameFor::Queue)) if n.is_empty()));
    assert!(screen.contains("Save Queue as Playlist") && screen.contains("Save"), "{screen}");
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_some(), "an empty name is refused");
    for c in "My queue".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert!(app.queue_view.is_some(), "the Queue view stays up");
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "saved the queue (3 songs) as playlist My queue"));
    let lib = crate::playlists::dir();
    let saved = crate::m3u::Playlist::open(&lib.join("My queue.m3u8")).unwrap().tracks(|_| true).paths;
    assert_eq!(saved, *order_now);

    app.run(Action::SaveQueue);
    for c in "My queue".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    render(&mut app);
    click(&mut app, |t| *t == Target::PlaylistNameOk);
    assert!(lib.join("My queue_2.m3u8").is_file());
    app.run(Action::SaveQueue);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && !lib.join("x.m3u8").exists());

    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::QueueRow(0));
    click_at(&mut app, r.x + 1, r.y, MouseButton::Right);
    let items: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    let at = items.iter().position(|l| l == "Clear Queue").unwrap();
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(at));
    assert!(app.status.queue.is_empty());
    assert!(render(&mut app).contains("The queue is empty"));

    load_queue(&mut app, &songs);
    app.run(Action::ShuffleSelection);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "shuffled 3 songs"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn shift_c_clears_the_queue_and_the_selection_goes_back_to_the_top() {
    let _g = serial();
    let root = library();
    let (_, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    load_queue(&mut app, &songs);
    app.run(Action::Queue);
    key(&mut app, KeyCode::End);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 2);
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(app.status.queue.len(), 3);
    key(&mut app, KeyCode::Char('C'));
    assert!(app.status.queue.is_empty());
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 0);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "cleared the queue"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_queue_is_kept_between_runs_and_comes_back_stopped() {
    use crate::audio::State;
    let _g = serial();
    let root = library();
    let (_, songs) = album_songs(&root);
    let file = crate::queue::Keeper::default_file();
    assert!(file.starts_with(&root), "the test must not touch the real queue file");

    let mut app = App::new(Some(root.join("Music")));
    app.restore_queue();
    assert!(app.status.queue.is_empty() && !file.exists());
    load_queue(&mut app, &songs);
    app.tick();
    app.save_waiting();
    assert_eq!(crate::queue::Keeper::open(file.clone()).1, songs);

    drop(app);
    let mut s = crate::settings::Settings::load();
    s.resume = None;
    s.save();
    let mut app = App::new(Some(root.join("Music")));
    app.restore_queue();
    assert_eq!(app.status.queue.to_vec(), songs);
    assert_eq!(app.status.state, State::Stopped);
    app.run(Action::Queue);
    let screen = render(&mut app);
    assert!(screen.contains("Queue · 3 songs") && screen.contains("01 - Filler"), "{screen}");

    key(&mut app, KeyCode::Delete);
    app.tick();
    app.save_waiting();
    assert_eq!(crate::queue::Keeper::open(file.clone()).1, songs[1..]);
    app.run(Action::ClearQueue);
    app.tick();
    app.save_waiting();
    assert!(crate::queue::Keeper::open(file.clone()).1.is_empty());

    load_queue(&mut app, &songs);
    app.tick();
    app.save_waiting();
    drop(app);
    fs::remove_file(&songs[1]).unwrap();
    let mut app = App::new(Some(root.join("Music")));
    app.restore_queue();
    assert_eq!(app.status.queue.to_vec(), [songs[0].clone(), songs[2].clone()]);

    drop(app);
    let _ = fs::remove_file(&file);
    let mut app = App::new(Some(root.join("Music")));
    load_queue(&mut app, &songs);
    app.tick();
    app.save_waiting();
    assert!(!file.exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn saving_songs_as_a_playlist_names_it_and_leaves_nothing_behind_on_failure() {
    let root = std::env::temp_dir().join(format!("urchin_save_songs_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let lib = root.join("playlists");
    let songs: Vec<PathBuf> = ["a.wav", "b.wav"].iter().map(|n| root.join(n)).collect();
    let path = crate::playlists::save_songs_as("Night / drive", &lib, &songs).unwrap();
    assert_eq!(path.file_name().unwrap(), "Night _ drive.m3u8");
    assert_eq!(crate::m3u::Playlist::open(&path).unwrap().tracks(|_| true).paths.len(), 0, "missing files are skipped when reading");
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("#EXTM3U\n{}\n{}\n", songs[0].display(), songs[1].display()));
    let second = crate::playlists::save_songs_as("Night / drive", &lib, &songs).unwrap();
    assert_eq!(second.file_name().unwrap(), "Night _ drive_2.m3u8");
    assert!(crate::playlists::save_songs_as("  ", &lib, &songs).is_err());
    assert_eq!(crate::playlists::list_in(&lib).len(), 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_context_menu_opens_by_a_corner_at_the_click_in_whichever_direction_has_room() {
    use crate::app::Anchor;
    use ratatui::layout::Rect;
    let area = Rect { x: 0, y: 0, width: 80, height: 24 };
    let menu = |n: usize, anchor| {
        let items = (0..n)
            .map(|i| MenuItem { label: format!("item {i}"), shortcut: "", action: Some(Action::Stop), checked: None, heading: false })
            .collect();
        OpenMenu::new(items, anchor)
    };
    let click = |n: usize, col: u16, row: u16| {
        let r = ui::menu_rect(&menu(n, Anchor::Click { col, row }), area);
        (r.x, r.y, r.x + r.width, r.y + r.height)
    };

    assert_eq!(click(5, 30, 5), (29, 6, 29 + 14, 13));
    assert_eq!(click(5, 78, 8), (65, 9, 79, 16));
    assert_eq!(click(10, 3, 20), (2, 8, 16, 20));
    assert_eq!(click(10, 78, 20), (65, 8, 79, 20));
    assert_eq!(click(15, 30, 6).1, 7);
    assert_eq!(click(15, 30, 7), (29, 8, 44, 24));
    assert_eq!(click(15, 30, 18).1 + 17, 18);
    assert_eq!(click(40, 30, 14).1, 1);
    assert_eq!(click(40, 30, 12).1, 13, "a tie goes down");
    assert_eq!(click(3, 0, 8).0, 0);
    let narrow = Rect { x: 0, y: 0, width: 12, height: 24 };
    let r = ui::menu_rect(&menu(3, Anchor::Click { col: 10, row: 8 }), narrow);
    assert_eq!(r.x + r.width, 11);

    let r = ui::menu_rect(&menu(30, Anchor::At { x: 3, y: 1 }), area);
    assert_eq!((r.x, r.y, r.height), (2, 1, 23));
    let r = ui::menu_rect(&menu(3, Anchor::At { x: 70, y: 1 }), area);
    assert_eq!(r.x + r.width, 80);
}

#[test]
fn a_right_click_menu_near_the_bottom_rises_instead_of_running_off_the_screen() {
    let _g = serial();
    let root = library();
    let dir = root.join("many");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..60 {
        write_wav(&dir.join(format!("{i:02} - song.wav")), 8_000, 0.1);
    }
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&dir);
    let item_ys = |app: &App| -> Vec<u16> {
        app.hits.iter().filter(|h| matches!(h.target, Target::MenuItem(_))).map(|h| h.rect.y).collect()
    };
    let bottom_row = |app: &App| app.hits.iter().filter(|h| matches!(h.target, Target::Row { .. })).map(|h| h.rect).max_by_key(|r| r.y).unwrap();

    key(&mut app, KeyCode::End);
    let screen = render(&mut app);
    assert!(!screen.contains('▲') && !screen.contains('▼'));
    let row = bottom_row(&app);
    let before = screen.lines().nth(row.y as usize).unwrap().to_string();
    click_at(&mut app, row.x + 3, row.y, MouseButton::Right);
    let screen = render(&mut app);
    let ys = item_ys(&app);
    assert_eq!(ys.len(), 8, "{ys:?}: {screen}");
    assert!(ys.iter().all(|&y| y < row.y), "the menu is above the click: {ys:?} vs row {}", row.y);
    assert_eq!(*ys.iter().max().unwrap() + 2, row.y, "its bottom edge sits right over the row clicked");
    assert!(!screen.contains('▲') && !screen.contains('▼'), "nothing is cut off: {screen}");
    assert!(screen.contains("Add to Playlist"));
    assert_eq!(screen.lines().nth(row.y as usize).unwrap(), before, "{screen}");
    let at = item_ys(&app).len();
    assert_eq!(at, 8);
    click(&mut app, |t| *t == Target::MenuItem(2));
    wait_for_queue(&mut app, 1);
    assert!(app.menu.is_none());

    render(&mut app);
    key(&mut app, KeyCode::Char('m'));
    render(&mut app);
    let ys = item_ys(&app);
    assert_eq!(ys.len(), 8);
    assert!(ys.iter().all(|&y| y < row.y), "{ys:?} vs row {}", row.y);
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Home);
    render(&mut app);
    let top = app.hits.iter().filter(|h| matches!(h.target, Target::Row { .. })).map(|h| h.rect).min_by_key(|r| r.y).unwrap();
    click_at(&mut app, top.x + 3, top.y, MouseButton::Right);
    render(&mut app);
    let ys = item_ys(&app);
    assert_eq!(ys.len(), 7, "the first song has no Move Up");
    assert_eq!(*ys.iter().min().unwrap(), top.y + 2, "the first item is two rows down: the top border, then it");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Home);
    render_at(&mut app, 110, 50);
    let top = app.hits.iter().filter(|h| matches!(h.target, Target::Row { .. })).map(|h| h.rect).min_by_key(|r| r.y).unwrap();
    let mid = app.hits.iter().filter(|h| matches!(h.target, Target::Row { .. })).map(|h| h.rect).find(|r| r.y == top.y + 8).expect("a row in the middle");
    let col = mid.x + mid.width - 2;
    click_at(&mut app, col, mid.y, MouseButton::Right);
    let screen = render_at(&mut app, 110, 50);
    let rects: Vec<_> = app.hits.iter().filter(|h| matches!(h.target, Target::MenuItem(_))).map(|h| h.rect).collect();
    assert_eq!(rects.len(), 8, "{screen}");
    assert!(rects.iter().all(|r| r.y > mid.y), "drops down, under the row clicked: {rects:?} click row {}", mid.y);
    assert!(rects.iter().all(|r| r.x + r.width <= col + 1 && r.x < col - 10), "opens to the left of the click: {rects:?} click at {col}");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::End);
    render(&mut app);
    let row = bottom_row(&app);
    let col = row.x + row.width - 2;
    click_at(&mut app, col, row.y, MouseButton::Right);
    let screen = render(&mut app);
    let rects: Vec<_> = app.hits.iter().filter(|h| matches!(h.target, Target::MenuItem(_))).map(|h| h.rect).collect();
    assert_eq!(rects.len(), 8, "{screen}");
    assert!(rects.iter().all(|r| r.y < row.y && r.x + r.width <= col + 1 && r.x < col - 10), "{rects:?} click at {col},{}", row.y);
    assert!(!screen.contains('▲') && !screen.contains('▼'), "{screen}");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::End);
    render(&mut app);
    let row = bottom_row(&app);
    click_at(&mut app, row.x + 3, row.y, MouseButton::Right);
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(3));
    let screen = render(&mut app);
    assert!(screen.contains("No Playlists Yet"), "{screen}");
    assert!(item_ys(&app).iter().all(|&y| y < row.y));
    let _ = fs::remove_dir_all(&root);
}

mod search_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::search::{fold, row_cells, trail_text, words, Search};

    const ROOT: &str = "/home/me/Music";

    fn over(files: &[&str]) -> Search {
        let root = PathBuf::from(ROOT);
        Search::from_tracks(root.clone(), files.iter().map(|f| root.join(f)).collect())
    }

    fn find(s: &mut Search, query: &str) -> Vec<String> {
        while !s.query.is_empty() {
            s.backspace();
        }
        for c in query.chars() {
            s.type_char(c);
        }
        (0..s.count())
            .map(|i| {
                let song = s.result(i).unwrap();
                format!("{} @ {}", song.title(), s.trail(song).join("/"))
            })
            .collect()
    }

    #[test]
    fn every_word_typed_must_be_somewhere_in_the_path_in_any_order() {
        let mut s = over(&[
            "Classic Rock/Pink Floyd/The Wall/06 - Comfortably Numb.flac",
            "Classic Rock/Pink Floyd/Wish You Were Here/01 - Shine On You Crazy Diamond.flac",
            "Classic Rock/Pink Floyd/A Collection/01 - Shine On You Crazy Diamond.flac",
            "Classic Rock/Led Zeppelin/IV/04 - Stairway to Heaven.flac",
            "Pink Floyd - Shine On You Crazy Diamond.mp3",
        ]);
        let numb = "06 - Comfortably Numb @ Classic Rock/Pink Floyd/The Wall";
        let wish = "01 - Shine On You Crazy Diamond @ Classic Rock/Pink Floyd/Wish You Were Here";
        let collection = "01 - Shine On You Crazy Diamond @ Classic Rock/Pink Floyd/A Collection";
        let flat = "Pink Floyd - Shine On You Crazy Diamond @ Music";

        assert_eq!(find(&mut s, "Pink Floyd"), [numb, wish, collection, flat]);
        assert_eq!(find(&mut s, "pink FLOYD"), find(&mut s, "Pink Floyd"));
        let shine = [wish, collection, flat];
        assert_eq!(find(&mut s, "pink floyd shine"), shine);
        assert_eq!(find(&mut s, "pink floyd diamond"), shine);
        assert_eq!(find(&mut s, "diamond floyd"), shine);
        assert_eq!(find(&mut s, "crazy dia"), shine);
        assert_eq!(find(&mut s, "Pink Floyd - Shine on you crazy diamond"), shine);
        assert_eq!(find(&mut s, "wall numb"), [numb]);
        assert_eq!(find(&mut s, "zeppelin stairway"), ["04 - Stairway to Heaven @ Classic Rock/Led Zeppelin/IV"]);
        assert!(find(&mut s, "pink floyd stairway").is_empty());
        assert!(find(&mut s, "flac").is_empty() && find(&mut s, "mp3").is_empty());
        assert!(find(&mut s, "music").is_empty());
        for blank in ["", "   ", "-", " - & "] {
            assert!(find(&mut s, blank).is_empty() && s.is_blank(), "{blank:?}");
        }
    }

    #[test]
    fn accents_apostrophes_and_the_way_a_mac_writes_names_do_not_matter() {
        let mut s = over(&[
            "Pop/Beyoncé/Lemonade/01 - Pray You Catch Me.flac",
            "Pop/Beyonce\u{301}/Lemonade/02 - Hold Up.flac",
            "Rock/Queen/Don't Stop Me Now.flac",
            "Rock/Journey/Don\u{2019}t Stop Believin\u{2019}.flac",
            "Metal/Mötley Crüe/Dr. Feelgood.flac",
            "J-Pop/か.flac",
            "J-Pop/が.flac",
        ]);
        let beyonce = ["01 - Pray You Catch Me @ Pop/Beyoncé/Lemonade", "02 - Hold Up @ Pop/Beyonce\u{301}/Lemonade"];
        assert_eq!(find(&mut s, "beyonce"), beyonce);
        assert_eq!(find(&mut s, "BEYONCÉ"), beyonce);
        assert_eq!(find(&mut s, "beyonce\u{301}"), beyonce);
        assert_eq!(find(&mut s, "motley crue"), ["Dr. Feelgood @ Metal/Mötley Crüe"]);
        assert_eq!(find(&mut s, "mötley"), ["Dr. Feelgood @ Metal/Mötley Crüe"]);
        let dont = ["Don't Stop Me Now @ Rock/Queen", "Don\u{2019}t Stop Believin\u{2019} @ Rock/Journey"];
        assert_eq!(find(&mut s, "dont stop"), dont);
        assert_eq!(find(&mut s, "don't stop"), dont);
        assert_eq!(find(&mut s, "don\u{2019}t stop"), dont);
        assert_eq!(find(&mut s, "か"), ["か @ J-Pop"]);
        assert_eq!(find(&mut s, "が"), ["が @ J-Pop"]);
        assert_eq!(find(&mut s, "か\u{3099}"), ["が @ J-Pop"]);
    }

    #[test]
    fn folding_takes_off_capitals_accents_and_apostrophes_and_nothing_else() {
        assert_eq!(fold("Beyoncé"), "beyonce");
        assert_eq!(fold("Beyonce\u{301}"), "beyonce");
        assert_eq!(fold("ÅNGSTRÖM"), "angstrom");
        assert_eq!(fold("İstanbul"), "istanbul");
        assert_eq!(fold("Don't Don\u{2019}t"), "dont dont");
        assert_eq!(fold("が"), "が");
        assert_eq!(fold("か\u{3099}"), "が");
        assert_ne!(fold("か"), fold("が"));
        assert_eq!(fold("한글"), "한글");
        assert_eq!(words("  Pink   Floyd -  Shine "), ["pink", "floyd", "shine"]);
        assert!(words("- & /").is_empty());
        assert_eq!(words("AC/DC 50 Cent"), ["ac/dc", "50", "cent"]);
    }

    #[test]
    fn the_highlight_and_the_scrolling_stay_inside_the_list() {
        let files: Vec<String> = (1..=50).map(|n| format!("Many/Song {n:02}.mp3")).collect();
        let mut s = over(&files.iter().map(String::as_str).collect::<Vec<_>>());
        find(&mut s, "song");
        assert_eq!(s.count(), 50);
        s.select(30, 10);
        assert_eq!((s.selected, s.offset), (30, 21));
        s.step(-1, 10);
        assert_eq!((s.selected, s.offset), (29, 21));
        s.step(-30, 10);
        assert_eq!((s.selected, s.offset), (0, 0));
        s.step(1_000, 10);
        assert_eq!((s.selected, s.offset), (49, 40));
        s.scroll(-1_000, 10);
        assert_eq!((s.selected, s.offset), (49, 0));
        s.scroll(1_000, 10);
        assert_eq!((s.selected, s.offset), (49, 40));
        s.select(7, 10);
        s.type_char(' ');
        assert_eq!(s.selected, 7);
        s.type_char('1');
        assert_eq!((s.count(), s.selected, s.offset), (14, 0, 0));
        s.backspace();
        s.backspace();
        assert_eq!(s.count(), 50);
        find(&mut s, "song 4");
        s.select(3, 10);
        let (songs, from) = s.playlist().unwrap();
        let names: Vec<String> = songs.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Song 04.mp3", "Song 14.mp3", "Song 24.mp3", "Song 34.mp3", "Song 40.mp3", "Song 41.mp3", "Song 42.mp3", "Song 43.mp3", "Song 44.mp3", "Song 45.mp3", "Song 46.mp3", "Song 47.mp3", "Song 48.mp3", "Song 49.mp3"]);
        assert_eq!((from, songs[from].file_name().unwrap().to_string_lossy().into_owned()), (3, "Song 34.mp3".to_string()));
        find(&mut s, "zzz");
        s.step(5, 10);
        assert_eq!((s.count(), s.selected, s.offset), (0, 0, 0));
        assert!(s.highlighted().is_none() && s.playlist().is_none());
    }

    #[test]
    fn long_trails_lose_their_outer_folders_and_rows_share_their_cells() {
        let t = |parts: &[&str], width| trail_text(&parts.iter().map(|p| p.to_string()).collect::<Vec<_>>(), width);
        let deep = ["Classic Rock", "Pink Floyd", "The Wall"];
        assert_eq!(t(&deep, 80), "Classic Rock › Pink Floyd › The Wall");
        assert_eq!(t(&deep, 36), "Classic Rock › Pink Floyd › The Wall");
        assert_eq!(t(&deep, 35), "… › Pink Floyd › The Wall");
        assert_eq!(t(&deep, 25), "… › Pink Floyd › The Wall");
        assert_eq!(t(&deep, 24), "… › The Wall");
        assert_eq!(t(&deep, 3), "… › The Wall");
        assert_eq!(t(&["A very long folder name"], 5), "A very long folder name");

        assert_eq!(row_cells(40, 38, 80), (40, 38));
        assert_eq!(row_cells(60, 38, 80), (46, 32));
        assert_eq!(row_cells(10, 5, 80), (73, 5));
        assert_eq!(row_cells(70, 20, 30), (30, 0));
        assert_eq!(row_cells(70, 0, 80), (80, 0));
        for width in 40..140 {
            for title in [1, 20, 60, 200] {
                for trail in [1, 20, 60, 200] {
                    let (name, folders) = row_cells(title, trail, width);
                    assert!(folders <= trail && name + folders + 2 <= width, "{title} {trail} {width}: {name} {folders}");
                    assert!(folders >= trail.min(width * 2 / 5), "{title} {trail} {width}: {name} {folders}");
                }
            }
        }
    }

    #[test]
    fn the_library_is_read_on_another_thread_and_searched_when_it_arrives() {
        let root = std::env::temp_dir().join(format!("urchin_search_walk_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Classic Rock/Pink Floyd/The Wall")).unwrap();
        fs::write(root.join("Classic Rock/Pink Floyd/The Wall/06 - Comfortably Numb.flac"), b"").unwrap();
        fs::write(root.join("Classic Rock/Pink Floyd/The Wall/notes.txt"), b"").unwrap();
        fs::write(root.join("Classic Rock/Pink Floyd/The Wall/.hidden.flac"), b"").unwrap();

        let mut s = Search::open(root.clone());
        for c in "numb".chars() {
            s.type_char(c);
        }
        let end = Instant::now() + Duration::from_secs(10);
        while s.indexing() {
            assert!(Instant::now() < end, "the library was never read");
            s.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!((s.total(), s.count()), (1, 1));
        let song = s.highlighted().unwrap();
        assert_eq!(s.trail(song), ["Classic Rock", "Pink Floyd", "The Wall"]);
        let _ = fs::remove_dir_all(&root);
    }
}

fn search_library() -> PathBuf {
    let root = library();
    let music = root.join("Music");
    for (folder, name) in [
        ("Classic Rock/Led Zeppelin/IV", "04 - Stairway to Heaven"),
        ("Classic Rock/Pink Floyd/The Wall", "06 - Comfortably Numb"),
        ("Classic Rock/Pink Floyd/Wish You Were Here", "01 - Shine On You Crazy Diamond"),
        ("Classic Rock/Pink Floyd/Wish You Were Here", "02 - Welcome to the Machine"),
        ("Classic Rock/Pink Floyd/A Collection", "01 - Shine On You Crazy Diamond"),
        ("", "Pink Floyd - Shine On You Crazy Diamond"),
    ] {
        let dir = music.join(folder);
        fs::create_dir_all(&dir).unwrap();
        write_wav(&dir.join(format!("{name}.wav")), 44_100, 0.25);
    }
    root
}

fn wait_for_search(app: &mut App) {
    for _ in 0..2000 {
        app.tick();
        match &app.overlay {
            Some(Overlay::Search(s)) if !s.indexing() => return,
            Some(Overlay::Search(_)) => std::thread::sleep(std::time::Duration::from_millis(2)),
            _ => panic!("the Search box is not open"),
        }
    }
    panic!("the Search box never finished reading the library");
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

fn search_of(app: &App) -> &crate::search::Search {
    match &app.overlay {
        Some(Overlay::Search(s)) => s,
        _ => panic!("the Search box is not open"),
    }
}

fn line_with(screen: &str, needle: &str) -> String {
    screen.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no line with {needle:?} in\n{screen}")).to_string()
}

fn music_song(root: &Path, relative: &str) -> PathBuf {
    root.join("Music").join(relative)
}

#[test]
fn search_is_in_the_playback_menu_under_queue_and_opens_with_the_slash_key() {
    let _g = serial();
    let root = search_library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    let items = app.menu_items(1);
    assert_eq!((items[1].label.as_str(), items[2].label.as_str()), ("Queue", "Search…"));
    assert_eq!((items[2].shortcut, items[2].action.clone()), ("/", Some(Action::Search)));
    assert_eq!((items[3].label.as_str(), items[3].action.clone()), ("Library", Some(Action::Library)));
    assert!(items[4].is_separator());

    key(&mut app, KeyCode::Char('/'));
    assert!(matches!(app.overlay, Some(Overlay::Search(_))));
    wait_for_search(&mut app);
    type_text(&mut app, "a/b");
    assert_eq!(search_of(&app).query, "a/b");
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());

    app.run(Action::Queue);
    key(&mut app, KeyCode::Char('/'));
    assert!(matches!(app.overlay, Some(Overlay::Search(_))));
    key(&mut app, KeyCode::Esc);
    app.run(Action::Queue);
    render(&mut app);
    click(&mut app, |t| *t == Target::Menu(1));
    let screen = render(&mut app);
    assert!(line_with(&screen, "Search…").contains('/'), "{screen}");
    click(&mut app, |t| *t == Target::MenuItem(2));
    assert!(matches!(app.overlay, Some(Overlay::Search(_))));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn search_finds_songs_shows_where_they_are_and_plays_them() {
    let _g = serial();
    let root = search_library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    key(&mut app, KeyCode::Char('/'));
    wait_for_search(&mut app);
    let screen = render(&mut app);
    assert!(screen.contains("Playback › Search") && screen.contains("Type any part of a song, album or artist"), "{screen}");
    assert!(screen.contains("Searching 9 songs in"), "{screen}");

    type_text(&mut app, "pink floyd");
    let screen = render(&mut app);
    assert!(screen.contains("5 of 9 songs"), "{screen}");
    assert!(line_with(&screen, "06 - Comfortably Numb").contains("Classic Rock › Pink Floyd › The Wall"), "{screen}");
    assert!(line_with(&screen, "02 - Welcome to the Machine").contains("Classic Rock › Pink Floyd › Wish You Were Here"), "{screen}");
    assert!(line_with(&screen, "Pink Floyd - Shine On You Crazy Diamond").contains("Music"), "{screen}");
    let shines: Vec<&str> = screen.lines().filter(|l| l.contains("01 - Shine On You Crazy Diamond")).collect();
    assert_eq!(shines.len(), 2, "{screen}");
    assert!(shines[0].contains("A Collection") && shines[1].contains("Wish You Were Here"), "{screen}");
    assert!(screen.contains("in Classic Rock › Pink Floyd › A Collection"), "{screen}");

    type_text(&mut app, " diamond");
    let screen = render(&mut app);
    assert!(screen.contains("3 of 9 songs") && !screen.contains("Comfortably Numb"), "{screen}");
    key(&mut app, KeyCode::Down);
    let screen = render(&mut app);
    assert!(screen.contains("in Classic Rock › Pink Floyd › Wish You Were Here"), "{screen}");

    let plays = app.engine.plays_sent();
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!(app.engine.plays_sent(), plays + 1);
    wait_for_queue(&mut app, 3);
    let expected = [
        music_song(&root, "Classic Rock/Pink Floyd/A Collection/01 - Shine On You Crazy Diamond.wav"),
        music_song(&root, "Classic Rock/Pink Floyd/Wish You Were Here/01 - Shine On You Crazy Diamond.wav"),
        music_song(&root, "Pink Floyd - Shine On You Crazy Diamond.wav"),
    ];
    assert_eq!(app.status.queue.as_slice(), expected);
    for _ in 0..600 {
        app.tick();
        if app.engine.caught_up(&app.status) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if app.status.state != crate::audio::State::Stopped {
        assert_eq!(app.status.index, 1);
    }
    assert_eq!(app.current_dir(), root.join("Music"));
    app.engine.stop();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn search_works_with_the_mouse() {
    let _g = serial();
    let root = search_library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);
    app.run(Action::Search);
    wait_for_search(&mut app);
    type_text(&mut app, "pink floyd");
    render(&mut app);

    click(&mut app, |t| *t == Target::SearchRow(2));
    assert_eq!(search_of(&app).selected, 2);
    let screen = render(&mut app);
    assert!(screen.contains("in Classic Rock › Pink Floyd › Wish You Were Here"), "{screen}");

    let r = rect_of(&app, |t| *t == Target::SearchRow(3));
    click_at(&mut app, r.x + 5, r.y, MouseButton::Right);
    assert_eq!(search_of(&app).selected, 3);
    let labels: Vec<&str> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Play", "Play Next", "Add to Queue", "Add to Playlist", "Show in Folder", crate::reveal::label(), "Copy Path"]);
    render(&mut app);

    click(&mut app, |t| *t == Target::MenuItem(4));
    assert!(app.overlay.is_none() && app.menu.is_none());
    let folder = music_song(&root, "Classic Rock/Pink Floyd/Wish You Were Here");
    assert_eq!(app.current_dir(), folder);
    assert_eq!(app.selected_entry().unwrap().path, folder.join("02 - Welcome to the Machine.wav"));
    let screen = render(&mut app);
    assert!(screen.contains("02 - Welcome to the Machine"), "{screen}");

    app.run(Action::Search);
    wait_for_search(&mut app);
    render(&mut app);
    click(&mut app, |t| *t == Target::SearchClose);
    assert!(app.overlay.is_none());
    app.run(Action::Search);
    wait_for_search(&mut app);
    render(&mut app);
    click_at(&mut app, 0, 5, MouseButton::Left);
    assert!(app.overlay.is_none());

    app.run(Action::Search);
    wait_for_search(&mut app);
    type_text(&mut app, "numb");
    render(&mut app);
    let plays = app.engine.plays_sent();
    click(&mut app, |t| *t == Target::SearchPlay);
    assert!(app.overlay.is_none());
    assert_eq!(app.engine.plays_sent(), plays + 1);
    app.run(Action::Search);
    wait_for_search(&mut app);
    type_text(&mut app, "pink");
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::SearchRow(1));
    click_at(&mut app, r.x + 3, r.y, MouseButton::Left);
    assert!(app.overlay.is_some(), "one click only highlights");
    click_at(&mut app, r.x + 3, r.y, MouseButton::Left);
    assert!(app.overlay.is_none());
    assert_eq!(app.engine.plays_sent(), plays + 2);
    wait_for_queue(&mut app, 5);
    app.engine.stop();

    app.run(Action::Search);
    wait_for_search(&mut app);
    type_text(&mut app, "pink floyd");
    render_at(&mut app, 110, 14);
    let list = rect_of(&app, |t| *t == Target::SearchList);
    assert_eq!(list.height, 2);
    app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: list.x + 3, row: list.y, modifiers: KeyModifiers::NONE });
    let screen = render_at(&mut app, 110, 14);
    assert!(search_of(&app).offset > 0 && search_of(&app).selected == 0, "{screen}");
    assert!(!screen.contains("A Collection"), "{screen}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn search_is_driven_from_the_keyboard_and_letters_are_typed_not_obeyed() {
    let _g = serial();
    let root = search_library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);
    key(&mut app, KeyCode::Char('/'));
    wait_for_search(&mut app);

    type_text(&mut app, "q u n e p");
    assert!(!app.quit && app.queue_view.is_none());
    assert_eq!(search_of(&app).query, "q u n e p");
    for _ in 0.."q u n e p".len() {
        key(&mut app, KeyCode::Backspace);
    }
    assert_eq!(search_of(&app).query, "");
    app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(app.quit);
    app.quit = false;

    type_text(&mut app, "stairway");
    key(&mut app, KeyCode::Right);
    assert_eq!(app.menu.as_ref().unwrap().hover, Some(0));
    key(&mut app, KeyCode::Esc);
    assert!(app.menu.is_none() && app.overlay.is_some());

    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_some() && app.menu.is_none());
    wait_for_queue(&mut app, 1);
    assert_eq!(app.status.queue.as_slice(), [music_song(&root, "Classic Rock/Led Zeppelin/IV/04 - Stairway to Heaven.wav")]);
    app.engine.stop();

    let playlist = crate::playlists::create_in("Favorites", &crate::playlists::dir()).unwrap();
    key(&mut app, KeyCode::Right);
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    let labels: Vec<&str> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Favorites"]);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    let screen = render(&mut app);
    assert!(screen.contains("added 04 - Stairway to Heaven to Favorites") && matches!(app.overlay, Some(Overlay::Search(_))), "{screen}");
    let saved = crate::m3u::Playlist::open(&playlist).unwrap().tracks(|_| true).paths;
    assert_eq!(saved, [music_song(&root, "Classic Rock/Led Zeppelin/IV/04 - Stairway to Heaven.wav")]);

    for _ in 0.."stairway".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "pink floyd");
    key(&mut app, KeyCode::End);
    assert_eq!(search_of(&app).selected, 4);
    key(&mut app, KeyCode::Up);
    assert_eq!(search_of(&app).selected, 3);
    key(&mut app, KeyCode::Home);
    assert_eq!(search_of(&app).selected, 0);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(search_of(&app).selected, 4);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(search_of(&app).selected, 0);
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert_eq!(search_of(&app).selected, 1);
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(search_of(&app).selected, 0);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn search_in_a_folder_without_songs_says_so_and_small_windows_still_work() {
    let _g = serial();
    let root = search_library();
    let mut app = App::new(Some(root.join("Music/ambient")));
    render(&mut app);
    key(&mut app, KeyCode::Char('/'));
    wait_for_search(&mut app);
    let screen = render(&mut app);
    assert!(screen.contains("Searching 0 songs in") && screen.contains("0 songs"), "{screen}");
    type_text(&mut app, "pink");
    let screen = render(&mut app);
    assert!(screen.contains("No songs match") && screen.contains("0 of 0 songs"), "{screen}");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Right);
    assert!(app.menu.is_none() && app.overlay.is_some());
    let screen = render(&mut app);
    assert!(screen.contains("there is nothing to play yet"), "{screen}");
    key(&mut app, KeyCode::Esc);

    let mut app = App::new(Some(root.join("Music")));
    for (w, h) in [(30, 10), (40, 12), (60, 20), (80, 24), (200, 60)] {
        app.run(Action::Search);
        wait_for_search(&mut app);
        type_text(&mut app, "pink floyd");
        let screen = render_at(&mut app, w, h);
        assert!(screen.contains("Search"), "{w}x{h}\n{screen}");
        if w >= 60 {
            assert!(screen.contains("in Classic Rock › Pink Floyd › A Collection"), "{w}x{h}\n{screen}");
        }
        key(&mut app, KeyCode::Esc);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore]
fn search_cells_dump() {
    use ratatui::style::{Color, Modifier};
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (w, h) = (env("SEARCH_COLS", "85").parse::<u16>().unwrap(), env("SEARCH_ROWS", "34").parse::<u16>().unwrap());
    let out = env("SEARCH_OUT", "/tmp/search_cells");
    let names: Vec<String> = env("SEARCH_THEMES", "Gilded-Forest-Dark").split(',').map(|s| s.trim().to_string()).collect();
    let _g = serial();

    let root = std::env::temp_dir().join(format!("urchin_search_render_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    std::env::set_var("HOME", &root);
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let music = root.join("Music");
    for (folder, song) in [
        ("Rock/Pink Floyd/Wish You Were Here", "Pink Floyd - Shine on you crazy diamond"),
        ("Rock/Pink Floyd/Wish You Were Here", "Pink Floyd - Welcome to the machine"),
        ("Rock/Pink Floyd/Wish You Were Here", "Pink Floyd - Have a cigar"),
        ("Rock/Pink Floyd/Wish You Were Here", "Pink Floyd - Wish you were here"),
        ("Rock/Pink Floyd/The Wall", "Pink Floyd - Another brick in the wall"),
        ("Rock/Pink Floyd/The Wall", "Pink Floyd - Comfortably numb"),
        ("Rock/Pink Floyd/The Wall", "Pink Floyd - Hey you"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "Pink Floyd - Time"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "Pink Floyd - Money"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "Pink Floyd - Us and them"),
        ("Rock/Led Zeppelin/IV", "Led Zeppelin - Black dog"),
        ("Rock/Led Zeppelin/IV", "Led Zeppelin - Stairway to heaven"),
        ("Rock/Queen/Greatest Hits", "Queen - Bohemian rhapsody"),
        ("Rock/Queen/Greatest Hits", "Queen - Don't stop me now"),
        ("Compilations/Best of Pink Floyd", "Pink Floyd - Shine on you crazy diamond"),
        ("Compilations/Best of Pink Floyd", "Pink Floyd - Money"),
        ("Jazz/Miles Davis/Kind of Blue", "Miles Davis - So what"),
        ("Jazz/Miles Davis/Kind of Blue", "Miles Davis - Blue in green"),
        ("Pop/Beyoncé/Lemonade", "Beyoncé - Hold up"),
        ("Pop/Beyoncé/Lemonade", "Beyoncé - Sorry"),
        ("Punk/Minor Threat/Complete Discography", "Minor Threat - Filler"),
        ("Punk/Minor Threat/Complete Discography", "Minor Threat - Seeing red"),
    ] {
        let dir = music.join(folder);
        fs::create_dir_all(&dir).unwrap();
        write_wav(&dir.join(format!("{song}.wav")), 44_100, 0.25);
    }
    let album = music.join("Rock/Pink Floyd/Wish You Were Here");

    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        other => panic!("not a plain color: {other:?}"),
    };
    let dump = |buf: &ratatui::buffer::Buffer, file: String| {
        let mut text = format!("{} {}\n", buf.area.width, buf.area.height);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                assert_eq!(c.symbol().chars().count(), 1, "cell {x},{y} holds {:?}", c.symbol());
                text.push_str(c.symbol());
                text.push_str(&hex(c.fg));
                text.push_str(&hex(c.bg));
                text.push(if c.modifier.contains(Modifier::BOLD) { '1' } else { '0' });
            }
            text.push('\n');
        }
        fs::write(file, text).unwrap();
    };
    let playing = |app: &mut App| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![album.join("Pink Floyd - Welcome to the machine.wav")]);
        app.status.meta.title = "Welcome to the machine".into();
        app.status.meta.artist = "Pink Floyd".into();
        app.status.position = 71.0;
        app.status.duration = 207.0;
        app.status.error = None;
    };

    for name in &names {
        let index = crate::theme::index_of(name)
            .or_else(|| crate::theme::index_of(&name.replace('-', " ")))
            .unwrap_or_else(|| panic!("no theme called {name}"));
        let dir = format!("{out}/{}", name.to_lowercase().replace([' ', '-'], "_"));
        fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(Some(music.clone()));
        app.theme = index;
        app.navigate_to(&album);
        playing(&mut app);
        let scene = |app: &mut App, file: &str| {
            playing(app);
            app.toast = None;
            let buf = draw_at(app, w, h);
            dump(&buf, format!("{dir}/{file}.window"));
        };

        draw_at(&mut app, w, h);
        let tap = app.tap.clone();
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (columns, segments) = (((w - 2 + 1) / 3) as usize, crate::ui::analyzer_rows(h, false) - 1);
        run_analyzer(app.visuals.spectrum(), &tap, columns, segments, 0.0, 1.3, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);

        click(&mut app, |t| *t == Target::Menu(1));
        draw_at(&mut app, w, h);
        let item = rect_of(&app, |t| *t == Target::MenuItem(2));
        app.on_mouse(MouseEvent { kind: MouseEventKind::Moved, column: item.x + 3, row: item.y, modifiers: KeyModifiers::NONE });
        scene(&mut app, "1_playback_menu");
        click(&mut app, |t| *t == Target::MenuItem(2));
        wait_for_search(&mut app);
        scene(&mut app, "2_search_opened");

        type_text(&mut app, "pink floyd");
        scene(&mut app, "3_pink_floyd");
        type_text(&mut app, " diamond");
        key(&mut app, KeyCode::Down);
        scene(&mut app, "4_pink_floyd_diamond");

        let r = rect_of(&app, |t| *t == Target::SearchRow(0));
        click_at(&mut app, r.x + 12, r.y, MouseButton::Right);
        scene(&mut app, "5_menu_of_a_result");
        key(&mut app, KeyCode::Esc);

        for _ in 0.."pink floyd diamond".len() {
            key(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "pink floyd stairway");
        scene(&mut app, "6_no_match");
        for _ in 0.."pink floyd stairway".len() {
            key(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "beyonce");
        scene(&mut app, "7_accents");
        key(&mut app, KeyCode::Esc);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore]
fn scope_cells_dump() {
    use ratatui::style::{Color, Modifier};
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (w, h) = (env("SCOPE_COLS", "90").parse::<u16>().unwrap(), env("SCOPE_ROWS", "30").parse::<u16>().unwrap());
    let out = env("SCOPE_OUT", "/tmp/scope_cells");
    let names: Vec<String> = env("SCOPE_THEMES", "Gilded-Forest-Dark").split(',').map(|s| s.trim().to_string()).collect();
    let _g = serial();
    let root = std::env::temp_dir().join(format!("urchin_scope_render_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    std::env::set_var("HOME", &root);
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let album = root.join("Music/Rock/Pink Floyd/Wish You Were Here");
    fs::create_dir_all(&album).unwrap();
    for song in ["Shine on you crazy diamond", "Welcome to the machine", "Have a cigar", "Wish you were here"] {
        write_wav(&album.join(format!("Pink Floyd - {song}.wav")), 44_100, 0.25);
    }
    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        other => panic!("not a plain color: {other:?}"),
    };
    let dump = |buf: &ratatui::buffer::Buffer, file: String| {
        let mut text = format!("{} {}\n", buf.area.width, buf.area.height);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                assert_eq!(c.symbol().chars().count(), 1, "cell {x},{y} holds {:?}", c.symbol());
                text.push_str(c.symbol());
                text.push_str(&hex(c.fg));
                text.push_str(&hex(c.bg));
                text.push(if c.modifier.contains(Modifier::BOLD) { '1' } else { '0' });
            }
            text.push('\n');
        }
        fs::write(file, text).unwrap();
    };
    let playing = |app: &mut App| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![album.join("Pink Floyd - Welcome to the machine.wav")]);
        app.status.meta.title = "Welcome to the machine".into();
        app.status.meta.artist = "Pink Floyd".into();
        app.status.position = 71.0;
        app.status.duration = 207.0;
        app.status.error = None;
        app.toast = None;
    };
    let feed = |tap: &crate::spectrum::Tap, from: f32, secs: f32| {
        let rate = 48_000.0;
        let mut wr = tap.writer();
        for i in 0..(secs * rate) as usize {
            let t = from + i as f32 / rate;
            let l = (synth_music(t) * 1.6).clamp(-1.0, 1.0);
            let r = (synth_music(t - 0.0009) * 1.25 + 0.15 * (std::f32::consts::TAU * 440.0 * t).sin()).clamp(-1.0, 1.0);
            wr.push_stereo((l + r) * 0.5, l, r);
        }
    };
    for name in &names {
        let index = crate::theme::index_of(name)
            .or_else(|| crate::theme::index_of(&name.replace('-', " ")))
            .unwrap_or_else(|| panic!("no theme called {name}"));
        let dir = format!("{out}/{}", name.to_lowercase().replace([' ', '-'], "_"));
        fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(Some(root.join("Music")));
        app.theme = index;
        app.tap.set_rate(48_000);
        app.navigate_to(&album);
        let tap = app.tap.clone();
        let mut t = 0.0f32;
        for (action, file) in [
            (Action::ToggleVisual(Visual::Oscilloscope), "oscilloscope"),
            (Action::ToggleVisual(Visual::Vectorscope), "vectorscope"),
            (Action::ToggleVisual(Visual::Spectroscope), "spectroscope"),
        ] {
            app.run(action);
            for frame in 0..3 {
                for _ in 0..8 {
                    feed(&tap, t, 0.033);
                    t += 0.033;
                    playing(&mut app);
                    draw_at(&mut app, w, h);
                }
                playing(&mut app);
                let buf = draw_at(&mut app, w, h);
                dump(&buf, format!("{dir}/{file}_{frame}.window"));
            }
        }
        app.run(Action::ToggleVisual(Visual::Oscilloscope));
        click(&mut app, |t| *t == Target::Scope);
        for _ in 0..30 {
            key(&mut app, KeyCode::Up);
        }
        key(&mut app, KeyCode::Char('u'));
        for _ in 0..4 {
            feed(&tap, t, 0.033);
            t += 0.033;
            playing(&mut app);
            draw_at(&mut app, w, h);
        }
        playing(&mut app);
        let buf = draw_at(&mut app, w, h);
        dump(&buf, format!("{dir}/oscilloscope_focused.window"));
        key(&mut app, KeyCode::Char('q'));
        click(&mut app, |t| *t == Target::Menu(2));
        playing(&mut app);
        let buf = draw_at(&mut app, w, h);
        dump(&buf, format!("{dir}/settings_menu.window"));
        key(&mut app, KeyCode::Esc);
    }
    let _ = fs::remove_dir_all(&root);
}

fn tagged_library() -> PathBuf {
    let root = std::env::temp_dir().join(format!("urchin_test_tagged_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let misc = root.join("Music/misc");
    fs::create_dir_all(&misc).unwrap();
    for (file, title, artist, album, genre, no) in [
        ("f1", "Time", "Pink Floyd", "Dark Side", "Rock", "4"),
        ("f2", "Speak to Me", "Pink Floyd", "Dark Side", "Rock", "1"),
        ("f3", "Money", "Pink Floyd", "Dark Side", "Rock", "6"),
        ("f4", "Roygbiv", "Boards of Canada", "Music Has the Right", "Electronic", "1"),
        ("f5", "Hey", "Pixies", "Greatest Hits", "Rock", "1"),
        ("f6", "Hello", "Adele", "Greatest Hits", "Pop", "1"),
    ] {
        let wav = crate::library::wav_with_tags(&[("INAM", title), ("IART", artist), ("IPRD", album), ("IGNR", genre), ("IPRT", no)]);
        fs::write(misc.join(format!("{file}.wav")), wav).unwrap();
    }
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    root
}

fn wait_for_library(app: &mut App) {
    for _ in 0..2000 {
        app.tick();
        match &app.library {
            Some(v) if !v.reading() => return,
            Some(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
            None => panic!("the Library view is not open"),
        }
    }
    panic!("the Library view never finished reading the tags");
}

fn open_library_view(root: &Path) -> App {
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);
    key(&mut app, KeyCode::Char('i'));
    wait_for_library(&mut app);
    render(&mut app);
    app
}

fn lib(app: &App) -> &crate::library::View {
    app.library.as_ref().expect("the Library view is open")
}

fn lib_row(app: &App, col: usize, name: &str) -> usize {
    lib(app).cols[col].rows.iter().position(|r| r.name == name).unwrap_or_else(|| panic!("no {name:?} in column {col}"))
}

fn lib_names(app: &App, col: usize) -> Vec<String> {
    lib(app).cols[col].rows.iter().map(|r| r.name.clone()).collect()
}

#[test]
fn the_library_view_opens_from_the_menu_or_the_i_key_and_lists_the_tags_not_the_folders() {
    let _g = serial();
    let root = tagged_library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);

    click(&mut app, |t| *t == Target::Menu(1));
    render(&mut app);
    let item = app.menu_items(1).iter().position(|i| i.action == Some(Action::Library)).unwrap();
    assert_eq!(app.menu_items(1)[item].shortcut, "I");
    click(&mut app, |t| *t == Target::MenuItem(item));
    let reading = render(&mut app);
    assert!(app.library.is_some());
    if lib(&app).reading() {
        assert!(reading.contains("Reading your music"), "{reading}");
    }
    wait_for_library(&mut app);
    let screen = render(&mut app);
    for word in ["Genre", "Artist", "Album", "Electronic", "Pop", "Rock", "Pink Floyd", "Dark Side"] {
        assert!(screen.contains(word), "{word} is missing from\n{screen}");
    }
    assert!(!screen.contains("misc"), "{screen}");
    assert!(line_with(&screen, "Library").contains("Urchin") || screen.contains("Library"));
    assert_eq!(lib_names(&app, 0), ["All", "Electronic", "Pop", "Rock"]);
    assert!(app.menu_items(1).iter().any(|i| i.action == Some(Action::Library) && i.checked == Some(true)));

    key(&mut app, KeyCode::Char('i'));
    assert!(app.library.is_none());
    key(&mut app, KeyCode::Char('i'));
    assert!(app.library.is_some());
    key(&mut app, KeyCode::Char('W'));
    assert!(app.library.is_none() && app.queue_view.is_some());
    key(&mut app, KeyCode::Char('i'));
    assert!(app.library.is_some() && app.queue_view.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_library_view_is_driven_from_the_keyboard_alone() {
    let _g = serial();
    let root = tagged_library();
    let mut app = open_library_view(&root);

    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(lib(&app).cols[0].selected, lib_row(&app, 0, "Rock"));
    assert_eq!(lib_names(&app, 1), ["All", "Pink Floyd", "Pixies"]);
    key(&mut app, KeyCode::Right);
    assert_eq!(lib(&app).focus, 1);
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(lib_names(&app, 2), ["All", "Dark Side"]);
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(lib(&app).focus, 2);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    assert_eq!(lib(&app).focus, crate::library::SONGS);
    let screen = render(&mut app);
    assert!(screen.contains("Rock › Pink Floyd › Dark Side · 3 songs"), "{screen}");
    let at = |s: &str| screen.find(s).unwrap_or_else(|| panic!("{s} missing from\n{screen}"));
    assert!(at("Speak to Me") < at("Time") && at("Time") < at("Money"), "{screen}");
    assert!(!screen.contains("Genre"), "the songs are one pane\n{screen}");

    key(&mut app, KeyCode::Down);
    let plays = app.engine.plays_sent();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.engine.plays_sent(), plays + 1);
    wait_for_queue(&mut app, 3);
    assert!(app.status.queue[0].ends_with("f2.wav"), "Speak to Me first: {:?}", app.status.queue);

    key(&mut app, KeyCode::Left);
    assert_eq!(lib(&app).focus, 2);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(lib(&app).focus, 2, "Backspace in the songs goes back to the columns");
    key(&mut app, KeyCode::Backspace);
    assert!(app.library.is_none(), "Backspace in the columns goes back to the folders");

    key(&mut app, KeyCode::Char('i'));
    wait_for_library(&mut app);
    render(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('h'));
    let plays = app.engine.plays_sent();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.engine.plays_sent(), plays + 1);
    wait_for_queue(&mut app, 4);
    key(&mut app, KeyCode::Esc);
    assert!(app.library.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_library_view_works_with_the_mouse() {
    let _g = serial();
    let root = tagged_library();
    let mut app = open_library_view(&root);

    let rock = lib_row(&app, 0, "Rock");
    click(&mut app, |t| *t == Target::LibraryRow { col: 0, row: rock });
    render(&mut app);
    assert_eq!(lib_names(&app, 1), ["All", "Pink Floyd", "Pixies"]);
    let pixies = lib_row(&app, 1, "Pixies");
    click(&mut app, |t| *t == Target::LibraryRow { col: 1, row: pixies });
    render(&mut app);
    assert_eq!(lib_names(&app, 2), ["All", "Greatest Hits"]);
    assert_eq!(lib(&app).focus, 1, "a click gives the column the keyboard");

    let hits = lib_row(&app, 2, "Greatest Hits");
    click(&mut app, |t| *t == Target::LibraryOpen(hits));
    let screen = render(&mut app);
    assert_eq!(lib(&app).focus, crate::library::SONGS);
    assert!(screen.contains("Rock › Pixies › Greatest Hits · 1 song") && screen.contains("Hey"), "{screen}");

    click(&mut app, |t| *t == Target::Back);
    render(&mut app);
    assert_eq!(lib(&app).focus, 2);
    let plays = app.engine.plays_sent();
    click(&mut app, |t| *t == Target::LibraryRow { col: 2, row: hits });
    click(&mut app, |t| *t == Target::LibraryRow { col: 2, row: hits });
    assert_eq!(app.engine.plays_sent(), plays + 1);
    wait_for_queue(&mut app, 1);

    click(&mut app, |t| *t == Target::LibraryOpen(hits));
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::LibraryRow { col: crate::library::SONGS, row: 0 });
    click_at(&mut app, r.x + 2, r.y, MouseButton::Right);
    let labels: Vec<String> = app.menu.as_ref().expect("a menu").items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(labels, ["Shuffle", "Play Next", "Add to Queue", "", "Add to Playlist", "Show in Folder", crate::reveal::label(), "Copy Path"]);
    render(&mut app);
    let show = labels.iter().position(|l| l == "Show in Folder").unwrap();
    click(&mut app, |t| *t == Target::MenuItem(show));
    assert!(app.library.is_none(), "Show in Folder leaves the Library");
    assert_eq!(app.current_dir(), root.join("Music/misc"));
    assert_eq!(app.selected_entry().map(|e| e.name.clone()), Some("f5.wav".to_string()), "the Pixies song is highlighted");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn right_click_an_album_offers_its_songs_and_a_genre_can_be_queued() {
    let _g = serial();
    let root = tagged_library();
    let mut app = open_library_view(&root);
    let rock = lib_row(&app, 0, "Rock");
    click(&mut app, |t| *t == Target::LibraryRow { col: 0, row: rock });
    render(&mut app);
    let r = rect_of(&app, |t| *t == Target::LibraryRow { col: 0, row: rock });
    click_at(&mut app, r.x + 2, r.y, MouseButton::Right);
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(labels, ["Shuffle", "Play Next", "Add to Queue"]);
    app.menu = None;
    app.engine.stop();
    key(&mut app, KeyCode::Char('Q'));
    wait_for_queue(&mut app, 4);

    let floyd = lib_row(&app, 1, "Pink Floyd");
    click(&mut app, |t| *t == Target::LibraryRow { col: 1, row: floyd });
    render(&mut app);
    let dark = lib_row(&app, 2, "Dark Side");
    let r = rect_of(&app, |t| *t == Target::LibraryRow { col: 2, row: dark });
    click_at(&mut app, r.x + 2, r.y, MouseButton::Right);
    assert_eq!(app.menu.as_ref().unwrap().items[0].label, "Show Songs");
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(0));
    assert_eq!(lib(&app).focus, crate::library::SONGS);
    assert_eq!(lib(&app).songs.len(), 3);
    key(&mut app, KeyCode::Char('m'));
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert!(labels.contains(&"Show in Folder".to_string()) && labels.contains(&"Add to Playlist".to_string()), "{labels:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn search_in_the_library_matches_tags_and_shows_where_a_song_sits() {
    let _g = serial();
    let root = tagged_library();
    let mut app = open_library_view(&root);
    key(&mut app, KeyCode::Char('/'));
    wait_for_search(&mut app);
    assert!(search_of(&app).by_tags());
    let screen = render(&mut app);
    assert!(screen.contains("Library › Search") && screen.contains("Searching the tags of 6 songs"), "{screen}");

    type_text(&mut app, "dark floyd");
    assert_eq!(search_of(&app).count(), 3);
    let screen = render(&mut app);
    assert!(line_with(&screen, "Time").contains("Rock › Pink Floyd › Dark Side"), "{screen}");
    for _ in 0.."dark floyd".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "misc");
    assert_eq!(search_of(&app).count(), 0, "folder names are not tags");
    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "greatest rock");
    assert_eq!(search_of(&app).count(), 1, "Greatest Hits by a Rock artist: the Pixies");
    for _ in 0.."greatest rock".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "greatest");
    let found: Vec<String> = (0..search_of(&app).count()).map(|i| search_of(&app).result(i).unwrap().title().into_owned()).collect();
    assert_eq!(found, ["Hello", "Hey"], "Adele before the Pixies");
    for _ in 0.."greatest".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "pop");
    let screen = render(&mut app);
    assert!(line_with(&screen, "Hello").contains("Pop › Adele › Greatest Hits"), "{screen}");
    let plays = app.engine.plays_sent();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.engine.plays_sent(), plays + 1);
    assert!(app.library.is_some(), "the Library stays where it was");

    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
    assert!(app.library.is_none());
    key(&mut app, KeyCode::Char('/'));
    wait_for_search(&mut app);
    assert!(!search_of(&app).by_tags());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_library_view_fits_small_windows_and_an_empty_music_folder() {
    let _g = serial();
    let root = tagged_library();
    let mut app = open_library_view(&root);
    for (w, h) in [(110u16, 30u16), (90, 30), (60, 20), (59, 12), (45, 10), (40, 10), (39, 10), (30, 10), (20, 5)] {
        let screen = render_at(&mut app, w, h);
        if w >= 30 && h >= 10 {
            assert!(screen.contains("Genre") || screen.contains("Artist") || screen.contains("Album"), "{w}x{h}\n{screen}");
        }
    }
    let wide = render_at(&mut app, 90, 30);
    assert!(wide.contains("Genre") && wide.contains("Artist") && wide.contains("Album"), "{wide}");
    let narrow = render_at(&mut app, 50, 20);
    assert!(narrow.contains("Genre") && narrow.contains("Artist") && !narrow.contains("Album"), "{narrow}");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    let narrow = render_at(&mut app, 50, 20);
    assert!(narrow.contains("Album") && narrow.contains("Artist") && !narrow.contains("Genre"), "{narrow}");
    let tiny = render_at(&mut app, 30, 12);
    assert!(tiny.contains("Album") && !tiny.contains("Genre"), "{tiny}");
    key(&mut app, KeyCode::Right);
    for (w, h) in [(110u16, 30u16), (60, 12), (30, 10), (12, 4)] {
        let _ = render_at(&mut app, w, h);
    }
    let _ = fs::remove_dir_all(&root);

    let empty = std::env::temp_dir().join(format!("urchin_test_empty_{}", std::process::id()));
    let _ = fs::remove_dir_all(&empty);
    fs::create_dir_all(empty.join("Music")).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", empty.join("config"));
    let mut app = open_library_view(&empty);
    let screen = render(&mut app);
    assert!(screen.contains("No songs found"), "{screen}");
    for code in [KeyCode::Down, KeyCode::Right, KeyCode::Right, KeyCode::Enter, KeyCode::End, KeyCode::Left] {
        key(&mut app, code);
        render(&mut app);
    }
    assert!(app.library.is_some());
    let _ = fs::remove_dir_all(&empty);
}

#[test]
#[ignore]
fn library_cells_dump() {
    use ratatui::style::{Color, Modifier};
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (w, h) = (env("LIBRARY_COLS", "90").parse::<u16>().unwrap(), env("LIBRARY_ROWS", "30").parse::<u16>().unwrap());
    let out = env("LIBRARY_OUT", "/tmp/library_cells");
    let names: Vec<String> = env("LIBRARY_THEMES", "Gilded-Forest-Dark").split(',').map(|s| s.trim().to_string()).collect();
    let _g = serial();

    let root = std::env::temp_dir().join(format!("urchin_library_render_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    std::env::set_var("HOME", &root);
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let music = root.join("Music");
    for (folder, file, title, artist, album, genre, no) in [
        ("Rock/Pink Floyd/Wish You Were Here", "01", "Shine On You Crazy Diamond (Parts I-V)", "Pink Floyd", "Wish You Were Here", "Rock", "1"),
        ("Rock/Pink Floyd/Wish You Were Here", "02", "Welcome to the Machine", "Pink Floyd", "Wish You Were Here", "Rock", "2"),
        ("Rock/Pink Floyd/Wish You Were Here", "03", "Have a Cigar", "Pink Floyd", "Wish You Were Here", "Rock", "3"),
        ("Rock/Pink Floyd/Wish You Were Here", "04", "Wish You Were Here", "Pink Floyd", "Wish You Were Here", "Rock", "4"),
        ("Rock/Pink Floyd/The Wall", "01", "Another Brick in the Wall, Pt. 2", "Pink Floyd", "The Wall", "Rock", "5"),
        ("Rock/Pink Floyd/The Wall", "02", "Comfortably Numb", "Pink Floyd", "The Wall", "Rock", "6"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "01", "Speak to Me", "Pink Floyd", "The Dark Side of the Moon", "Rock", "1"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "02", "Breathe", "Pink Floyd", "The Dark Side of the Moon", "Rock", "2"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "03", "Time", "Pink Floyd", "The Dark Side of the Moon", "Rock", "4"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "04", "Money", "Pink Floyd", "The Dark Side of the Moon", "Rock", "6"),
        ("Rock/Pink Floyd/The Dark Side of the Moon", "05", "Us and Them", "Pink Floyd", "The Dark Side of the Moon", "Rock", "7"),
        ("Rock/Led Zeppelin/IV", "01", "Black Dog", "Led Zeppelin", "IV", "Rock", "1"),
        ("Rock/Led Zeppelin/IV", "02", "Stairway to Heaven", "Led Zeppelin", "IV", "Rock", "4"),
        ("Rock/Queen/Greatest Hits", "01", "Bohemian Rhapsody", "Queen", "Greatest Hits", "Rock", "1"),
        ("Rock/Queen/Greatest Hits", "02", "Don't Stop Me Now", "Queen", "Greatest Hits", "Rock", "2"),
        ("Jazz/Miles Davis/Kind of Blue", "01", "So What", "Miles Davis", "Kind of Blue", "Jazz", "1"),
        ("Jazz/Miles Davis/Kind of Blue", "02", "Blue in Green", "Miles Davis", "Kind of Blue", "Jazz", "3"),
        ("Pop/Beyoncé/Lemonade", "01", "Hold Up", "Beyoncé", "Lemonade", "Pop", "3"),
        ("Pop/Beyoncé/Lemonade", "02", "Sorry", "Beyoncé", "Lemonade", "Pop", "5"),
        ("Pop/Adele/21", "01", "Rolling in the Deep", "Adele", "21", "Pop", "1"),
        ("Pop/Adele/21", "02", "Someone Like You", "Adele", "21", "Pop", "11"),
        ("Pop/Queen/Greatest Hits", "01", "Another One Bites the Dust", "Queen", "Greatest Hits", "Pop", "3"),
        ("Punk/Minor Threat/Complete Discography", "01", "Filler", "Minor Threat", "Complete Discography", "Punk", "1"),
        ("Punk/Minor Threat/Complete Discography", "02", "Seeing Red", "Minor Threat", "Complete Discography", "Punk", "2"),
        ("Electronic/Boards of Canada", "01", "Roygbiv", "Boards of Canada", "Music Has the Right to Children", "Electronic", "4"),
        ("Misc", "mystery", "", "", "", "", ""),
    ] {
        let dir = music.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let mut info: Vec<(&str, &str)> = Vec::new();
        for (id, text) in [("INAM", title), ("IART", artist), ("IPRD", album), ("IGNR", genre), ("IPRT", no)] {
            if !text.is_empty() {
                info.push((id, text));
            }
        }
        fs::write(dir.join(format!("{file}.wav")), crate::library::wav_with_tags(&info)).unwrap();
    }
    let playing_song = music.join("Rock/Pink Floyd/Wish You Were Here/02.wav");

    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        other => panic!("not a plain color: {other:?}"),
    };
    let dump = |buf: &ratatui::buffer::Buffer, file: String| {
        let mut text = format!("{} {}\n", buf.area.width, buf.area.height);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                assert_eq!(c.symbol().chars().count(), 1, "cell {x},{y} holds {:?}", c.symbol());
                text.push_str(c.symbol());
                text.push_str(&hex(c.fg));
                text.push_str(&hex(c.bg));
                text.push(if c.modifier.contains(Modifier::BOLD) { '1' } else { '0' });
            }
            text.push('\n');
        }
        fs::write(file, text).unwrap();
    };
    let playing = |app: &mut App| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![playing_song.clone()]);
        app.status.meta.title = "Welcome to the Machine".into();
        app.status.meta.artist = "Pink Floyd".into();
        app.status.position = 71.0;
        app.status.duration = 207.0;
        app.status.error = None;
    };

    for name in &names {
        let index = crate::theme::index_of(name)
            .or_else(|| crate::theme::index_of(&name.replace('-', " ")))
            .unwrap_or_else(|| panic!("no theme called {name}"));
        let dir = format!("{out}/{}", name.to_lowercase().replace([' ', '-'], "_"));
        fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(Some(music.clone()));
        app.theme = index;
        playing(&mut app);
        let scene = |app: &mut App, file: &str| {
            playing(app);
            app.toast = None;
            let buf = draw_at(app, w, h);
            dump(&buf, format!("{dir}/{file}.window"));
        };

        draw_at(&mut app, w, h);
        let tap = app.tap.clone();
        let mut clock = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (columns, segments) = (((w - 2 + 1) / 3) as usize, crate::ui::analyzer_rows(h, false) - 1);
        run_analyzer(app.visuals.spectrum(), &tap, columns, segments, 0.0, 1.3, |t| (synth_music(t) * 2.2).clamp(-1.0, 1.0), &mut clock);

        click(&mut app, |t| *t == Target::Menu(1));
        draw_at(&mut app, w, h);
        let at = app.menu_items(1).iter().position(|i| i.action == Some(Action::Library)).unwrap();
        let item = rect_of(&app, |t| *t == Target::MenuItem(at));
        app.on_mouse(MouseEvent { kind: MouseEventKind::Moved, column: item.x + 3, row: item.y, modifiers: KeyModifiers::NONE });
        scene(&mut app, "1_playback_menu");

        app.library = Some(crate::library::View::reading_at(1_204, 5_000));
        app.menu = None;
        scene(&mut app, "2_reading");

        app.library = None;
        key(&mut app, KeyCode::Char('i'));
        wait_for_library(&mut app);
        scene(&mut app, "3_opened");

        let rock = lib_row(&app, 0, "Rock");
        click(&mut app, |t| *t == Target::LibraryRow { col: 0, row: rock });
        draw_at(&mut app, w, h);
        scene(&mut app, "4_rock");

        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        scene(&mut app, "5_pink_floyd");

        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Down);
        scene(&mut app, "6_songs_of_an_album");

        draw_at(&mut app, w, h);
        let r = rect_of(&app, |t| matches!(t, Target::LibraryRow { col: 3, row: 1 }));
        click_at(&mut app, r.x + 14, r.y, MouseButton::Right);
        scene(&mut app, "7_menu_of_a_song");
        key(&mut app, KeyCode::Esc);

        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Right);
        scene(&mut app, "8_songs_of_an_artist");

        key(&mut app, KeyCode::Char('/'));
        wait_for_search(&mut app);
        type_text(&mut app, "queen greatest");
        scene(&mut app, "9_search_tags");
        key(&mut app, KeyCode::Esc);
    }
    let _ = fs::remove_dir_all(&root);
}

fn folder_order(app: &App) -> Vec<String> {
    app.levels[app.focus]
        .entries
        .iter()
        .filter(|e| e.kind == crate::fsx::Kind::Track)
        .map(|e| e.name[..2].to_string())
        .collect()
}

#[test]
fn songs_in_a_folder_can_be_reordered_and_the_order_is_remembered() {
    let _g = serial();
    let root = library();
    let (album, _) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.restore_order();
    app.navigate_to(&album);
    render(&mut app);
    let col = app.focus;
    assert_eq!(folder_order(&app), ["01", "02", "10"]);

    click(&mut app, |t| *t == Target::Row { col, row: 0 });
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(folder_order(&app), ["02", "01", "10"]);
    assert_eq!(app.levels[col].selected, Some(1));
    key(&mut app, KeyCode::Char('J'));
    key(&mut app, KeyCode::Char('J'));
    assert_eq!(folder_order(&app), ["02", "10", "01"]);
    assert_eq!(app.levels[col].selected, Some(2));

    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    key(&mut app, KeyCode::Char('K'));
    key(&mut app, KeyCode::Char('K'));
    assert_eq!(folder_order(&app), ["01", "02", "10"]);
    assert_eq!(app.levels[col].selected, Some(0));

    let labels = |app: &App| app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col, row: 0 });
    key(&mut app, KeyCode::Char('m'));
    let menu = labels(&app);
    assert!(menu.contains(&"Move Down".to_string()) && !menu.contains(&"Move Up".to_string()), "{menu:?}");
    assert!(app.menu.as_ref().unwrap().items.iter().all(|i| i.label != "Reset Order"), "{menu:?}");
    let down = app.menu.as_ref().unwrap().items.iter().find(|i| i.label == "Move Down").and_then(|i| i.action.clone()).unwrap();
    app.run(down);
    assert_eq!(folder_order(&app), ["02", "01", "10"]);
    key(&mut app, KeyCode::Char('m'));
    let menu = labels(&app);
    assert!(menu.contains(&"Move Up".to_string()) && menu.contains(&"Move Down".to_string()) && menu.contains(&"Reset Order".to_string()), "{menu:?}");
    key(&mut app, KeyCode::Esc);

    render(&mut app);
    let (first, last) = (rect_of(&app, |t| *t == Target::Row { col, row: 0 }), rect_of(&app, |t| *t == Target::Row { col, row: 2 }));
    let at = |kind, r: ratatui::layout::Rect| MouseEvent { kind, column: r.x + 2, row: r.y, modifiers: KeyModifiers::NONE };
    app.on_mouse(at(MouseEventKind::Down(MouseButton::Left), first));
    app.on_mouse(at(MouseEventKind::Drag(MouseButton::Left), last));
    app.on_mouse(at(MouseEventKind::Up(MouseButton::Left), last));
    assert_eq!(folder_order(&app), ["01", "10", "02"]);
    assert_eq!(app.levels[col].selected, Some(2));
    assert!(app.drag.is_none());
    render(&mut app);
    click(&mut app, |t| *t == Target::Row { col, row: 1 });
    assert_eq!(folder_order(&app), ["01", "10", "02"]);

    app.save_waiting();
    let mut again = App::new(Some(root.join("Music")));
    again.restore_order();
    again.navigate_to(&album);
    assert_eq!(folder_order(&again), ["01", "10", "02"]);

    again.navigate_to(album.parent().unwrap());
    render(&mut again);
    let row = row_named(&again, again.focus, "Complete Discography");
    let pcol = again.focus;
    click(&mut again, |t| *t == Target::Row { col: pcol, row });
    again.run(Action::PlaySelection);
    wait_for_queue(&mut again, 3);
    assert_eq!(queue_order(&again), ["01", "10", "02"]);
    again.engine.stop();

    again.navigate_to(&album);
    render(&mut again);
    again.run(Action::ResetOrder);
    assert_eq!(folder_order(&again), ["01", "02", "10"]);
    again.save_waiting();
    let mut last_run = App::new(Some(root.join("Music")));
    last_run.restore_order();
    last_run.navigate_to(&album);
    assert_eq!(folder_order(&last_run), ["01", "02", "10"]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn crossfade_and_replaygain_are_set_from_the_settings_menu_by_mouse_or_keyboard() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    render_at(&mut app, 90, 30);

    click(&mut app, |t| *t == Target::Menu(2));
    let screen = render_at(&mut app, 90, 30);
    assert!(screen.contains("Crossfade: Off") && screen.contains("ReplayGain: Off"), "{screen}");
    let i = app.menu.as_ref().unwrap().items.iter().position(|it| it.action == Some(Action::CrossfadeMenu)).unwrap();
    click(&mut app, |t| *t == Target::MenuItem(i));
    let screen = render_at(&mut app, 90, 30);
    println!("{screen}");
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|it| it.label.clone()).collect();
    assert_eq!(labels, ["Off", "2 seconds", "4 seconds", "6 seconds", "8 seconds", "10 seconds", "12 seconds"]);
    let six = labels.iter().position(|l| l == "6 seconds").unwrap();
    click(&mut app, |t| *t == Target::MenuItem(six));
    assert_eq!(app.settings.crossfade, 6);
    assert_eq!(crate::settings::Settings::load().crossfade, 6, "remembered across restarts");

    app.run(Action::ReplayGainMenu);
    let album = app.menu.as_ref().unwrap().items.iter().position(|it| it.label == "Album").unwrap();
    while app.menu.as_ref().unwrap().hover != Some(album) {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.replaygain, crate::audio::ReplayGain::Album);
    assert_eq!(crate::settings::Settings::load().replaygain, crate::audio::ReplayGain::Album);
    click(&mut app, |t| *t == Target::Menu(2));
    let screen = render_at(&mut app, 90, 30);
    println!("{screen}");
    assert!(screen.contains("Crossfade: 6 s") && screen.contains("ReplayGain: Album"), "{screen}");
    let _ = fs::remove_dir_all(&root);
}

fn feed_music(tap: &crate::spectrum::Tap, from: &mut u64, ms: u64, loud: bool) {
    use std::f32::consts::TAU;
    let rate = 48_000u64;
    let mut w = tap.writer();
    for _ in 0..ms * rate / 1000 {
        let t = *from as f32 / rate as f32;
        let beat = (*from / (rate / 20)).wrapping_mul(2_654_435_761) % 1000;
        let a = 0.1 + 0.8 * beat as f32 / 1000.0;
        let x = if loud {
            0.3 * a * (TAU * 60.0 * t).sin() + 0.2 * (1.0 - a) * (TAU * 1_000.0 * t).sin() + 0.15 * a * (TAU * 6_000.0 * t).sin()
        } else {
            0.0
        };
        w.push(x);
        *from += 1;
    }
}

#[test]
fn a_pause_freezes_every_visualizer_and_a_lost_output_does_not() {
    use crate::audio::State;
    use std::sync::Arc;
    use std::time::Duration;
    let _g = serial();
    let root = library();
    let song = root.join("Music/punk/hardcore/Minor Threat/Complete Discography/01 - Filler.wav");
    let frame = Duration::from_millis(33);
    for visual in [
        Visual::Spectrum,
        Visual::Trace,
        Visual::Bokeh,
        Visual::BokehLight,
        Visual::Lines,
        Visual::Synthwave,
        Visual::SynthMoonWave,
        Visual::Galaxy,
        Visual::ClubLights,
        Visual::Oscilloscope,
        Visual::Vectorscope,
        Visual::Spectroscope,
    ] {
        let mut app = App::new(Some(root.join("Music")));
        app.settings.analyzer = true;
        app.settings.visual = visual;
        app.tap.set_rate(48_000);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !app.engine.caught_up(&app.engine.status()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        app.status = app.engine.status();
        app.status.queue = Arc::new(vec![song.clone()]);
        app.status.index = 0;
        let mut at = 0u64;
        let tap = app.tap.clone();
        let slot = ui::Layout::new(ratatui::layout::Rect::new(0, 0, 90, 30), app.settings.flipped, true).analyzer;
        assert!(slot.height > 0);
        let mut step = |app: &mut App, loud: bool| {
            std::thread::sleep(frame);
            feed_music(&tap, &mut at, 33, loud);
            app.follow_pause();
            let buf = draw_at(app, 90, 30);
            let mut cells = Vec::new();
            for y in slot.y..slot.y + slot.height {
                for x in slot.x..slot.x + slot.width {
                    cells.push(buf[(x, y)].clone());
                }
            }
            cells
        };

        app.status.state = State::Playing;
        for _ in 0..15 {
            step(&mut app, true);
        }
        app.status.state = State::Paused;
        let frozen = step(&mut app, false);
        assert!(app.frame_interval() >= Duration::from_millis(500), "{visual:?}: a frozen picture rests the screen");
        for i in 0..15 {
            assert!(step(&mut app, false) == frozen, "{visual:?}: the picture moved {i} frames into the pause");
        }
        app.status.state = State::Playing;
        let first = step(&mut app, true);
        assert!(first == frozen, "{visual:?}: play jumped ahead instead of carrying on from the paused picture");
        let mut moved = false;
        let mut playing = frozen.clone();
        for _ in 0..12 {
            playing = step(&mut app, true);
            moved |= playing != frozen;
        }
        assert!(moved, "{visual:?}: did not move again after the pause");
        app.status.state = State::Paused;
        app.status.output_lost = true;
        let held = step(&mut app, false);
        let mut moved = false;
        for _ in 0..25 {
            moved |= step(&mut app, false) != held;
        }
        if matches!(visual, Visual::Oscilloscope | Visual::Vectorscope | Visual::Spectroscope) {
            moved |= held != playing;
        }
        assert!(moved, "{visual:?}: a lost output froze the picture like a pause");
    }
}

#[test]
fn only_the_same_song_paused_keeps_the_visualizer_frozen() {
    use crate::audio::State;
    use std::sync::Arc;
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    app.status.queue = Arc::new(vec![root.join("a.wav"), root.join("b.wav")]);
    let frozen = |app: &mut App, state: State, index: usize, lost: bool| {
        app.status.state = state;
        app.status.index = index;
        app.status.output_lost = lost;
        app.follow_pause();
        app.viz_hold()
    };
    assert!(!frozen(&mut app, State::Playing, 0, false));
    assert!(frozen(&mut app, State::Paused, 0, false), "paused");
    assert!(frozen(&mut app, State::Paused, 0, false), "still paused");
    assert!(frozen(&mut app, State::Playing, 0, false), "the frame play resumes on holds once more");
    assert!(!frozen(&mut app, State::Playing, 0, false), "then it runs");
    assert!(frozen(&mut app, State::Paused, 0, false));
    assert!(frozen(&mut app, State::Paused, 1, false), "another song: the last held frame");
    assert!(!frozen(&mut app, State::Paused, 1, false), "another song, still paused: not frozen");
    assert!(!frozen(&mut app, State::Stopped, 1, false), "stopped");
    assert!(frozen(&mut app, State::Paused, 1, false), "paused again");
    frozen(&mut app, State::Paused, 1, true);
    assert!(!frozen(&mut app, State::Paused, 1, true), "the output went away: a stop");
}

fn albums_library() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("urchin_test_albums_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let rock = root.join("Music/rock");
    for d in ["A", "B/CD1", "B/CD2", "C"] {
        fs::create_dir_all(rock.join(d)).unwrap();
    }
    for f in ["A/01 a.wav", "A/02 a.wav", "A/03 a.wav", "B/CD1/01 b.wav", "B/CD1/02 b.wav", "B/CD2/01 b.wav"] {
        write_wav(&rock.join(f), 44_100, 0.1);
    }
    for (file, album) in [("x1", "X"), ("x2", "X"), ("y1", "Y")] {
        let wav = crate::library::wav_with_tags(&[("INAM", file), ("IART", "Someone"), ("IPRD", album)]);
        fs::write(rock.join(format!("C/{file}.wav")), wav).unwrap();
    }
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    (root, rock)
}

fn album_of(p: &Path) -> &'static str {
    let s = p.to_string_lossy();
    if s.contains("/A/") {
        "A"
    } else if s.contains("/B/") {
        "B"
    } else if s.ends_with("/y1.wav") {
        "Y"
    } else {
        "X"
    }
}

fn albums_in(queue: &[PathBuf]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for p in queue {
        let a = album_of(p);
        if order.last() != Some(&a) {
            assert!(!order.contains(&a), "album {a} was split: {queue:?}");
            order.push(a);
        }
    }
    for a in ["A", "B", "X"] {
        let songs: Vec<&PathBuf> = queue.iter().filter(|p| album_of(p) == a).collect();
        let mut sorted = songs.clone();
        sorted.sort();
        assert_eq!(songs, sorted, "album {a} out of order");
    }
    order
}

#[test]
fn shuffle_albums_mixes_whole_albums_and_next_album_skips_the_rest() {
    let _g = serial();
    let (root, rock) = albums_library();
    let mut app = App::new(Some(root.join("Music")));

    let item = |app: &App| app.menu_items(1).into_iter().find(|i| i.label == "Shuffle Albums").unwrap().checked;
    assert_eq!(item(&app), Some(false));
    app.run(Action::ToggleShuffleAlbums);
    assert_eq!(item(&app), Some(true));
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "shuffle now mixes whole albums"));
    assert!(crate::settings::Settings::load().shuffle_albums);

    render(&mut app);
    app.navigate_to(&rock);
    key(&mut app, KeyCode::Left);
    render(&mut app);
    assert_eq!(app.selected_entry().map(|e| e.name.as_str()), Some("rock"));
    let mut orders = std::collections::HashSet::new();
    for _ in 0..15 {
        app.run(Action::ShuffleSelection);
        wait_for_queue(&mut app, 9);
        let order = albums_in(&app.status.queue);
        assert_eq!(order.len(), 4, "{order:?}");
        orders.insert(order);
        app.engine.stop();
        app.status.queue = Default::default();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    assert!(orders.len() > 1, "fifteen shuffles gave one album order");
    let apart = |o: &Vec<&str>| o.iter().position(|a| *a == "Y") != o.iter().position(|a| *a == "X").map(|x| x + 1);
    assert!(orders.iter().any(apart), "Y always followed X: {orders:?}");

    let in_order: Vec<PathBuf> = crate::fsx::collect_tracks(&rock);
    assert_eq!(albums_in(&in_order), ["A", "B", "X", "Y"]);
    load_queue(&mut app, &in_order);
    app.run(Action::ShuffleQueue);
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "shuffled 4 albums"), "{:?}", app.toast);
    assert_eq!(albums_in(&app.status.queue).len(), 4);

    app.status.queue = std::sync::Arc::new(in_order.clone());
    app.status.state = crate::audio::State::Playing;
    for (at, to) in [(0, 3), (2, 3), (3, 6), (5, 6), (6, 8)] {
        app.status.index = at;
        assert_eq!(app.next_album_index(), Ok(to), "from song {at}");
    }
    app.status.index = 8;
    assert_eq!(app.next_album_index(), Err("this is the last album in the queue"));
    app.run(Action::ToggleRepeat(crate::audio::Repeat::All));
    assert_eq!(app.next_album_index(), Ok(0), "Repeat All goes back to the first album");
    app.status.state = crate::audio::State::Stopped;
    key(&mut app, KeyCode::Char('A'));
    assert!(app.toast.as_ref().is_some_and(|(m, _)| m == "nothing is playing"));

    app.run(Action::ToggleShuffleAlbums);
    assert!(!crate::settings::Settings::load().shuffle_albums);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pane_widths_keep_every_pane_usable() {
    use crate::ui::{pane_widths, split_at, MIN_PANE_W};
    assert_eq!(pane_widths(90, &[500]), [45, 45]);
    assert_eq!(pane_widths(90, &[0]), [MIN_PANE_W, 90 - MIN_PANE_W]);
    assert_eq!(pane_widths(90, &[1000]), [90 - MIN_PANE_W, MIN_PANE_W]);
    assert_eq!(pane_widths(90, &[333, 667]), [30, 30, 30]);
    assert_eq!(pane_widths(90, &[900, 100]), [66, 12, 12]);
    assert_eq!(pane_widths(20, &[0]), [10, 10]);
    for total in [24u16, 40, 90, 213, 400] {
        for edge in MIN_PANE_W..=total - MIN_PANE_W {
            assert_eq!(pane_widths(total, &[split_at(total, edge)])[0], edge, "{total} {edge}");
        }
    }
}

#[test]
fn dragging_pane_dividers_and_the_resize_keys() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    let left_w = |app: &App| app.hits.iter().find(|h| matches!(h.target, Target::Column(_))).unwrap().rect.width;
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    let d = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Folders));
    assert_eq!((d.x, d.width), (44, 2), "the two touching border lines");
    assert_eq!(left_w(&app), 45);
    let buf = draw_at(&mut app, w, h);
    for y in d.y + 1..d.y + d.height - 1 {
        assert_eq!((buf[(44, y)].symbol(), buf[(45, y)].symbol()), ("│", "│"), "row {y}");
    }

    let y = d.y + 5;
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 45, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 40, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 35, y));
    assert_eq!(app.pointer_shape(), "col-resize");
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 35, y));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert_eq!(left_w(&app), 35, "the line under the pointer followed it");
    assert!(crate::settings::Settings::load().folder_split < 400, "remembered");
    let d = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Folders));
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), d.x, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 0, y));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 0, y));
    render_at(&mut app, w, h);
    assert_eq!(left_w(&app), crate::ui::MIN_PANE_W);

    for _ in 0..3 {
        key(&mut app, KeyCode::Char('>'));
    }
    app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    render_at(&mut app, w, h);
    assert_eq!(left_w(&app), crate::ui::MIN_PANE_W + 4);
    key(&mut app, KeyCode::Char('<'));
    app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    render_at(&mut app, w, h);
    assert_eq!(left_w(&app), crate::ui::MIN_PANE_W + 2);

    let d = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Folders));
    click_at(&mut app, d.x, y, MouseButton::Left);
    click_at(&mut app, d.x, y, MouseButton::Left);
    render_at(&mut app, w, h);
    assert_eq!(left_w(&app), 45);

    let auto = app.analyzer_rect.height;
    let v = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual));
    assert_eq!(v.y, app.analyzer_rect.y + auto);
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 20, v.y));
    assert_eq!(app.pointer_shape(), "row-resize");
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 20, v.y - 4));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 20, v.y - 4));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert_eq!(app.analyzer_rect.height, auto - 4);
    assert_eq!(app.pointer_shape(), "default", "let go away from it");
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto - 2);
    app.on_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::ALT));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto - 3);
    for _ in 0..40 {
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    }
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, 5);
    for _ in 0..40 {
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    }
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert_eq!(app.panes_rect.height, 5);

    app.run(Action::SetFlipped(true));
    app.settings.visual_rows = 0;
    render_at(&mut app, w, h);
    let auto = app.analyzer_rect.height;
    let v = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual));
    assert_eq!(v.y + 1, app.analyzer_rect.y, "the panes' bottom border");
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto + 1);
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 20, v.y - 1));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 20, v.y + 2));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 20, v.y + 2));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto - 2);
    app.run(Action::SetFlipped(false));

    app.settings.visual_rows = 0;
    app.run(Action::Library);
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    let lib = |app: &App, i| rect_of(app, |t| *t == Target::Divider(crate::app::Divider::Library(i))).x;
    let (a, b) = (lib(&app, 0), lib(&app, 1));
    key(&mut app, KeyCode::Char('>'));
    render_at(&mut app, w, h);
    assert_eq!((lib(&app, 0), lib(&app, 1)), (a + 1, b));
    let r = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Library(1)));
    let y = r.y + 3;
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), r.x, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), r.x + 6, y));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), r.x + 6, y));
    render_at(&mut app, w, h);
    assert_eq!(lib(&app, 1), b + 6);
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), b + 6, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 0, y));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 0, y));
    render_at(&mut app, w, h);
    assert_eq!(lib(&app, 1), lib(&app, 0) + crate::ui::MIN_PANE_W);
    let _ = fs::remove_dir_all(&root);
}

fn names_of(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.file_name().unwrap().to_string_lossy()[..2].to_string()).collect()
}

#[test]
fn ticking_rows_with_the_mouse_and_x_and_acting_on_them() {
    use crate::app::TickRow;
    let _g = serial();
    let root = library();
    let (album, songs) = album_songs(&root);
    for n in ["04 - Four", "05 - Five"] {
        write_wav(&album.join(format!("{n}.wav")), 44_100, 0.25);
    }
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);
    app.navigate_to(&album);
    let col = app.focus;
    let screen = render(&mut app);
    println!("{screen}");
    let row = |app: &App, name: &str| app.levels[col].entries.iter().position(|e| e.name.starts_with(name)).unwrap();
    let ticked = |app: &App| app.levels[col].marks.sorted();

    app.select(col, row(&app, "01"));
    let (r02, r04) = (row(&app, "02"), row(&app, "04"));
    click(&mut app, |t| *t == Target::Tick(TickRow::Folder { col, row: r02 }));
    render(&mut app);
    click(&mut app, |t| *t == Target::Tick(TickRow::Folder { col, row: r04 }));
    assert_eq!(ticked(&app), [r02, r04]);
    assert_eq!(app.levels[col].selected, Some(row(&app, "01")));
    let screen = render(&mut app);
    println!("{screen}");
    assert!(screen.contains("· 2 ticked"), "{screen}");
    assert!(line_with(&screen, "I Don't Wanna").contains('✓'));
    assert!(!line_with(&screen, "Filler").contains('✓'));
    let r = rect_of(&app, |t| *t == Target::Tick(TickRow::Folder { col, row: row(&app, "05") }));
    app.on_mouse(MouseEvent { kind: MouseEventKind::Moved, column: r.x, row: r.y, modifiers: KeyModifiers::NONE });
    assert!(line_with(&render(&mut app), "Five").contains('✓'));

    key(&mut app, KeyCode::Enter);
    wait_for_queue(&mut app, 2);
    assert_eq!(queue_order(&app), ["02", "04"]);
    app.engine.stop();

    render(&mut app);
    click_at(&mut app, r.x + 6, r.y, MouseButton::Right);
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(labels[0], "2 ticked");
    assert!(labels.contains(&"Clear Ticks".to_string()) && labels.contains(&"Add to Queue".to_string()), "{labels:?}");
    key(&mut app, KeyCode::Esc);

    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    let order: Vec<String> = app.levels[col].entries.iter().map(|e| e.name[..2].to_string()).collect();
    assert_eq!(order, ["01", "05", "02", "04", "10"]);
    assert_eq!(ticked(&app), [row(&app, "02"), row(&app, "04")]);

    key(&mut app, KeyCode::Esc);
    assert!(ticked(&app).is_empty());

    render(&mut app);
    let (a, b) = (row(&app, "01"), row(&app, "02"));
    let ra = rect_of(&app, |t| *t == Target::Tick(TickRow::Folder { col, row: a }));
    let rb = rect_of(&app, |t| *t == Target::Row { col, row: b });
    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), ra.x, ra.y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), rb.x + 5, rb.y));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), rb.x + 5, rb.y));
    assert_eq!(ticked(&app), (a.min(b)..=a.max(b)).collect::<Vec<_>>());
    key(&mut app, KeyCode::Esc);

    app.select(col, 0);
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(app.levels[col].selected, Some(1));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('X'));
    assert_eq!(ticked(&app), [0, 1, 2, 3]);
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(ticked(&app), [0, 1, 2]);
    let before = app.status.queue.len();
    key(&mut app, KeyCode::Char('Q'));
    assert_eq!(app.status.queue.len(), before + 3);
    key(&mut app, KeyCode::Esc);

    render(&mut app);
    let at = |app: &App, r: usize| rect_of(app, |t| *t == Target::Row { col, row: r });
    let (r1, r3) = (at(&app, 1), at(&app, 3));
    app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r1.x + 6, row: r1.y, modifiers: KeyModifiers::CONTROL });
    app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r3.x + 6, row: r3.y, modifiers: KeyModifiers::SHIFT });
    assert_eq!(ticked(&app), [1, 2, 3]);
    key(&mut app, KeyCode::Esc);

    app.engine.stop();
    load_queue(&mut app, &songs);
    app.run(Action::Queue);
    render(&mut app);
    click(&mut app, |t| *t == Target::Tick(TickRow::Queue(0)));
    click(&mut app, |t| *t == Target::Tick(TickRow::Queue(2)));
    assert!(render(&mut app).contains("Queue · 3 songs · 2 ticked"));
    key(&mut app, KeyCode::Delete);
    assert_eq!(queue_order(&app), ["02"]);
    app.run(Action::Queue);

    let playlist = crate::playlists::create_in("Picked", &crate::playlists::dir()).unwrap();
    render(&mut app);
    app.select(col, row(&app, "01"));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Char('a'));
    let i = app.menu.as_ref().unwrap().items.iter().position(|it| it.label == "Picked").unwrap();
    let action = app.menu.as_ref().unwrap().items[i].action.clone().unwrap();
    assert!(matches!(action, Action::AddManyToPlaylist { .. }));
    app.run(action);
    let pl = crate::m3u::Playlist::open(&playlist).unwrap();
    assert_eq!(names_of(&pl.tracks(crate::fsx::is_audio).paths), ["01", "05"]);

    key(&mut app, KeyCode::Esc);
    app.run(Action::Search);
    wait_for_search(&mut app);
    type_text(&mut app, "filler");
    app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    for _ in 0.."filler".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "seeing");
    let screen = render(&mut app);
    assert!(screen.contains("1 ticked ·"), "{screen}");
    click(&mut app, |t| *t == Target::Tick(TickRow::Search(0)));
    assert_eq!(names_of(&search_of(&app).ticked_paths()), ["01", "10"]);
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    wait_for_queue(&mut app, 2);
    assert_eq!(queue_order(&app), ["01", "10"]);
    app.engine.stop();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn ticking_songs_of_a_playlist() {
    use crate::app::TickRow;
    let _g = serial();
    let root = library();
    let (_, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    let playlist = crate::playlists::create_in("Three", &crate::playlists::dir()).unwrap();
    for s in &songs {
        crate::playlists::add_song(&playlist, s).unwrap();
    }
    app.run(Action::Playlists);
    render(&mut app);
    click(&mut app, |t| *t == Target::Tick(TickRow::Playlist(2)));
    assert!(app.playlists.as_ref().unwrap().on_songs);
    key(&mut app, KeyCode::Char('x'));
    let screen = render(&mut app);
    println!("{screen}");
    assert!(screen.contains("Three · 2 ticked"), "{screen}");
    key(&mut app, KeyCode::Enter);
    wait_for_queue(&mut app, 2);
    assert_eq!(queue_order(&app), ["01", "10"]);
    app.engine.stop();
    key(&mut app, KeyCode::Esc);
    assert!(app.playlists.is_some() && app.tick_count() == 0);
    key(&mut app, KeyCode::Esc);
    assert!(app.playlists.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn your_own_presets_are_saved_named_picked_updated_renamed_and_deleted() {
    use crate::eq::{Source, Store};
    let _g = serial();
    let root = library();
    let dir = root.join("Music/mine");
    let other = root.join("Music/other");
    fs::create_dir_all(&dir).unwrap();
    fs::create_dir_all(&other).unwrap();
    let song = |d: &Path, name: &str| {
        let p = d.join(format!("{name}.wav"));
        write_wav(&p, 44_100, 0.2);
        p
    };
    let (a, b, c) = (song(&dir, "a"), song(&dir, "b"), song(&dir, "c"));
    let z = song(&other, "z");
    let mut app = App::new(Some(root.join("Music")));
    let play = |app: &mut App, s: &Path| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![s.to_path_buf()]);
        app.status.index = 0;
    };
    let curve = |app: &App, s: &Path| app.eq_store.lock().unwrap().get(s);
    let label = |app: &App, s: &Path| {
        let st = app.eq_store.lock().unwrap();
        st.label(&st.get(s))
    };
    let toast = |app: &App| app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    let type_name = |app: &mut App, name: &str| {
        for ch in name.chars() {
            key(app, KeyCode::Char(ch));
        }
    };

    play(&mut app, &a);
    app.run(Action::Equalizer);
    key(&mut app, KeyCode::Char('s'));
    assert!(toast(&app).contains("flat"), "{}", toast(&app));
    assert!(matches!(app.overlay, Some(Overlay::Equalizer)));
    key(&mut app, KeyCode::Right);
    for _ in 0..8 {
        key(&mut app, KeyCode::Up);
    }
    let shaped = curve(&app, &a);

    key(&mut app, KeyCode::Char('s'));
    assert!(render(&mut app).contains("Save as Preset"));
    type_name(&mut app, "rock");
    key(&mut app, KeyCode::Enter);
    assert_eq!(toast(&app), "Rock is a built-in preset; pick another name");
    assert!(matches!(app.overlay, Some(Overlay::PlaylistName(..))));
    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    type_name(&mut app, "My Bass");
    key(&mut app, KeyCode::Enter);
    assert_eq!(toast(&app), "saved preset My Bass");
    assert!(matches!(app.overlay, Some(Overlay::Equalizer)), "back in the equalizer");
    assert_eq!(label(&app, &a), "My Bass");
    let id = app.eq_store.lock().unwrap().find_mine("my bass").unwrap();
    assert!(render(&mut app).contains("Preset: My Bass ▾"));

    click(&mut app, |t| *t == Target::EqPreset);
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(&labels[..5], ["Save as Preset…", "", "Your Presets", "My Bass", "Built-in"]);
    key(&mut app, KeyCode::Esc);

    play(&mut app, &z);
    app.run(Action::EqPreset(id as usize));
    play(&mut app, &b);
    app.run(Action::EqPreset(id as usize));
    assert_eq!(app.eq_store.lock().unwrap().source(&c), Source::Folder(id));
    assert_eq!(curve(&app, &c).gains, shaped.gains);
    play(&mut app, &z);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Up);
    assert_eq!(label(&app, &z), "My Bass (customized)");
    let z_own = curve(&app, &z);

    play(&mut app, &a);
    key(&mut app, KeyCode::Left);
    for _ in 0..4 {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(label(&app, &a), "My Bass (customized)");
    key(&mut app, KeyCode::Char('s'));
    assert!(render(&mut app).contains("My Bass▌"), "the name box starts with the preset's name");
    key(&mut app, KeyCode::Enter);
    assert_eq!(toast(&app), "updated preset My Bass; folders that use it follow");
    let updated = curve(&app, &a);
    assert_eq!(curve(&app, &b).gains, updated.gains);
    assert_eq!(curve(&app, &c).gains, updated.gains);
    assert_eq!(curve(&app, &z), z_own);

    render(&mut app);
    click(&mut app, |t| *t == Target::EqPreset);
    assert_eq!(app.menu.as_ref().unwrap().items[app.menu.as_ref().unwrap().hover.unwrap()].label, "My Bass");
    key(&mut app, KeyCode::F(2));
    assert!(render(&mut app).contains("Rename Preset"));
    for _ in 0.."My Bass".len() {
        key(&mut app, KeyCode::Backspace);
    }
    type_name(&mut app, "Warm");
    key(&mut app, KeyCode::Enter);
    assert_eq!(toast(&app), "renamed preset My Bass to Warm");
    assert_eq!(label(&app, &b), "Warm");
    assert_eq!(app.eq_store.lock().unwrap().source(&c), Source::Folder(id));

    key(&mut app, KeyCode::Esc);
    let store = Store::load();
    assert_eq!(store.preset_name(id), "Warm");
    assert_eq!(store.source(&c), Source::Folder(id));
    drop(store);

    app.run(Action::Equalizer);
    render(&mut app);
    click(&mut app, |t| *t == Target::EqPreset);
    render(&mut app);
    let row = app.menu.as_ref().unwrap().items.iter().position(|i| i.label == "Warm").unwrap();
    let r = rect_of(&app, |t| *t == Target::MenuItem(row));
    app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: r.x + 2, row: r.y, modifiers: KeyModifiers::NONE });
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(labels, ["Rename…", "Delete"]);
    app.run(Action::EqDeletePreset(id));
    assert_eq!(toast(&app), "deleted preset Warm; the 2 folders that used it are flat again");
    let st = app.eq_store.lock().unwrap();
    assert!(st.my_presets().is_empty());
    for s in [&a, &b, &c] {
        assert_eq!(st.source(s), Source::None, "{s:?}");
    }
    assert_eq!(st.get(&z).gains, z_own.gains);
    assert_eq!(st.label(&st.get(&z)), "Custom");
    drop(st);
    assert!(Store::load().my_presets().is_empty());
    let _ = fs::remove_dir_all(&root);
}

fn write_tone(path: &Path, freq: f32) {
    let rate = 44_100u32;
    let mut data = Vec::new();
    for i in 0..rate * 3 {
        let v = ((i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * 8000.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 2, 0], rate.to_le_bytes(), (rate * 4).to_le_bytes(), [4, 0, 16, 0]] {
        out.extend_from_slice(&v);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, out).unwrap();
}

#[test]
fn eq_follows_a_song_or_folder_that_was_moved_or_renamed() {
    use crate::eq::{preset_name, Store, PRESET_COUNT};
    let _g = serial();
    let root = library();
    let album = root.join("Music/rock/Band/Album");
    let songs: Vec<PathBuf> = [440.0, 550.0, 660.0, 770.0]
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let p = album.join(format!("0{} - song.wav", i + 1));
            write_tone(&p, f);
            p
        })
        .collect();
    let rnb = (0..PRESET_COUNT).find(|&i| preset_name(i) == "R&B").unwrap();
    let play = |app: &mut App, s: &Path| {
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![s.to_path_buf()]);
        app.status.index = 0;
    };
    let settle = |app: &mut App, done: &dyn Fn(&App) -> bool| {
        for _ in 0..400 {
            app.follow_moves();
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("never settled");
    };

    let mut app = App::new(Some(root.join("Music")));
    app.follow_moved_files();
    play(&mut app, &songs[0]);
    app.run(Action::Equalizer);
    key(&mut app, KeyCode::Up);
    let own = app.eq_store.lock().unwrap().get(&songs[0]);
    settle(&mut app, &|a| !a.eq_store.lock().unwrap().wants_print(&songs[0]));
    play(&mut app, &songs[1]);
    app.run(Action::EqPreset(rnb));
    settle(&mut app, &|a| {
        let s = a.eq_store.lock().unwrap();
        !s.wants_folder_prints(&album) && !s.wants_print(&songs[1])
    });
    key(&mut app, KeyCode::Esc);
    app.save_on_exit();
    let config = crate::settings::config_dir();
    let eq_file = fs::read_to_string(config.join("equalizer.json")).unwrap();
    assert_eq!(eq_file.matches("\"print\":").count(), 2, "{eq_file}");
    let folders_file = fs::read_to_string(config.join("equalizer_folders.json")).unwrap();
    assert!(folders_file.contains("\"preset\":") && folders_file.matches(',').count() >= 3, "{folders_file}");
    drop(app);

    let renamed = root.join("Music/rock/Band/Album (Remastered)");
    fs::rename(&album, &renamed).unwrap();
    let moved: Vec<PathBuf> = songs.iter().map(|s| renamed.join(s.file_name().unwrap())).collect();

    let mut app = App::new(Some(root.join("Music")));
    app.follow_moved_files();
    assert!(!app.eq_store.lock().unwrap().has(&moved[0]));
    play(&mut app, &moved[0]);
    settle(&mut app, &|a| a.eq_store.lock().unwrap().has(&moved[0]));
    assert_eq!(app.eq_store.lock().unwrap().get(&moved[0]), own);
    settle(&mut app, &|a| a.eq_store.lock().unwrap().folder_preset_for(&moved[2]).is_some());
    let both = "found this song's EQ and Album (Remastered)'s preset (R&B) again: they were moved or renamed";
    let song_only = "found this song's EQ again: the file was moved or renamed";
    let toast = app.toast.as_ref().unwrap().0.clone();
    assert!(toast == both || toast == song_only, "{toast}");
    play(&mut app, &moved[2]);
    settle(&mut app, &|a| a.eq_store.lock().unwrap().folder_preset_for(&moved[2]).is_some());
    assert_eq!(app.eq_store.lock().unwrap().folder_preset_for(&moved[2]), Some(rnb as u8));
    if toast == song_only {
        assert_eq!(app.toast.as_ref().unwrap().0, "found Album (Remastered)'s preset (R&B) again: the folder was moved or renamed");
    }
    app.save_on_exit();
    let store = Store::load();
    assert_eq!(store.get(&moved[0]), own);
    assert_eq!(store.folder_preset_for(&moved[3]), Some(rnb as u8));
    assert!(!store.has(&songs[0]) && store.folder_preset_for(&songs[3]).is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn eq_files_from_before_fingerprints_still_load_and_an_unplugged_drive_keeps_its_eq() {
    use crate::eq::{Curve, Store};
    let _g = serial();
    let root = library();
    let config = crate::settings::config_dir();
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("equalizer.json"), r#"{"/m/a.flac":{"pre":0.0,"gains":[1.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0],"based_on":3}}"#).unwrap();
    fs::write(config.join("equalizer_folders.json"), r#"{"/m":13}"#).unwrap();
    let store = Store::load();
    assert_eq!(store.get(Path::new("/m/a.flac")).gains[0], 1.0);
    assert_eq!(store.folder_preset_for(Path::new("/m/b.flac")), Some(13));
    assert!(store.wants_print(Path::new("/m/a.flac")) && store.wants_folder_prints(Path::new("/m")));

    let curve = Curve { gains: [2.0; 10], ..Curve::default() };
    let mut s = Store::default();
    let offline = Path::new("/Volumes/Urchin_No_Such_Drive/music/a.flac");
    s.set(offline, curve);
    s.set_print(offline, "aa".repeat(16));
    assert!(!s.has_lost_songs());
    assert!(!s.relink_song(&root.join("copy.flac"), &"aa".repeat(16)));
    let here = root.join("Music/x.flac");
    fs::create_dir_all(here.parent().unwrap()).unwrap();
    fs::write(root.join("Music/keep.txt"), b"x").unwrap();
    s.set(&here, curve);
    s.set_print(&here, "bb".repeat(16));
    assert!(s.has_lost_songs());
    assert!(s.relink_song(&root.join("Music/renamed.flac"), &"bb".repeat(16)));
    assert!(s.has(&root.join("Music/renamed.flac")) && !s.has(&here));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fingerprints_an_older_decoder_made_are_made_again_once() {
    use crate::eq::Store;
    let _g = serial();
    let root = library();
    let album = root.join("Music/rock/Band/Album");
    let songs: Vec<PathBuf> = [440.0, 550.0].iter().enumerate().map(|(i, &f)| {
        let p = album.join(format!("0{} - song.wav", i + 1));
        write_tone(&p, f);
        p
    }).collect();
    let gone = root.join("Music/rock/Band/Album/09 - moved away.wav");
    let config = crate::settings::config_dir();
    fs::create_dir_all(&config).unwrap();
    let curve = r#""pre":0.0,"gains":[1.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0],"based_on":3"#;
    let old = "0".repeat(32);
    fs::write(
        config.join("equalizer.json"),
        format!(
            r#"{{"{}":{{{curve},"print":"{old}"}},"{}":{{{curve},"print":"{old}"}}}}"#,
            songs[0].display(),
            gone.display()
        ),
    )
    .unwrap();
    fs::write(config.join("equalizer_folders.json"), format!(r#"{{"{}":{{"preset":13,"prints":["{old}"]}}}}"#, album.display()))
        .unwrap();

    let store = std::sync::Mutex::new(Store::load());
    assert_eq!(store.lock().unwrap().stale_prints(), (vec![songs[0].clone()], vec![album.clone()]));
    crate::app::refresh_stale_prints(&store);
    assert_eq!(store.lock().unwrap().stale_prints(), (vec![], vec![]));

    let songs_json: serde_json::Value = serde_json::from_str(&fs::read_to_string(config.join("equalizer.json")).unwrap()).unwrap();
    let here = &songs_json[songs[0].to_str().unwrap()];
    assert_eq!(here["print"], crate::fingerprint::of(&songs[0]).unwrap().as_str());
    assert_eq!(here["print_version"], crate::fingerprint::VERSION);
    let away = &songs_json[gone.to_str().unwrap()];
    assert_eq!((away["print"].as_str(), away["print_version"].as_u64()), (Some(old.as_str()), Some(1)));
    let folders_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config.join("equalizer_folders.json")).unwrap()).unwrap();
    let folder = &folders_json[album.to_str().unwrap()];
    let expected: Vec<String> = songs.iter().map(|p| crate::fingerprint::of(p).unwrap()).collect();
    assert_eq!(folder["prints"], serde_json::json!(expected));
    assert_eq!(folder["prints_version"], crate::fingerprint::VERSION);

    assert_eq!(Store::load().stale_prints(), (vec![], vec![]));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_queue_comes_back_paused_where_it_was_left_off() {
    use crate::audio::State;
    use crate::settings::{Resume, Settings};
    let _g = serial();
    let root = library();
    let (_, songs) = album_songs(&root);
    for s in &songs {
        write_wav(s, 44_100, 3.0);
    }
    let (mut keeper, _) = crate::queue::Keeper::open(crate::queue::Keeper::default_file());
    keeper.keep(&std::sync::Arc::new(songs.clone()));
    let left_off = |song: usize, seconds: f64| {
        let mut s = Settings::load();
        s.resume = Some(Resume { song: songs[song].clone(), index: song, seconds });
        s.save();
    };
    let start = || {
        let mut app = App::new(Some(root.join("Music")));
        app.restore_queue();
        app.tick();
        app
    };

    left_off(1, 0.7);
    let mut app = start();
    assert_eq!((app.status.state, app.status.index, app.status.position), (State::Paused, 1, 0.7));
    assert_eq!(app.status.current(), Some(&songs[1]));
    assert!(!app.status.output_lost);
    let screen = render(&mut app);
    assert!(screen.contains("I Don't Wanna Hear It"), "{screen}");
    assert!(screen.contains("0:00"), "the place in the song is shown: {screen}");
    app.run(Action::Queue);
    assert_eq!(app.queue_view.as_ref().unwrap().selected, 1);
    app.save_on_exit();
    assert_eq!(Settings::load().resume, Some(Resume { song: songs[1].clone(), index: 1, seconds: 0.7 }));
    drop(app);

    let length = start().status.duration;
    assert!(length > 1.0, "{length}");
    left_off(1, length - 0.3);
    let mut app = start();
    app.save_on_exit();
    assert_eq!(Settings::load().resume, Some(Resume { song: songs[2].clone(), index: 2, seconds: 0.0 }));
    drop(app);
    left_off(2, length - 0.3);
    let mut app = start();
    app.save_on_exit();
    assert_eq!(Settings::load().resume, None);
    drop(app);

    left_off(1, 0.7);
    fs::remove_file(&songs[1]).unwrap();
    let app = start();
    assert_eq!((app.status.state, app.status.index, app.status.position), (State::Paused, 1, 0.0));
    assert_eq!(app.status.current(), Some(&songs[2]));
    drop(app);

    let mut s = Settings::load();
    s.resume = None;
    s.save();
    assert_eq!(start().status.state, State::Stopped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn when_a_folder_ends_the_next_folder_follows_on() {
    use crate::audio::{Repeat, State};
    let _g = serial();
    let root = library();
    let lib = root.join("Lib");
    for (folder, n) in [("punk/A/Album 1", 2), ("punk/A/Album 2", 2), ("punk/B/Album 3", 1), ("rock/C/Album 4", 3)] {
        fs::create_dir_all(lib.join(folder)).unwrap();
        for i in 1..=n {
            write_wav(&lib.join(folder).join(format!("0{i} - song.wav")), 44_100, 0.2);
        }
    }
    fs::create_dir_all(lib.join("punk/B/Empty")).unwrap();
    let mut s = crate::settings::Settings::load();
    s.music_root = Some(lib.clone());
    s.continue_folders = true;
    s.save();

    let mut app = App::new(Some(lib.clone()));
    app.navigate_to(&lib.join("punk/A"));
    render(&mut app);
    let col = app.focus;
    let row = row_named(&app, col, "Album 1");
    click(&mut app, |t| *t == Target::Row { col, row });
    app.run(Action::PlaySelection);
    wait_for_queue(&mut app, 2);
    let on_last = |app: &mut App| {
        app.status.state = State::Playing;
        app.status.index = app.status.queue.len() - 1;
        app.continue_into_next_folder();
    };
    let folder_of = |app: &App, i: usize| app.status.queue[i].parent().unwrap().strip_prefix(&lib).unwrap().to_string_lossy().into_owned();

    on_last(&mut app);
    wait_for_queue(&mut app, 4);
    assert_eq!((folder_of(&app, 2), folder_of(&app, 3)), ("punk/A/Album 2".into(), "punk/A/Album 2".into()));
    assert_eq!(app.toast.as_ref().unwrap().0, "next up: Album 2 (2 songs)");
    on_last(&mut app);
    wait_for_queue(&mut app, 5);
    assert_eq!(folder_of(&app, 4), "punk/B/Album 3");
    on_last(&mut app);
    wait_for_queue(&mut app, 8);
    assert_eq!(folder_of(&app, 7), "rock/C/Album 4");
    on_last(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(50));
    app.tick();
    assert_eq!(app.status.queue.len(), 8);

    let count_after = |app: &mut App, setup: &dyn Fn(&mut App)| {
        let row = row_named(app, col, "Album 1");
        app.navigate_to(&lib.join("punk/A"));
        render(app);
        click(app, |t| *t == Target::Row { col, row });
        app.run(Action::PlaySelection);
        wait_for_queue(app, 2);
        setup(app);
        on_last(app);
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.tick();
        app.status.queue.len()
    };
    assert_eq!(count_after(&mut app, &|a| a.settings.repeat = Repeat::All), 2);
    assert_eq!(count_after(&mut app, &|a| {
        a.settings.repeat = Repeat::Off;
        a.run(Action::ToggleContinue);
    }), 2);
    assert!(!crate::settings::Settings::load().continue_folders, "the setting is kept");
    assert!(app.menu_items(1).iter().any(|i| i.label == "Continue Into Next Folder" && i.checked == Some(false)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn show_in_the_file_manager_and_copy_path_are_on_every_songs_menu() {
    let _g = serial();
    let root = library();
    let (album, songs) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);
    render(&mut app);
    let col = app.focus;
    let row = row_named(&app, col, "02 - I Don't Wanna Hear It.wav");
    let r = rect_of(&app, |t| *t == Target::Row { col, row });
    click_at(&mut app, r.x + 6, r.y, MouseButton::Right);
    let labels: Vec<String> = app.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    let show = crate::reveal::label();
    assert!(labels.iter().any(|l| l == show) && labels.iter().any(|l| l == "Copy Path"), "{labels:?}");

    let i = labels.iter().position(|l| l == "Copy Path").unwrap();
    render(&mut app);
    click(&mut app, |t| *t == Target::MenuItem(i));
    assert_eq!(app.osc_out, [crate::reveal::clipboard_escape(&songs[1].to_string_lossy())]);
    assert!(app.toast.as_ref().unwrap().0.starts_with("copied the path: "), "{:?}", app.toast);
    app.osc_out.clear();

    let ssh = std::env::var_os("SSH_CONNECTION");
    std::env::set_var("SSH_CONNECTION", "10.0.0.2 51000 10.0.0.3 22");
    app.run(Action::Reveal(songs[1].clone()));
    for _ in 0..200 {
        app.tick();
        if !app.osc_out.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    match ssh {
        Some(v) => std::env::set_var("SSH_CONNECTION", v),
        None => std::env::remove_var("SSH_CONNECTION"),
    }
    assert_eq!(app.osc_out, [crate::reveal::clipboard_escape(&songs[1].to_string_lossy())]);
    assert!(app.toast.as_ref().unwrap().0.starts_with("no desktop here to show it on; copied the path: "), "{:?}", app.toast);

    app.navigate_to(album.parent().unwrap());
    render(&mut app);
    let row = row_named(&app, app.focus, "Complete Discography");
    let r = rect_of(&app, |t| *t == Target::Row { col: app.focus, row });
    click_at(&mut app, r.x + 6, r.y, MouseButton::Right);
    assert!(app.menu.as_ref().unwrap().items.iter().any(|i| i.label == show), "folders too");
    let _ = fs::remove_dir_all(&root);
}

mod config_folder_tests {
    use super::*;
    use std::ffi::OsString;

    struct Env(Vec<(&'static str, Option<OsString>)>);
    impl Env {
        fn set(pairs: &[(&'static str, Option<&Path>)]) -> Env {
            let saved = pairs.iter().map(|(k, _)| (*k, std::env::var_os(k))).collect();
            for (k, v) in pairs {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
            Env(saved)
        }
    }
    impl Drop for Env {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    fn temp_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("urchin_config_folder_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn the_config_folder_is_where_it_has_always_been() {
        let _g = serial();
        let home = temp_home("plain");
        let _env = Env::set(&[("HOME", Some(&home)), ("XDG_CONFIG_HOME", None)]);
        #[cfg(target_os = "macos")]
        let expected = home.join("Library/Application Support/urchin");
        #[cfg(all(unix, not(target_os = "macos")))]
        let expected = home.join(".config/urchin");
        #[cfg(windows)]
        let expected = PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA is set on Windows")).join("urchin");
        assert_eq!(crate::settings::config_dir(), expected);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn xdg_config_home_wins_on_every_system() {
        let _g = serial();
        let home = temp_home("xdg");
        let xdg = home.join("elsewhere");
        let _env = Env::set(&[("HOME", Some(&home)), ("XDG_CONFIG_HOME", Some(&xdg))]);
        assert_eq!(crate::settings::config_dir(), xdg.join("urchin"));
        let _ = fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn home_and_music_folders_come_from_the_usual_places() {
        let _g = serial();
        let home = temp_home("music");
        let _env = Env::set(&[("HOME", Some(&home)), ("XDG_CONFIG_HOME", None)]);
        assert_eq!(crate::fsx::home(), home);
        assert_eq!(crate::fsx::default_music_dir(), home);
        fs::create_dir_all(home.join("Music")).unwrap();
        assert_eq!(crate::fsx::default_music_dir(), home.join("Music"));
        #[cfg(not(target_os = "macos"))]
        {
            fs::create_dir_all(home.join(".config")).unwrap();
            fs::create_dir_all(home.join("Tunes")).unwrap();
            fs::write(home.join(".config/user-dirs.dirs"), "XDG_MUSIC_DIR=\"$HOME/Tunes\"\n").unwrap();
            assert_eq!(crate::fsx::default_music_dir(), home.join("Tunes"));
        }
        let _ = fs::remove_dir_all(&home);
    }
}

#[test]
fn toolbar_buttons_show_the_symbol_first_and_only_the_symbols_when_narrow() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let toolbar = |app: &App, screen: &str| {
        let y = rect_of(app, |t| *t == Target::EqBadge).y as usize;
        screen.lines().nth(y).unwrap().to_string()
    };
    for w in [110u16, 90, 85, 61] {
        let screen = render_at(&mut app, w, 30);
        let line = toolbar(&app, &screen);
        assert!(line.ends_with(" [≋ EQ] [⇄ Shuffle] [★ Pin Folder] "), "{w} columns: {line:?}");
        assert!(!line.contains('#'), "{w} columns: {line:?}");
    }
    for w in [60u16, 45, 30] {
        let screen = render_at(&mut app, w, 30);
        let line = toolbar(&app, &screen);
        assert!(line.ends_with(" [≋] [⇄] [★] "), "{w} columns: {line:?}");
        assert!(!line.contains("Shuffle") && !line.contains("Pin Folder"), "{w} columns: {line:?}");
        for target in [Target::EqBadge, Target::ToolShuffle, Target::ToolPin] {
            assert_eq!(rect_of(&app, |t| *t == target).width, 3, "{w} columns: {target:?}");
        }
    }
    render_at(&mut app, 45, 30);
    let pinned = app.settings.pinned.len();
    click(&mut app, |t| *t == Target::ToolPin);
    assert_eq!(app.settings.pinned.len(), pinned + 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_song_renamed_outside_urchin_shows_up_and_keeps_the_highlight() {
    let _g = serial();
    let root = library();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let mut app = App::new(Some(album.clone()));
    app.check_folders();
    let names = |app: &App| app.levels[0].entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    let at = |app: &App, n: &str| app.levels[0].entries.iter().position(|e| e.name.starts_with(n));
    let i = at(&app, "02").unwrap();
    app.levels[0].selected = Some(i);
    app.levels[0].marks.set(i, true);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::rename(album.join("02 - I Don't Wanna Hear It.wav"), album.join("99 - I Dont Wanna Hear It (fixed).wav")).unwrap();
    assert!(names(&app).iter().any(|n| n.starts_with("02")));
    app.check_folders();
    assert!(!names(&app).iter().any(|n| n.starts_with("02")), "{:?}", names(&app));
    let j = at(&app, "99").expect("new name listed");
    assert_ne!(i, j, "the new name sorts elsewhere");
    assert_eq!(app.levels[0].selected, Some(j), "the highlight followed the rename");
    assert!(app.levels[0].marks.contains(&j) && app.levels[0].marks.len() == 1, "the tick followed it too");

    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::copy(album.join("01 - Filler.wav"), album.join("05 - New.wav")).unwrap();
    app.check_folders();
    assert!(at(&app, "05").is_some(), "{:?}", names(&app));
    assert!(app.levels[0].entries[app.levels[0].selected.unwrap()].name.starts_with("99 - "), "highlight still on the renamed song");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn folders_are_only_watched_after_show_in_finder_until_you_come_back() {
    let _g = serial();
    let root = library();
    let album = root.join("Music/punk/hardcore/Minor Threat/Complete Discography");
    let mut app = App::new(Some(album.clone()));
    let has = |app: &App, n: &str| app.levels[0].entries.iter().any(|e| e.name.starts_with(n));
    let later = || std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::copy(album.join("01 - Filler.wav"), album.join("05 - A.wav")).unwrap();
    later();
    app.tick();
    assert!(!has(&app, "05"), "checked without Show in Finder");
    app.run(Action::Reveal(album.join("01 - Filler.wav")));
    fs::rename(album.join("05 - A.wav"), album.join("06 - B.wav")).unwrap();
    later();
    app.tick();
    assert!(has(&app, "06") && !has(&app, "05"), "the rename made in Finder shows up");
    fs::copy(album.join("01 - Filler.wav"), album.join("07 - C.wav")).unwrap();
    app.on_focus();
    assert!(has(&app, "07"), "checked on coming back");
    fs::copy(album.join("01 - Filler.wav"), album.join("08 - D.wav")).unwrap();
    later();
    app.tick();
    app.on_focus();
    assert!(!has(&app, "08"), "still watching after coming back");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_folder_renamed_outside_urchin_while_you_are_in_it_follows_the_new_name() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music/punk")));
    let i = app.levels[0].entries.iter().position(|e| e.name == "hardcore").unwrap();
    app.select(0, i);
    assert_eq!(app.levels[1].dir, root.join("Music/punk/hardcore"));
    app.check_folders();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::rename(root.join("Music/punk/hardcore"), root.join("Music/punk/hardcore punk")).unwrap();
    app.check_folders();
    let left = &app.levels[0];
    assert_eq!(left.entries[left.selected.unwrap()].name, "hardcore punk");
    assert_eq!(app.levels[1].dir, root.join("Music/punk/hardcore punk"));
    assert!(app.levels[1].entries.iter().any(|e| e.name == "Minor Threat"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn view_items_are_in_order_and_a_fresh_install_is_on_standard_view() {
    let _g = serial();
    let root = library();
    let app = App::new(Some(root.join("Music")));
    assert!(!app.settings.flipped && !app.settings.compact);
    let items = app.menu_items(2);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    let at = labels.iter().position(|l| *l == "Standard View").expect("Standard View item");
    assert_eq!(labels[at..at + 3], ["Standard View", "Flipped View", "Minimalist View"]);
    let checked: Vec<bool> = items[at..at + 3].iter().map(|i| i.checked == Some(true)).collect();
    assert_eq!(checked, [true, false, false], "only Standard View is ticked on a fresh install");
    assert!(!labels.contains(&"Compact Mode") && !labels.contains(&"Compact View"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn compact_mode_hides_the_menu_bar_and_pane_frames() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    let row = |s: &str, y: usize| s.lines().nth(y).unwrap().to_string();
    let normal = render_at(&mut app, w, h);
    assert!(row(&normal, 0).contains("File"));
    let panes_h = app.panes_rect.height;

    app.run(crate::app::Action::ToggleCompact);
    assert!(crate::settings::Settings::load().compact, "remembered");
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert!(!row(&screen, 0).contains("File"), "no menu bar row");
    assert!(!app.hits.iter().any(|h| matches!(h.target, Target::Menu(_))));
    let p = app.panes_rect;
    assert!(app.panes_rect.height + app.analyzer_rect.height > panes_h, "the menu row went to the panes/visualizer");
    for y in p.y..p.y + p.height {
        let line: String = row(&screen, y as usize).chars().skip(p.x as usize).take(p.width as usize).collect();
        assert!(!line.contains(['╭', '╮', '╰', '╯', '│']), "row {y}: {line}");
    }
    let start: String = root.join("Music").to_string_lossy().chars().take(12).collect();
    assert!(row(&screen, p.y as usize).contains(&start), "{start}");
    let a = app.analyzer_rect;
    assert_eq!(p.y, a.y + a.height + 1);
    assert!(row(&screen, (a.y + a.height) as usize).trim().is_empty());
    assert_eq!(rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual)).y, a.y + a.height);

    let small = render_at(&mut app, w, 14);
    assert!(!small.contains('┃'), "{small}");
    app.run(crate::app::Action::ToggleCompact);
    assert!(render_at(&mut app, w, 14).contains('┃'), "framed mode still has it");
    app.run(crate::app::Action::ToggleCompact);

    key(&mut app, KeyCode::F(10));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert!(row(&screen, 0).contains("File") && row(&screen, 0).contains("Playback"));
    key(&mut app, KeyCode::Right);
    let screen = render_at(&mut app, w, h);
    assert!(row(&screen, 0).contains("Settings"), "moving between menus keeps the bar out");
    key(&mut app, KeyCode::Esc);
    let screen = render_at(&mut app, w, h);
    assert!(!row(&screen, 0).contains("File"));

    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    let d = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Folders));
    let y = d.y + 4;
    app.on_mouse(ev(MouseEventKind::Moved, d.x, y));
    let buf = draw_at(&mut app, w, h);
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert_eq!(buf[(d.x, y)].symbol(), "│", "hover line");
    let left_w = |app: &App| app.hits.iter().find(|h| matches!(h.target, Target::Column(_))).unwrap().rect.width;
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), d.x, y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), d.x - 10, y));
    let buf = draw_at(&mut app, w, h);
    assert_eq!(buf[(d.x - 10, y)].symbol(), "│", "line follows the drag");
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), d.x - 10, y));
    render_at(&mut app, w, h);
    assert_eq!(left_w(&app), d.x + 1 - 10);
    app.on_mouse(ev(MouseEventKind::Moved, 1, 1));

    let v = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual));
    app.on_mouse(ev(MouseEventKind::Moved, 60, v.y));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    let edge = row(&screen, v.y as usize);
    assert!(edge.trim().chars().all(|c| c == '─'), "the whole empty row: {edge}");
    assert!(!row(&screen, v.y as usize + 1).contains('─'), "titles below it untouched");

    app.run(crate::app::Action::ToggleCompact);
    let screen = render_at(&mut app, w, h);
    assert!(row(&screen, 0).contains("File"));
    assert!(screen.contains('╭'));
}

fn feed_stereo(tap: &crate::spectrum::Tap, ms: u64) {
    use std::f32::consts::TAU;
    let rate = 48_000u64;
    let mut w = tap.writer();
    for i in 0..ms * rate / 1000 {
        let t = i as f32 / rate as f32;
        let (l, r) = (0.6 * (TAU * 220.0 * t).sin(), 0.4 * (TAU * 330.0 * t).sin());
        w.push_stereo((l + r) * 0.5, l, r);
    }
}

fn scope_slot(app: &mut App) -> Vec<(String, ratatui::style::Color, ratatui::style::Color)> {
    let buf = draw_at(app, 90, 30);
    let slot = app.analyzer_rect;
    let mut v = Vec::new();
    for y in slot.y..slot.y + slot.height {
        for x in slot.x..slot.x + slot.width {
            let c = &buf[(x, y)];
            v.push((c.symbol().to_string(), c.fg, c.bg));
        }
    }
    v
}

#[test]
fn scopes_follow_the_theme_and_show_the_music() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    for (action, visual, name) in [
        (Action::ToggleVisual(Visual::Oscilloscope), Visual::Oscilloscope, "oscilloscope"),
        (Action::ToggleVisual(Visual::Vectorscope), Visual::Vectorscope, "vectorscope"),
        (Action::ToggleVisual(Visual::Spectroscope), Visual::Spectroscope, "spectroscope"),
    ] {
        app.run(action);
        assert_eq!(app.settings.visual, visual);
        assert_eq!(visual.name(), name);
        assert_eq!(Visual::from_name(name), visual);
        assert_eq!(crate::settings::Settings::load().visual, visual);
        for theme in ["Dracula", "Nord", "GitHub Light Default"] {
            let Some(t) = theme::THEMES.iter().position(|x| x.name == theme) else { continue };
            app.theme = t;
            let colors = theme::THEMES[t].scope_colors();
            feed_stereo(&app.tap, 60);
            let cells = scope_slot(&mut app);
            assert!(cells.iter().all(|c| c.2 == colors.ground), "{name} on {theme}: background isn't the theme's");
            let traced = |fg| cells.iter().filter(|c| c.1 == fg && c.0.chars().any(|ch| ('\u{2801}'..='\u{28FF}').contains(&ch))).count();
            assert!(traced(colors.left) > 10, "{name} on {theme}: no left channel trace");
            if visual != Visual::Vectorscope {
                assert!(traced(colors.right) > 10, "{name} on {theme}: no right channel trace");
            }
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn every_theme_gives_the_scopes_readable_colors() {
    for t in theme::THEMES {
        let c = t.scope_colors();
        assert!(theme::contrast_of(c.left, c.ground) >= 2.9, "{}: left", t.name);
        assert!(theme::contrast_of(c.right, c.ground) >= 2.9, "{}: right", t.name);
        assert_ne!(c.left, c.right, "{}", t.name);
    }
}

#[test]
fn scopes_go_flat_when_the_music_stops() {
    use crate::scopes::{Mode, Scopes};
    use std::time::{Duration, Instant};
    let tap = crate::spectrum::Tap::new();
    let mut s = Scopes::new();
    s.use_theme(0, || theme::THEMES[0].scope_colors());
    let now = Instant::now();
    for mode in [Mode::Oscilloscope, Mode::Vectorscope, Mode::Spectroscope] {
        for (w, h) in [(90, 11), (1, 1), (3, 2), (200, 40)] {
            let area = ratatui::layout::Rect::new(0, 0, w, h);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            s.update(&tap, false, now);
            s.paint(&mut buf, area, mode);
        }
    }
    feed_stereo(&tap, 50);
    s.update(&tap, false, now);
    assert!(s.is_active(&tap));
    s.update(&tap, true, now + Duration::from_secs(1));
    s.update(&tap, false, now + Duration::from_millis(1100));
    s.update(&tap, false, now + Duration::from_millis(1400));
    assert!(s.channels_flat(), "still showing a trace after the music stopped");
    for i in 0..40 {
        s.update(&tap, false, now + Duration::from_millis(1400 + 200 * i));
    }
    assert!(!s.is_active(&tap), "still moving long after the music stopped");
}

#[test]
fn the_scopes_work_the_scope_tui_way() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let zoom = |app: &mut App| (app.visuals.scopes().zoom() * 100.0).round() as i32;
    app.run(Action::ToggleVisual(Visual::Vectorscope));
    feed_stereo(&app.tap, 60);
    let screen = render_at(&mut app, 90, 30);
    for gone in ["scope-tui", "corr ", "left -", "right", "1.00x", "kHz", "fps", "spf", "---", "|>", "||"] {
        assert!(!screen.contains(gone), "{gone}: {screen}");
    }

    let r = rect_of(&app, |t| *t == Target::Scope);
    click_at(&mut app, r.x + 10, r.y + 3, MouseButton::Left);
    assert!(app.scope_keys);
    render_at(&mut app, 90, 30);
    assert_eq!(zoom(&mut app), 100);
    key(&mut app, KeyCode::Up);
    assert_eq!(zoom(&mut app), 101);
    key(&mut app, KeyCode::Esc);
    assert_eq!(zoom(&mut app), 100);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.settings.visual, Visual::Oscilloscope);
    assert_eq!(crate::settings::Settings::load().visual, Visual::Oscilloscope);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.settings.visual, Visual::Vectorscope);
    key(&mut app, KeyCode::F(1));
    assert!(matches!(app.overlay, Some(Overlay::Help)));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    key(&mut app, KeyCode::Char('q'));
    assert!(!app.scope_keys);

    click_at(&mut app, r.x + 10, r.y + 3, MouseButton::Left);
    assert!(app.scope_keys);
    let col = rect_of(&app, |t| matches!(t, Target::Column(_)));
    click_at(&mut app, col.x + 2, col.y + 1, MouseButton::Left);
    assert!(!app.scope_keys);

    for _ in 0..30 {
        if app.key_focus == Some(Target::Scope) {
            break;
        }
        key(&mut app, KeyCode::Tab);
        render_at(&mut app, 90, 30);
    }
    assert_eq!(app.key_focus, Some(Target::Scope));
    key(&mut app, KeyCode::Enter);
    assert!(app.scope_keys);
    key(&mut app, KeyCode::Char('q'));

    app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: r.x + 10, row: r.y + 3, modifiers: KeyModifiers::NONE });
    assert!(!app.scope_keys);
    render_at(&mut app, 90, 30);
    assert_eq!(zoom(&mut app), 105);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_oscilloscope_gives_its_header_and_label_rows_to_the_folders() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    let ev = |kind, column, row| MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    for (flipped, compact) in [(false, false), (true, false), (false, true), (true, true)] {
        app.run(Action::SetFlipped(flipped));
        app.settings.compact = compact;
        app.settings.visual_rows = 0;
        (app.settings.visual, app.settings.analyzer) = (Visual::Spectrum, true);
        feed_stereo(&app.tap, 60);
        render_at(&mut app, w, h);
        let (slot, panes) = (app.analyzer_rect, app.panes_rect);
        app.run(Action::ToggleVisual(Visual::Oscilloscope));
        feed_stereo(&app.tap, 60);
        let screen = render_at(&mut app, w, h);
        let what = format!("flipped {flipped}, minimalist {compact}:\n{screen}");
        for gone in ["scope-tui", "amplitude", "time -", "dBFS", "1.00x"] {
            assert!(!screen.contains(gone), "{gone}: {what}");
        }
        assert_eq!(app.analyzer_rect.height, slot.height - 3, "{what}");
        assert_eq!(app.panes_rect.height, panes.height + 3, "{what}");
        if flipped {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y, slot.y + 3), "{what}");
        } else {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y - 3, slot.y), "{what}");
        }
        assert_eq!(rect_of(&app, |t| *t == Target::Scope), app.analyzer_rect);
        for row in screen.lines().skip(app.analyzer_rect.y as usize).take(app.analyzer_rect.height as usize) {
            assert!(row.chars().all(|c| c == ' ' || ('\u{2800}'..='\u{28FF}').contains(&c)), "{row:?}\n{what}");
        }
        let r = app.analyzer_rect;
        click_at(&mut app, r.x + 10, r.y + 2, MouseButton::Left);
        assert!(app.scope_keys);
        let screen = render_at(&mut app, w, h);
        assert!(!screen.contains("scope-tui") && !screen.contains("amplitude"), "{screen}");
        key(&mut app, KeyCode::Char('q'));
        assert!(!app.scope_keys);
    }

    app.run(Action::SetFlipped(false));
    app.settings.compact = false;
    app.settings.visual_rows = 0;
    render_at(&mut app, w, h);
    let auto = app.analyzer_rect.height;
    let v = rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual));
    assert_eq!(v.y, app.analyzer_rect.y + auto);
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 20, v.y));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 20, v.y + 2));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 20, v.y + 2));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto + 2);
    assert_eq!(rect_of(&app, |t| *t == Target::Divider(crate::app::Divider::Visual)).y, v.y + 2);
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, auto - 1);
    for _ in 0..40 {
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    }
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, 4);
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    render_at(&mut app, w, h);
    assert_eq!(app.analyzer_rect.height, 5);
    let panes = app.panes_rect.height;
    app.run(Action::ToggleVisual(Visual::Spectrum));
    render_at(&mut app, w, h);
    assert_eq!((app.analyzer_rect.height, app.panes_rect.height), (8, panes - 3));
    app.run(Action::ToggleVisual(Visual::Vectorscope));
    render_at(&mut app, w, h);
    assert_eq!((app.analyzer_rect.height, app.panes_rect.height), (9, panes - 4));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_spectroscope_gives_its_header_row_to_the_folders() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    for (flipped, compact) in [(false, false), (true, false), (false, true), (true, true)] {
        app.run(Action::SetFlipped(flipped));
        app.settings.compact = compact;
        app.settings.visual_rows = 0;
        (app.settings.visual, app.settings.analyzer) = (Visual::Spectrum, true);
        feed_stereo(&app.tap, 60);
        render_at(&mut app, w, h);
        let (slot, panes) = (app.analyzer_rect, app.panes_rect);
        app.run(Action::ToggleVisual(Visual::Spectroscope));
        feed_stereo(&app.tap, 60);
        let screen = render_at(&mut app, w, h);
        let what = format!("flipped {flipped}, minimalist {compact}:\n{screen}");
        for gone in ["scope-tui", "level", "frequency", "amplitude", "G#", "Hz", "1.00x"] {
            assert!(!screen.contains(gone), "{gone}: {what}");
        }
        assert_eq!(app.analyzer_rect.height, slot.height - 1, "{what}");
        assert_eq!(app.panes_rect.height, panes.height + 1, "{what}");
        if flipped {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y, slot.y + 1), "{what}");
        } else {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y - 1, slot.y), "{what}");
        }
        assert_eq!(rect_of(&app, |t| *t == Target::Scope), app.analyzer_rect);
        for row in screen.lines().skip(app.analyzer_rect.y as usize).take(app.analyzer_rect.height as usize) {
            assert!(row.chars().all(|c| c == ' ' || ('\u{2800}'..='\u{28FF}').contains(&c)), "{row:?}\n{what}");
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_vectorscope_is_bigger_and_draws_nothing_but_the_trace() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    for (flipped, compact) in [(false, false), (true, false), (false, true), (true, true)] {
        app.run(Action::SetFlipped(flipped));
        app.settings.compact = compact;
        app.settings.visual_rows = 0;
        (app.settings.visual, app.settings.analyzer) = (Visual::Spectrum, true);
        feed_stereo(&app.tap, 60);
        render_at(&mut app, w, h);
        let (slot, panes) = (app.analyzer_rect, app.panes_rect);
        app.run(Action::ToggleVisual(Visual::Vectorscope));
        feed_stereo(&app.tap, 60);
        let screen = render_at(&mut app, w, h);
        let what = format!("flipped {flipped}, minimalist {compact}:\n{screen}");
        for gone in ["scope-tui", "left", "right", "corr", "kHz", "1.00x"] {
            assert!(!screen.contains(gone), "{gone}: {what}");
        }
        assert_eq!(app.analyzer_rect.height, slot.height + 1, "{what}");
        assert_eq!(app.panes_rect.height, panes.height - 1, "{what}");
        if flipped {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y, slot.y - 1), "{what}");
        } else {
            assert_eq!((app.panes_rect.y, app.analyzer_rect.y), (panes.y + 1, slot.y), "{what}");
        }
        assert_eq!(rect_of(&app, |t| *t == Target::Scope), app.analyzer_rect);
        for row in screen.lines().skip(app.analyzer_rect.y as usize).take(app.analyzer_rect.height as usize) {
            assert!(row.chars().all(|c| c == ' ' || ('\u{2800}'..='\u{28FF}').contains(&c)), "{row:?}\n{what}");
        }
    }
    app.run(Action::SetFlipped(false));
    app.settings.compact = false;
    app.settings.visual_rows = 0;
    render_at(&mut app, w, 18);
    assert!(app.panes_rect.height >= 5, "{}", app.panes_rect.height);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unfocused_pane_frames_stay_visible_in_every_theme() {
    use crate::theme::{contrast_of, THEMES};
    for t in THEMES {
        let p = t.palette();
        let (frame, dim) = (contrast_of(p.frame, p.bg), contrast_of(p.dim, p.bg));
        assert!(frame > contrast_of(p.raised, p.bg), "{}: the unfocused frame is no clearer than the old one", t.name);
        assert!(frame <= dim + 0.01, "{}: the unfocused frame is louder than the dim text", t.name);
        assert!(frame >= 1.8 || dim - frame < 0.05, "{}: unfocused frame contrast only {frame:.2}", t.name);
    }
}

#[test]
fn the_cursor_color_reads_and_stands_apart_from_the_playing_song_in_every_theme() {
    use crate::theme::{contrast_of, THEMES};
    for t in THEMES {
        let p = t.palette();
        assert!(contrast_of(p.cursor, p.bg) >= 2.9, "{}: the highlighted song can't be read", t.name);
        assert_ne!(p.cursor, p.playing, "{}: highlighted looks like playing", t.name);
    }
}

#[test]
fn q_quits_shift_q_queues_and_shift_n_plays_next() {
    let _g = serial();
    let root = library();
    let (album, _) = album_songs(&root);
    let mut app = App::new(Some(root.join("Music")));
    app.navigate_to(&album);
    render(&mut app);
    let col = app.focus;
    let ten = row_named(&app, col, "10 - Seeing Red.wav");
    let two = row_named(&app, col, "02 - I Don't Wanna Hear It.wav");
    app.select(col, ten);
    key(&mut app, KeyCode::Char('Q'));
    app.select(col, two);
    key(&mut app, KeyCode::Char('N'));
    assert!(!app.quit, "Shift+N and Shift+Q don't quit");
    let q = queue_order(&app);
    assert!(q.contains(&"10".to_string()) && q.contains(&"02".to_string()), "{q:?}");
    key(&mut app, KeyCode::Char('q'));
    assert!(app.quit, "q quits");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn clicking_a_menu_title_keeps_it_open() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    let click = |app: &mut App, x: u16, y: u16| app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), x, y));
    let open = |app: &App| app.menu.as_ref().and_then(|m| m.title_index);
    let title = |app: &App, i: usize| rect_of(app, |t| *t == Target::Menu(i));

    for compact in [true, false] {
        if app.settings.compact != compact {
            app.run(crate::app::Action::ToggleCompact);
        }
        render_at(&mut app, w, h);
        key(&mut app, KeyCode::F(10));
        render_at(&mut app, w, h);
        assert_eq!(open(&app), Some(0));

        for i in [0, 1, 2, 3, 2, 1] {
            let r = title(&app, i);
            app.on_mouse(ev(MouseEventKind::Moved, r.x + 1, 0));
            render_at(&mut app, w, h);
            assert_eq!(open(&app), Some(i), "hover, compact={compact}");
            click(&mut app, r.x + 1, 0);
            let screen = render_at(&mut app, w, h);
            assert_eq!(open(&app), Some(i), "click {i}, compact={compact}");
            assert!(screen.lines().next().unwrap().contains("File"), "the bar stays out");
        }

        let r = title(&app, 1);
        click(&mut app, r.x + 1, 0);
        let screen = render_at(&mut app, w, h);
        assert_eq!(open(&app), None, "second click, compact={compact}");
        assert_eq!(screen.lines().next().unwrap().contains("File"), !compact, "compact={compact}");

        key(&mut app, KeyCode::F(10));
        render_at(&mut app, w, h);
        click(&mut app, 40, 20);
        let screen = render_at(&mut app, w, h);
        assert_eq!(open(&app), None);
        assert_eq!(screen.lines().next().unwrap().contains("File"), !compact);
    }
}

#[test]
fn hovering_the_top_row_brings_out_the_menu_bar_in_minimalist_view() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    let (w, h) = (90, 30);
    let row0 = |s: &str| s.lines().next().unwrap().to_string();
    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    app.run(crate::app::Action::ToggleCompact);

    let screen = render_at(&mut app, w, h);
    assert!(!row0(&screen).contains("File"));
    app.on_mouse(ev(MouseEventKind::Moved, 45, 0));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert!(row0(&screen).contains("File") && row0(&screen).contains("Help"), "{}", row0(&screen));
    let file = rect_of(&app, |t| *t == Target::Menu(0));
    app.on_mouse(ev(MouseEventKind::Moved, file.x + 1, 0));
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), file.x + 1, 0));
    render_at(&mut app, w, h);
    assert!(app.menu.as_ref().is_some_and(|m| m.title_index == Some(0)), "File menu open");
    app.on_mouse(ev(MouseEventKind::Moved, file.x + 2, 2));
    assert!(row0(&render_at(&mut app, w, h)).contains("File"));
    key(&mut app, KeyCode::Esc);
    let screen = render_at(&mut app, w, h);
    assert!(!row0(&screen).contains("File"), "menu closed and pointer off the top row: bar gone");
    app.on_mouse(ev(MouseEventKind::Moved, 45, 0));
    app.on_mouse(ev(MouseEventKind::Moved, 45, 1));
    assert!(!row0(&render_at(&mut app, w, h)).contains("File"));

    let click = |app: &mut App, x: u16, y: u16| {
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), x, y));
        app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), x, y));
    };
    let bar_out = |app: &mut App| row0(&render_at(app, w, h)).contains("File");
    for title in 0..4 {
        app.on_mouse(ev(MouseEventKind::Moved, 45, 0));
        assert!(bar_out(&mut app));
        click(&mut app, file.x + 1, 0);
        render_at(&mut app, w, h);
        assert_eq!(app.menu.as_ref().and_then(|m| m.title_index), Some(0));
        let t = rect_of(&app, |t| *t == Target::Menu(title));
        app.on_mouse(ev(MouseEventKind::Moved, t.x + 1, 0));
        render_at(&mut app, w, h);
        click(&mut app, t.x + 1, 0);
        if title != 0 {
            render_at(&mut app, w, h);
            assert_eq!(app.menu.as_ref().and_then(|m| m.title_index), Some(title), "first click shows it");
            click(&mut app, t.x + 1, 0);
        }
        assert!(app.menu.is_none(), "title {title}: menu closed");
        assert!(!bar_out(&mut app), "title {title}: bar put away");
        app.on_mouse(ev(MouseEventKind::Moved, 60, 0));
        assert!(!bar_out(&mut app), "title {title}: stays away while on the row");
        app.on_mouse(ev(MouseEventKind::Moved, t.x + 1, 0));
        click(&mut app, t.x + 1, 0);
        assert!(bar_out(&mut app), "title {title}: click brings the bar back");
        assert_eq!(app.menu.as_ref().and_then(|m| m.title_index), Some(title), "title {title}: click reopens it");
        click(&mut app, t.x + 1, 0);
        assert!(app.menu.is_none() && !bar_out(&mut app), "title {title}: closes again");
        click(&mut app, 60, 0);
        assert!(bar_out(&mut app) && app.menu.is_none(), "title {title}: gap click brings the bar back");
        app.on_mouse(ev(MouseEventKind::Moved, 60, 0));
        assert!(bar_out(&mut app), "title {title}: and it stays out while on the row");
        app.on_mouse(ev(MouseEventKind::Moved, 60, 3));
        assert!(!bar_out(&mut app));
    }
    app.on_mouse(ev(MouseEventKind::Moved, 45, 0));
    assert!(bar_out(&mut app));
    click(&mut app, file.x + 1, 0);
    assert!(app.menu.is_some());
    key(&mut app, KeyCode::Esc);
    assert!(!bar_out(&mut app), "Esc");
    app.on_mouse(ev(MouseEventKind::Moved, 60, 3));
    app.on_mouse(ev(MouseEventKind::Moved, 60, 0));
    assert!(bar_out(&mut app));
    click(&mut app, 60, 0);
    assert!(!bar_out(&mut app), "a gap in the bar");
    app.on_mouse(ev(MouseEventKind::Moved, 60, 3));

    app.run(crate::app::Action::SetFlipped(true));
    render_at(&mut app, w, h);
    let back = rect_of(&app, |t| *t == Target::Back);
    assert_eq!(back.y, 0, "toolbar on the top row");
    app.on_mouse(ev(MouseEventKind::Moved, back.x + 1, 0));
    let screen = render_at(&mut app, w, h);
    assert!(!row0(&screen).contains("File"), "pointing at a button doesn't cover the toolbar");
    let empty = (0..w).find(|&x| app.hit_test(x, 0).is_none()).expect("the toolbar has an empty stretch");
    app.on_mouse(ev(MouseEventKind::Moved, empty, 0));
    let screen = render_at(&mut app, w, h);
    println!("{screen}");
    assert!(row0(&screen).contains("File"), "{}", row0(&screen));
    app.on_mouse(ev(MouseEventKind::Moved, 0, 0));
    assert!(row0(&render_at(&mut app, w, h)).contains("File"));
    let gap = (0..w).rev().find(|&x| app.hit_test(x, 0) == Some(Target::MenuBar)).unwrap();
    let dir = app.current_dir();
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), gap, 0));
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), gap, 0));
    assert_eq!(app.current_dir(), dir);
    assert!(app.menu.is_none());
    app.on_mouse(ev(MouseEventKind::Moved, 45, 5));
    let screen = render_at(&mut app, w, h);
    assert!(!row0(&screen).contains("File"), "toolbar back");
    assert!(app.hits.iter().any(|h| h.target == Target::Back));

    app.run(crate::app::Action::ToggleCompact);
    app.run(crate::app::Action::SetFlipped(false));
    app.on_mouse(ev(MouseEventKind::Moved, 45, 10));
    assert!(row0(&render_at(&mut app, w, h)).contains("File"));
    assert!(!app.menu_peek);
    let file = rect_of(&app, |t| *t == Target::Menu(0));
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), file.x + 1, 0));
    assert!(app.menu.is_some());
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), file.x + 1, 0));
    assert!(app.menu.is_none());
    assert!(row0(&render_at(&mut app, w, h)).contains("File"));
}

#[test]
#[ignore]
fn frame_cost() {
    let _g = serial();
    let root = library();
    let only = std::env::var("FRAME_COST_ONLY").ok();
    for (w, h) in [(90, 30), (200, 60)] {
        for visual in [Visual::Spectrum, Visual::Trace, Visual::Bokeh, Visual::Lines, Visual::Synthwave, Visual::SynthMoonWave, Visual::Galaxy, Visual::ClubLights, Visual::Oscilloscope, Visual::Spectroscope] {
            if only.as_ref().is_some_and(|o| o != visual.name()) {
                continue;
            }
            let mut app = App::new(Some(root.join("Music")));
            app.settings.visual = visual;
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let tap = app.tap.clone();
            let mut t = 0.0f32;
            let frames = 120;
            let mut total = std::time::Duration::ZERO;
            for _ in 0..frames {
                let mut wr = tap.writer();
                for _ in 0..1600 {
                    wr.push((synth_music(t) * 2.0).clamp(-1.0, 1.0));
                    t += 1.0 / 48_000.0;
                }
                drop(wr);
                let t0 = std::time::Instant::now();
                term.draw(|f| ui::draw(f, &mut app)).unwrap();
                total += t0.elapsed();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            eprintln!("{w}x{h} {visual:?}: {:.3} ms a frame", total.as_secs_f64() * 1000.0 / frames as f64);
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn minimalist_view_hides_the_toolbar_once_you_settle_into_a_song() {
    let _g = serial();
    let root = library();
    let music = root.join("Music");
    let album = music.join("punk/hardcore/Minor Threat/Complete Discography");
    let song = album.join("02 - I Don't Wanna Hear It.wav");
    let (w, h) = (90, 30);
    let ev = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    let shown = |app: &mut App| {
        let screen = render_at(app, w, h);
        let hit = app.hits.iter().any(|h| h.target == Target::Back);
        assert_eq!(screen.contains("Pin Folder"), hit, "{screen}");
        hit
    };
    let settle = |app: &mut App| {
        app.navigate_to(&album);
        render_at(app, w, h);
        let row = app.levels[app.focus].entries.iter().position(|e| e.path == song).unwrap();
        let col = app.focus;
        click(app, |t| *t == Target::Row { col, row });
        click(app, |t| *t == Target::Row { col, row });
        app.status.plays = app.engine.plays_sent();
        app.status.state = crate::audio::State::Playing;
        app.status.queue = std::sync::Arc::new(vec![album.join("01 - Filler.wav"), song.clone()]);
        app.status.index = 1;
        app.status.position = 4.0;
        app.check_solo();
    };

    for flipped in [false, true] {
        let mut app = App::new(Some(music.clone()));
        app.settings.flipped = flipped;
        app.settings.compact = true;
        assert!(!shown(&mut app), "flipped {flipped}: no toolbar at launch");
        key(&mut app, KeyCode::Down);
        assert!(shown(&mut app), "flipped {flipped}: a key brings it out");
        settle(&mut app);
        assert!(shown(&mut app), "flipped {flipped}: still there before 5 s");
        app.status.position = 13.5;
        app.check_solo();
        assert!(app.solo);
        assert!(!shown(&mut app), "flipped {flipped}: gone with the other folders");
        for code in [KeyCode::Down, KeyCode::Up, KeyCode::Char(' '), KeyCode::Esc] {
            key(&mut app, code);
            app.status.state = crate::audio::State::Playing;
            assert!(!shown(&mut app), "flipped {flipped}: {code:?}");
        }
        let y = app.toolbar_row;
        app.on_mouse(ev(MouseEventKind::Moved, 60, y));
        assert!(shown(&mut app), "flipped {flipped}: pointer on the row");
        let screen = render_at(&mut app, w, h);
        assert!(!screen.lines().next().unwrap().contains("File"), "the toolbar comes first, not the menu bar\n{screen}");
        assert!(app.solo, "looking at the toolbar isn't navigating");
        app.on_mouse(ev(MouseEventKind::Moved, 60, 12));
        assert!(!shown(&mut app), "flipped {flipped}: pointer off the row");
        app.on_mouse(ev(MouseEventKind::Moved, 45, 0));
        if flipped {
            render_at(&mut app, w, h);
            app.on_mouse(ev(MouseEventKind::Moved, 46, 0));
        }
        let screen = render_at(&mut app, w, h);
        assert!(screen.lines().next().unwrap().contains("File"), "flipped {flipped}: menu bar on hover\n{screen}");
        app.on_mouse(ev(MouseEventKind::Moved, 60, 12));
        assert!(!shown(&mut app), "flipped {flipped}: pointer off the top row");
        assert!(!render_at(&mut app, w, h).lines().next().unwrap().contains("File"));
        key(&mut app, KeyCode::Left);
        assert!(!app.solo);
        assert!(shown(&mut app), "flipped {flipped}: back after navigating");

        app.solo = true;
        assert!(!shown(&mut app));
        app.on_mouse(ev(MouseEventKind::Moved, 1, y));
        assert!(shown(&mut app));
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 1, y));
        app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 1, y));
        app.on_mouse(ev(MouseEventKind::Moved, 60, 12));
        assert!(!app.solo);
        assert!(shown(&mut app), "flipped {flipped}: back after the back button");
    }

    let mut app = App::new(Some(music.clone()));
    app.settings.compact = false;
    settle(&mut app);
    app.status.position = 13.5;
    app.check_solo();
    assert!(app.solo);
    assert!(shown(&mut app));
}

#[test]
fn about_shows_the_version_and_a_clickable_link() {
    let _g = serial();
    let root = library();
    let mut app = App::new(Some(root.join("Music")));
    render(&mut app);
    app.run(Action::About);
    let screen = render(&mut app);
    assert!(screen.contains(&format!("Urchin Player {}", env!("CARGO_PKG_VERSION"))), "{screen}");
    assert!(screen.contains("Copyright github.com/xgabrielmartin"), "{screen}");
    assert!(!screen.contains("musikcube"), "{screen}");
    let link = rect_of(&app, |t| *t == Target::AboutLink);
    assert_eq!(link.width as usize, "github.com/xgabrielmartin".len());
    let row: String = screen.lines().nth(link.y as usize).unwrap().chars().skip(link.x as usize).take(link.width as usize).collect();
    assert_eq!(row, "github.com/xgabrielmartin");
    key(&mut app, KeyCode::Esc);
    render(&mut app);
    assert!(app.overlay.is_none());
    let _ = fs::remove_dir_all(&root);
}
