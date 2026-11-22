use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub use souvlaki::{MediaControlEvent, SeekDirection};
use souvlaki::{MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::audio::{State, Status};

pub struct Media {
    controls: MediaControls,
    rx: Receiver<MediaControlEvent>,
    shown: Option<Shown>,
}

#[derive(PartialEq)]
struct Shown {
    playing: Option<bool>,
    index: usize,
    title: String,
    duration_s: u64,
}

impl Media {
    pub fn start() -> Option<Media> {
        if !service_available() {
            return None;
        }
        let config = PlatformConfig {
            dbus_name: "urchin",
            display_name: "Urchin Player",
            hwnd: None,
        };
        let mut controls = MediaControls::new(config).ok()?;
        let (tx, rx) = channel();
        controls
            .attach(move |e: MediaControlEvent| {
                let _ = tx.send(e);
            })
            .ok()?;
        Some(Media { controls, rx, shown: None })
    }

    pub fn events(&mut self) -> Vec<MediaControlEvent> {
        pump_os_events();
        self.rx.try_iter().collect()
    }

    pub fn update(&mut self, st: &Status) {
        let playing = match st.state {
            State::Playing => Some(true),
            State::Paused => Some(false),
            State::Stopped if !st.queue.is_empty() => Some(false),
            State::Stopped => None,
        };
        let now = Shown {
            playing,
            index: st.index,
            title: st.meta.title.clone(),
            duration_s: st.duration as u64,
        };
        if self.shown.as_ref() == Some(&now) {
            return;
        }
        let progress = Some(MediaPosition(Duration::from_secs_f64(st.position.max(0.0))));
        let _ = match playing {
            Some(true) => self.controls.set_playback(MediaPlayback::Playing { progress }),
            Some(false) => self.controls.set_playback(MediaPlayback::Paused { progress }),
            None => self.controls.set_playback(MediaPlayback::Stopped),
        };
        if playing.is_some() {
            fn opt(s: &str) -> Option<&str> {
                if s.is_empty() { None } else { Some(s) }
            }
            let _ = self.controls.set_metadata(MediaMetadata {
                title: opt(&st.meta.title),
                artist: opt(&st.meta.artist),
                album: opt(&st.meta.album),
                cover_url: None,
                duration: if st.duration > 0.0 { Some(Duration::from_secs_f64(st.duration)) } else { None },
            });
        }
        self.shown = Some(now);
    }
}

#[cfg(target_os = "macos")]
fn pump_os_events() {
    use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
    CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::ZERO, true);
}

#[cfg(not(target_os = "macos"))]
fn pump_os_events() {}

#[cfg(target_os = "linux")]
fn service_available() -> bool {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    let path = match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        Ok(address) => match address.split(';').next().and_then(|a| a.strip_prefix("unix:")) {
            Some(params) => match params.split(',').find_map(|kv| kv.strip_prefix("path=")) {
                Some(path) => PathBuf::from(path),
                None => return true,
            },
            None => return true,
        },
        Err(_) => match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir).join("bus"),
            None => return false,
        },
    };
    UnixStream::connect(path).is_ok()
}

#[cfg(not(target_os = "linux"))]
fn service_available() -> bool {
    true
}
