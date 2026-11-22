mod albums;
mod app;
mod audio;
mod bokeh;
mod lines;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod mediakeys;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod mediakeys {
    pub struct Media;
    impl Media {
        pub fn start() -> Option<Media> {
            None
        }
    }
}
mod synthwave;
mod club_lights;
mod decode;
mod eq;
mod fingerprint;
mod fsx;
mod galaxy;
mod library;
mod m3u;
mod night_sky;
mod marks;
mod order;
mod playlists;
mod queue;
mod reveal;
mod ring;
mod search;
mod settings;
mod slider;
mod scope_tui;
mod scopes;
mod spectrum;
mod theme;
mod trace;
mod ui;
mod viz;
mod visuals;
mod wav_info;

use std::io::{self, stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

const FRAME_BUFFER: usize = 64 * 1024;

const IDLE_FRAME: &[u8] = b"\x1b[39m\x1b[49m\x1b[59m\x1b[0m\x1b[?25l";

struct FrameOut {
    frame: Vec<u8>,
    out: io::Stdout,
}

impl Write for FrameOut {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.frame.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.frame.is_empty() && self.frame != IDLE_FRAME {
            let mut out = self.out.lock();
            out.write_all(&self.frame)?;
            out.flush()?;
        }
        self.frame.clear();
        Ok(())
    }
}

fn restore_terminal() {
    print!("\x1b[23;0t\x1b]22;default\x1b\\");
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), crossterm::event::DisableFocusChange, DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
}

fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("usage: urchin [FOLDER]\n\nOpens Urchin Player at FOLDER (default: your music folder).");
        return Ok(());
    }
    if matches!(arg.as_deref(), Some("-V" | "--version")) {
        println!("Urchin Player {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let start = arg.map(|a| {
        let p = PathBuf::from(fsx::expand_tilde(&a));
        std::fs::canonicalize(&p).unwrap_or(p)
    });

    let main_id = std::thread::current().id();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        if thread.id() == main_id {
            restore_terminal();
            default_hook(info);
        } else {
            trace::tlog!("panic on thread {}: {info}", thread.name().unwrap_or("?"));
        }
    }));

    trace::init();
    let mut app = app::App::new(start);
    app.enable_media_controls();
    app.restore_queue();
    app.restore_order();
    app.use_tag_cache();
    app.follow_moved_files();

    enable_raw_mode()?;
    print!("\x1b[22;0t");
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, crossterm::event::EnableFocusChange, crossterm::terminal::SetTitle("Urchin"))?;
    let out = FrameOut { frame: Vec::with_capacity(FRAME_BUFFER), out: stdout() };
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;
    terminal.hide_cursor()?;

    let result = run(&mut terminal, &mut app);
    trace::tlog!("quit: main loop ended ({})", if result.is_ok() { "normal" } else { "terminal error" });
    let _ = terminal.backend_mut().flush();

    app.engine.stop_now();
    app.save_on_exit();
    restore_terminal();
    result
}

fn watch_hangup(muter: audio::Muter) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        if let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGHUP, SIGTERM, SIGINT]) {
            let flag = flag.clone();
            let _ = std::thread::Builder::new().name("urchin-signals".into()).spawn(move || {
                if signals.forever().next().is_some() {
                    muter.mute();
                    flag.store(true, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(300));
                    muter.save_eq();
                    std::process::exit(0);
                }
            });
        }
    }
    #[cfg(not(unix))]
    let _ = muter;
    flag
}

fn terminal_gone() -> bool {
    #[cfg(unix)]
    unsafe {
        return libc::getppid() == 1 || libc::isatty(0) == 0;
    }
    #[allow(unreachable_code)]
    false
}

fn input_thread() -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("urchin-input".into())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(ev).is_err() {
                    break;
                }
            }
        })
        .expect("input thread");
    rx
}

fn run(terminal: &mut Terminal<CrosstermBackend<FrameOut>>, app: &mut app::App) -> anyhow::Result<()> {
    let hangup = watch_hangup(app.engine.muter());
    let input = input_thread();
    let mut pointer = "default";
    loop {
        if hangup.load(Ordering::Relaxed) || terminal_gone() {
            trace::tlog!("quit: {}", if hangup.load(Ordering::Relaxed) { "signal (tab closed, or asked to stop)" } else { "terminal gone" });
            app.engine.stop_now();
            return Ok(());
        }
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        if !app.osc_out.is_empty() {
            let out = terminal.backend_mut();
            for s in app.osc_out.drain(..) {
                let _ = write!(out, "{s}");
            }
            let _ = out.flush();
        }
        let shape = app.pointer_shape();
        if shape != pointer {
            pointer = shape;
            let out = terminal.backend_mut();
            let _ = write!(out, "\x1b]22;{shape}\x1b\\");
            let _ = out.flush();
        }
        match input.recv_timeout(app.frame_interval()) {
            Ok(ev) => {
                let mut next = Some(ev);
                let mut n = 0;
                while let Some(ev) = next.take() {
                    match ev {
                        Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                        Event::Mouse(m) => app.on_mouse(m),
                        Event::FocusGained => app.on_focus(),
                        Event::FocusLost => app.on_focus_lost(),
                        _ => {}
                    }
                    n += 1;
                    if app.quit || n >= 256 {
                        break;
                    }
                    next = input.try_recv().ok();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                app.engine.stop_now();
                return Ok(());
            }
        }
        if app.quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
