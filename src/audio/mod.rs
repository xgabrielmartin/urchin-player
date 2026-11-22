mod follower;
use follower::Follower;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler, WindowFunction};

use crate::eq::{Curve, Filter, Processor, Store};
use crate::ring::Ring;
use crate::trace::tlog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReplayGain {
    #[default]
    Off,
    Track,
    Album,
}

impl ReplayGain {
    fn code(self) -> u8 {
        match self {
            ReplayGain::Off => 0,
            ReplayGain::Track => 1,
            ReplayGain::Album => 2,
        }
    }

    fn from_code(c: u8) -> ReplayGain {
        match c {
            1 => ReplayGain::Track,
            2 => ReplayGain::Album,
            _ => ReplayGain::Off,
        }
    }

    pub fn factor(self, g: &crate::decode::Gain) -> f32 {
        let (db, peak) = match self {
            ReplayGain::Off => return 1.0,
            ReplayGain::Track => (g.track.or(g.album), g.track_peak.or(g.album_peak)),
            ReplayGain::Album => (g.album.or(g.track), g.album_peak.or(g.track_peak)),
        };
        let Some(db) = db else { return 1.0 };
        let mut f = 10f32.powf(db / 20.0);
        if let Some(peak) = peak {
            f = f.min(1.0 / peak);
        }
        f
    }
}

pub const MAX_CROSSFADE: u32 = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub queue: Arc<Vec<PathBuf>>,
    pub index: usize,
    pub position: f64,
    pub duration: f64,
    pub meta: TrackMeta,
    pub error: Option<String>,
    pub device: String,
    pub output_lost: bool,
    pub acked: u64,
    pub plays: u64,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            state: State::Stopped,
            queue: Arc::new(Vec::new()),
            index: 0,
            position: 0.0,
            duration: 0.0,
            meta: TrackMeta::default(),
            error: None,
            device: String::new(),
            output_lost: false,
            acked: 0,
            plays: 0,
        }
    }
}

impl Status {
    pub fn playing_index(&self) -> Option<usize> {
        (self.state != State::Stopped).then_some(self.index)
    }

    pub fn current(&self) -> Option<&PathBuf> {
        if self.state == State::Stopped {
            None
        } else {
            self.queue.get(self.index)
        }
    }
}

enum Cmd {
    Play(Vec<PathBuf>, usize),
    TogglePause,
    MediaToggle,
    Stop,
    Next,
    Previous,
    Seek(f64),
    SeekBy(f64),
    Restart,
    Repeat(Repeat),
    Enqueue { paths: Vec<PathBuf>, next: bool },
    Jump(usize, PathBuf),
    Remove(usize, PathBuf),
    Move { from: usize, to: usize, song: PathBuf },
    Clear,
    ShuffleUpcoming(u64),
    Reorder { upcoming: Vec<PathBuf>, order: Vec<usize> },
    Load(Vec<PathBuf>, Option<(usize, f64)>),
}

impl Cmd {
    fn describe(&self) -> String {
        match self {
            Cmd::Play(q, i) => format!("Play(song {} of {})", i + 1, q.len()),
            Cmd::TogglePause => "TogglePause".into(),
            Cmd::MediaToggle => "MediaToggle".into(),
            Cmd::Stop => "Stop".into(),
            Cmd::Next => "Next".into(),
            Cmd::Previous => "Previous".into(),
            Cmd::Seek(s) => format!("Seek({s:.1} s)"),
            Cmd::SeekBy(s) => format!("SeekBy({s:+.1} s)"),
            Cmd::Restart => "Restart".into(),
            Cmd::Repeat(r) => format!("Repeat({r:?})"),
            Cmd::Enqueue { paths, next } => format!("Enqueue({} songs, next: {next})", paths.len()),
            Cmd::Jump(i, _) => format!("Jump(song {})", i + 1),
            Cmd::Remove(i, _) => format!("Remove(song {})", i + 1),
            Cmd::Move { from, to, .. } => format!("Move({} -> {})", from + 1, to + 1),
            Cmd::Clear => "Clear".into(),
            Cmd::ShuffleUpcoming(_) => "ShuffleUpcoming".into(),
            Cmd::Reorder { order, .. } => format!("Reorder({} songs)", order.len()),
            Cmd::Load(q, at) => format!("Load({} songs, at {at:?})", q.len()),
        }
    }

    fn cuts(&self) -> bool {
        matches!(
            self,
            Cmd::Play(..) | Cmd::Next | Cmd::Previous | Cmd::Seek(_) | Cmd::SeekBy(_) | Cmd::Restart | Cmd::Jump(..) | Cmd::Remove(..) | Cmd::Clear
        )
    }
}

fn song_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

struct EqSlot {
    start: u64,
    song: PathBuf,
    curve: Curve,
    filter: Filter,
}

struct EqShared {
    slots: VecDeque<EqSlot>,
    version: u64,
    enabled: bool,
}

struct Shared {
    eq: Mutex<EqShared>,
    store: Arc<Mutex<Store>>,
    paused: AtomicBool,
    volume: AtomicU32,
    consumed: AtomicU64,
    rate: AtomicU32,
    sent: AtomicU64,
    handled: AtomicU64,
    plays_sent: AtomicU64,
    status: Mutex<Status>,
    tap: Arc<crate::spectrum::Tap>,
    crossfade_ms: AtomicU32,
    replaygain: AtomicU8,
    starved: AtomicU64,
    gate_busy: AtomicU64,
    #[cfg(test)]
    lose_now: AtomicBool,
    #[cfg(test)]
    idle_release_ms: AtomicU64,
    #[cfg(test)]
    opens: AtomicU64,
}

impl Shared {
    fn new(volume: f32, store: Arc<Mutex<Store>>, eq_enabled: bool) -> Shared {
        Shared {
            eq: Mutex::new(EqShared { slots: VecDeque::new(), version: 0, enabled: eq_enabled }),
            store,
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            consumed: AtomicU64::new(0),
            rate: AtomicU32::new(48_000),
            sent: AtomicU64::new(0),
            handled: AtomicU64::new(0),
            plays_sent: AtomicU64::new(0),
            status: Mutex::new(Status::default()),
            tap: Arc::new(crate::spectrum::Tap::new()),
            crossfade_ms: AtomicU32::new(0),
            replaygain: AtomicU8::new(0),
            starved: AtomicU64::new(0),
            gate_busy: AtomicU64::new(0),
            #[cfg(test)]
            lose_now: AtomicBool::new(false),
            #[cfg(test)]
            idle_release_ms: AtomicU64::new(IDLE_RELEASE.as_millis() as u64),
            #[cfg(test)]
            opens: AtomicU64::new(0),
        }
    }

    fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    fn replaygain(&self) -> ReplayGain {
        ReplayGain::from_code(self.replaygain.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    fn take_forced_loss(&self) -> bool {
        self.lose_now.swap(false, Ordering::Relaxed)
    }

    #[cfg(not(test))]
    fn take_forced_loss(&self) -> bool {
        false
    }

    #[cfg(test)]
    fn idle_release(&self) -> Duration {
        Duration::from_millis(self.idle_release_ms.load(Ordering::Relaxed))
    }

    #[cfg(not(test))]
    fn idle_release(&self) -> Duration {
        IDLE_RELEASE
    }
}

pub struct Engine {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

#[derive(Clone)]
pub struct Muter(Arc<Shared>);

impl Muter {
    pub fn mute(&self) {
        self.0.paused.store(true, Ordering::Relaxed);
    }

    pub fn save_eq(&self) {
        if let Ok(mut store) = self.0.store.try_lock() {
            store.save_if_changed();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_now();
    }
}

impl Engine {
    pub fn start(volume: f32, store: Arc<Mutex<Store>>, eq_enabled: bool) -> Engine {
        let shared = Arc::new(Shared::new(volume, store, eq_enabled));
        let (tx, rx) = mpsc::channel();
        let s = shared.clone();
        thread::Builder::new()
            .name("urchin-audio".into())
            .spawn(move || engine_thread(s, rx))
            .expect("audio thread");
        Engine { tx, shared }
    }

    fn send(&self, cmd: Cmd) {
        self.shared.sent.fetch_add(1, Ordering::AcqRel);
        if self.tx.send(cmd).is_err() {
            self.shared.sent.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub fn play(&self, queue: Vec<PathBuf>, index: usize) -> u64 {
        let serial = self.shared.plays_sent.fetch_add(1, Ordering::AcqRel) + 1;
        self.send(Cmd::Play(queue, index));
        serial
    }
    pub fn toggle_pause(&self) {
        self.send(Cmd::TogglePause);
    }
    pub fn media_toggle_pause(&self) {
        self.send(Cmd::MediaToggle);
    }
    pub fn enqueue(&self, paths: Vec<PathBuf>, next: bool) {
        self.send(Cmd::Enqueue { paths, next });
    }
    pub fn jump(&self, index: usize, song: PathBuf) {
        self.send(Cmd::Jump(index, song));
    }
    pub fn remove_from_queue(&self, index: usize, song: PathBuf) {
        self.send(Cmd::Remove(index, song));
    }
    pub fn move_in_queue(&self, from: usize, to: usize, song: PathBuf) {
        self.send(Cmd::Move { from, to, song });
    }
    pub fn clear_queue(&self) {
        self.send(Cmd::Clear);
    }
    pub fn shuffle_upcoming(&self, seed: u64) {
        self.send(Cmd::ShuffleUpcoming(seed));
    }
    pub fn reorder_upcoming(&self, upcoming: Vec<PathBuf>, order: Vec<usize>) {
        self.send(Cmd::Reorder { upcoming, order });
    }
    pub fn load_queue(&self, queue: Vec<PathBuf>) {
        self.send(Cmd::Load(queue, None));
    }
    pub fn load_queue_at(&self, queue: Vec<PathBuf>, index: usize, seconds: f64) {
        self.send(Cmd::Load(queue, Some((index, seconds.max(0.0)))));
    }
    pub fn tap(&self) -> Arc<crate::spectrum::Tap> {
        self.shared.tap.clone()
    }

    pub fn muter(&self) -> Muter {
        Muter(self.shared.clone())
    }

    pub fn stop_now(&self) {
        self.shared.paused.store(true, Ordering::Relaxed);
        self.send(Cmd::Stop);
    }
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }
    pub fn next(&self) {
        self.send(Cmd::Next);
    }
    pub fn previous(&self) {
        self.send(Cmd::Previous);
    }
    pub fn seek(&self, seconds: f64) {
        self.send(Cmd::Seek(seconds.max(0.0)));
    }
    pub fn seek_by(&self, seconds: f64) {
        self.send(Cmd::SeekBy(seconds));
    }
    pub fn restart(&self) {
        self.send(Cmd::Restart);
    }
    pub fn set_repeat(&self, repeat: Repeat) {
        self.send(Cmd::Repeat(repeat));
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn set_song_curve(&self, song: &Path, curve: Curve) {
        self.shared.store.lock().unwrap().set(song, curve);
        let filter = Filter::compile(&curve, self.shared.rate());
        let mut eq = self.shared.eq.lock().unwrap();
        for slot in eq.slots.iter_mut().filter(|s| s.song == song) {
            slot.curve = curve;
            slot.filter = filter;
        }
        eq.version += 1;
    }

    pub fn refresh_curves(&self) {
        let updated: Vec<(PathBuf, Curve)> = {
            let store = self.shared.store.lock().unwrap();
            let eq = self.shared.eq.lock().unwrap();
            eq.slots.iter().map(|s| (s.song.clone(), store.get(&s.song))).collect()
        };
        let rate = self.shared.rate();
        let mut eq = self.shared.eq.lock().unwrap();
        for (song, curve) in updated {
            let filter = Filter::compile(&curve, rate);
            for slot in eq.slots.iter_mut().filter(|s| s.song == song) {
                slot.curve = curve;
                slot.filter = filter;
            }
        }
        eq.version += 1;
    }

    pub fn set_folder_preset(&self, dir: &Path, preset: Option<u8>) {
        let updated: Vec<(PathBuf, Curve)> = {
            let mut store = self.shared.store.lock().unwrap();
            store.set_folder_preset(dir, preset);
            let eq = self.shared.eq.lock().unwrap();
            eq.slots
                .iter()
                .filter(|s| s.song.parent() == Some(dir))
                .map(|s| (s.song.clone(), store.get(&s.song)))
                .collect()
        };
        let rate = self.shared.rate();
        let updated: Vec<(PathBuf, Curve, Filter)> =
            updated.into_iter().map(|(song, curve)| (song, curve, Filter::compile(&curve, rate))).collect();
        let mut eq = self.shared.eq.lock().unwrap();
        for (song, curve, filter) in updated {
            for slot in eq.slots.iter_mut().filter(|s| s.song == song) {
                slot.curve = curve;
                slot.filter = filter;
            }
        }
        eq.version += 1;
    }

    pub fn set_crossfade(&self, seconds: u32) {
        self.shared.crossfade_ms.store(seconds.min(MAX_CROSSFADE) * 1000, Ordering::Relaxed);
    }

    pub fn set_replaygain(&self, mode: ReplayGain) {
        self.shared.replaygain.store(mode.code(), Ordering::Relaxed);
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        let mut eq = self.shared.eq.lock().unwrap();
        eq.enabled = enabled;
        eq.version += 1;
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn caught_up(&self, status: &Status) -> bool {
        status.acked == self.shared.sent.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub fn plays_sent(&self) -> u64 {
        self.shared.plays_sent.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub fn lose_output_for_test(&self) {
        self.shared.lose_now.store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn release_idle_output_after_for_test(&self, after: Duration) {
        self.shared.idle_release_ms.store(after.as_millis() as u64, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn outputs_opened_for_test(&self) -> u64 {
        self.shared.opens.load(Ordering::Relaxed)
    }
}

struct Source {
    backend: Box<dyn crate::decode::Backend>,
    duration: f64,
    meta: TrackMeta,
    resampler: Option<Fft<f32>>,
    pending: [Vec<f32>; 2],
    resampled: [Vec<f32>; 2],
    in_rate: u32,
    out_rate: u32,
    skip: usize,
    fed: u64,
    emitted: u64,
    finished: bool,
    ended: bool,
    own: u64,
    tail: Vec<f32>,
    carry: Vec<f32>,
    gain: crate::decode::Gain,
    level: f32,
}

const CHUNK: usize = 1024;

impl Source {
    fn open(path: &Path, out_rate: u32) -> Result<Source> {
        let o = crate::decode::open(path)?;
        let mut meta = o.meta;
        if meta.title.is_empty() {
            meta.title = crate::fsx::title_from_path(path);
        }
        let resampler = if o.rate == out_rate {
            None
        } else {
            Some(Fft::<f32>::new_custom(
                o.rate as usize,
                out_rate as usize,
                CHUNK,
                2,
                2,
                WindowFunction::BlackmanHarris2,
                FixedSync::Input,
            )?)
        };
        let skip = resampler.as_ref().map_or(0, |rs| rs.output_delay());
        Ok(Source {
            backend: o.backend,
            duration: o.duration,
            meta,
            resampler,
            pending: [Vec::new(), Vec::new()],
            resampled: [Vec::new(), Vec::new()],
            in_rate: o.rate,
            out_rate,
            skip,
            fed: 0,
            emitted: 0,
            finished: false,
            ended: false,
            own: 0,
            tail: Vec::new(),
            carry: Vec::new(),
            gain: o.gain,
            level: f32::NAN,
        })
    }

    fn apply_gain(&mut self, frames: &mut [f32], mode: ReplayGain) {
        let target = mode.factor(&self.gain);
        let from = if self.level.is_nan() { target } else { self.level };
        self.level = target;
        if from == 1.0 && target == 1.0 {
            return;
        }
        let n = (frames.len() / 2).max(1) as f32;
        for (i, f) in frames.chunks_exact_mut(2).enumerate() {
            let g = if from == target { target } else { from + (target - from) * (i as f32 + 1.0) / n };
            f[0] *= g;
            f[1] *= g;
        }
    }

    fn expected(&self) -> u64 {
        (self.fed as u128 * self.out_rate as u128 + self.in_rate as u128 / 2) as u64 / self.in_rate as u64
    }

    fn take(&mut self, l: &[f32], r: &[f32], finish: bool, out: &mut Vec<f32>) {
        let limit = if finish { self.expected() } else { u64::MAX };
        for i in 0..l.len() {
            if self.skip > 0 {
                self.skip -= 1;
            } else if self.emitted < limit {
                out.push(l[i]);
                out.push(r[i]);
                self.emitted += 1;
            } else {
                self.tail.push(l[i]);
                self.tail.push(r[i]);
            }
        }
    }

    fn mix_carry(&mut self, out: &mut [f32]) {
        let n = self.carry.len().min(out.len());
        for (o, c) in out.iter_mut().zip(&self.carry[..n]) {
            *o += c;
        }
        self.carry.drain(..n);
    }

    #[cfg(test)]
    fn next_frames(&mut self) -> Option<Vec<f32>> {
        self.next_block().or_else(|| self.flush())
    }

    fn next_block(&mut self) -> Option<Vec<f32>> {
        while !self.ended {
            let [l, r] = &mut self.pending;
            let before = l.len();
            let more = self.backend.read(l, r);
            let got = (self.pending[0].len() - before) as u64;
            self.fed += got;
            self.own += got;
            self.ended = !more;
            let mut out = self.drain(false);
            if !out.is_empty() {
                self.mix_carry(&mut out);
                return Some(out);
            }
        }
        None
    }

    fn can_hand_to(&self, next: &Source) -> bool {
        self.ended && !self.finished && self.resampler.is_some() && next.resampler.is_some() && self.in_rate == next.in_rate
    }

    fn adopt(&mut self, prev: &mut Source) -> u64 {
        let left = prev.expected().saturating_sub(prev.emitted);
        self.resampler = prev.resampler.take();
        let [l, r] = std::mem::take(&mut prev.pending);
        self.pending = [l, r];
        self.skip = prev.skip;
        self.fed = prev.fed;
        self.emitted = prev.emitted;
        self.carry = std::mem::take(&mut prev.carry);
        self.level = prev.level;
        prev.finished = true;
        left
    }

    fn drain(&mut self, finish: bool) -> Vec<f32> {
        let mut out = Vec::new();
        match self.resampler.as_mut() {
            None => {
                let n = self.pending[0].len();
                out.reserve(n * 2);
                for i in 0..n {
                    out.push(self.pending[0][i]);
                    out.push(self.pending[1][i]);
                }
                self.pending[0].clear();
                self.pending[1].clear();
                self.emitted += n as u64;
            }
            Some(_) => {
                loop {
                    let expected = self.expected();
                    let rs = self.resampler.as_mut().unwrap();
                    let need = rs.input_frames_next();
                    if self.pending[0].len() < need {
                        if !finish {
                            break;
                        }
                        let delay = rs.output_delay() as u64;
                        if self.pending[0].is_empty() && self.emitted >= expected && self.tail.len() as u64 >= 2 * delay {
                            break;
                        }
                        let pad = need - self.pending[0].len();
                        self.pending[0].extend(std::iter::repeat_n(0.0, pad));
                        self.pending[1].extend(std::iter::repeat_n(0.0, pad));
                    }
                    let made = rs.output_frames_next();
                    let mut res = std::mem::take(&mut self.resampled);
                    for plane in &mut res {
                        plane.resize(made, 0.0);
                    }
                    let input = SequentialSliceOfVecs::new(&self.pending[..], 2, need);
                    let output = SequentialSliceOfVecs::new_mut(&mut res[..], 2, made);
                    let done = match (input, output) {
                        (Ok(input), Ok(mut output)) => rs.process_into_buffer(&input, &mut output, None).map_err(|e| e.to_string()),
                        (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
                    };
                    match done {
                        Ok((_, n)) => self.take(&res[0][..n], &res[1][..n], finish, &mut out),
                        Err(e) => {
                            debug_assert!(false, "resampler: {e}");
                            res[0].fill(0.0);
                            res[1].fill(0.0);
                            self.take(&res[0], &res[1], finish, &mut out);
                        }
                    }
                    self.resampled = res;
                    self.pending[0].drain(..need);
                    self.pending[1].drain(..need);
                }
            }
        }
        out
    }

    fn flush(&mut self) -> Option<Vec<f32>> {
        if self.finished {
            return None;
        }
        let mut out = self.drain(true);
        self.finished = true;
        if let Some(rs) = &self.resampler {
            let keep = self.tail.len().min(2 * rs.output_delay());
            self.tail.truncate(keep);
        }
        self.mix_carry(&mut out);
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn seek(&mut self, seconds: f64) -> f64 {
        let actual = self.backend.seek(seconds);
        self.pending[0].clear();
        self.pending[1].clear();
        if let Some(rs) = self.resampler.as_mut() {
            rs.reset();
            self.skip = rs.output_delay();
        }
        self.fed = 0;
        self.emitted = 0;
        self.finished = false;
        self.ended = false;
        self.own = 1;
        self.tail.clear();
        self.carry.clear();
        actual
    }
}

struct Marker {
    start: u64,
    index: usize,
    base: f64,
    duration: f64,
    meta: TrackMeta,
}

struct Resume {
    index: usize,
    seconds: f64,
    saved: bool,
    meta: Option<TrackMeta>,
    duration: f64,
}

impl Resume {
    fn saved(queue: &[PathBuf], index: usize, seconds: f64) -> Resume {
        let (meta, duration) = match crate::decode::open(&queue[index]) {
            Ok(o) => {
                let mut meta = o.meta;
                if meta.title.is_empty() {
                    meta.title = crate::fsx::title_from_path(&queue[index]);
                }
                (Some(meta), o.duration)
            }
            Err(_) => (None, 0.0),
        };
        Resume { index, seconds, saved: true, meta, duration }
    }
}

struct Player {
    shared: Arc<Shared>,
    out_rate: u32,
    ring: Arc<Ring>,
    queue: Arc<Vec<PathBuf>>,
    decoding: usize,
    source: Option<Source>,
    produced: u64,
    staged: Vec<f32>,
    staged_pos: usize,
    markers: VecDeque<Marker>,
    active: bool,
    error: Option<String>,
    repeat: Repeat,
    misses: usize,
    device: String,
    plays: u64,
    lost: Option<Resume>,
    next_run: usize,
    last_audible: Option<usize>,
    hold: Vec<f32>,
    fade: Vec<f32>,
    fade_at: usize,
    last_fade: Option<(u64, Vec<f32>)>,
}

fn new_ring(rate: u32) -> Arc<Ring> {
    Arc::new(Ring::new(rate as usize))
}

impl Player {
    fn new(shared: Arc<Shared>, out_rate: u32, error: Option<String>) -> Player {
        Player {
            shared,
            out_rate,
            ring: new_ring(out_rate),
            queue: Arc::new(Vec::new()),
            decoding: 0,
            source: None,
            produced: 0,
            staged: Vec::new(),
            staged_pos: 0,
            markers: VecDeque::new(),
            active: false,
            error,
            repeat: Repeat::Off,
            misses: 0,
            device: String::new(),
            plays: 0,
            lost: None,
            next_run: 0,
            last_audible: None,
            hold: Vec::new(),
            fade: Vec::new(),
            fade_at: 0,
            last_fade: None,
        }
    }

    fn clear_buffer(&mut self) {
        self.ring.clear();
        self.staged.clear();
        self.staged_pos = 0;
        self.produced = self.shared.consumed.load(Ordering::Relaxed);
        self.markers.clear();
        self.hold.clear();
        self.fade.clear();
        self.fade_at = 0;
        self.last_fade = None;
        let mut eq = self.shared.eq.lock().unwrap();
        eq.slots.clear();
        eq.version += 1;
    }

    fn push_eq_slot(&self, index: usize) {
        let song = self.queue[index].clone();
        let curve = self.shared.store.lock().unwrap().get(&song);
        let filter = Filter::compile(&curve, self.out_rate);
        let mut eq = self.shared.eq.lock().unwrap();
        eq.slots.push_back(EqSlot { start: self.produced, song, curve, filter });
        eq.version += 1;
    }

    fn step(&self, index: usize) -> Option<usize> {
        if index.saturating_add(1) < self.queue.len() {
            Some(index + 1)
        } else if self.repeat == Repeat::All && !self.queue.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    fn follower(&self, index: usize) -> Option<usize> {
        match self.repeat {
            Repeat::One => Some(index),
            Repeat::Off | Repeat::All => self.step(index),
        }
    }

    fn open_from(&mut self, mut index: usize) -> bool {
        while index < self.queue.len() && self.misses < self.queue.len() {
            let began = Instant::now();
            let opened = Source::open(&self.queue[index], self.out_rate);
            if crate::trace::on() {
                let ms = began.elapsed().as_millis();
                let ahead = self.ring.len() as f64 / self.out_rate.max(1) as f64;
                match &opened {
                    Ok(src) => tlog!(
                        "open song {}: {} ({:.1} s, {} Hz) took {ms} ms, {ahead:.2} s already in the buffer{}",
                        index + 1,
                        song_name(&self.queue[index]),
                        src.duration,
                        src.in_rate,
                        if ms > 150 { "  <-- SLOW" } else { "" }
                    ),
                    Err(e) => tlog!("open song {}: {} FAILED after {ms} ms: {e}", index + 1, song_name(&self.queue[index])),
                }
            }
            match opened {
                Ok(src) => {
                    self.markers.push_back(Marker {
                        start: self.produced,
                        index,
                        base: 0.0,
                        duration: src.duration,
                        meta: src.meta.clone(),
                    });
                    self.decoding = index;
                    self.source = Some(src);
                    self.push_eq_slot(index);
                    self.error = None;
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!(
                        "can't play {}: {e}",
                        self.queue[index].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                    ));
                    self.misses += 1;
                    match self.step(index) {
                        Some(next) => index = next,
                        None => break,
                    }
                }
            }
        }
        self.source = None;
        false
    }

    fn start(&mut self, queue: Vec<PathBuf>, index: usize) {
        self.queue = Arc::new(queue);
        self.start_at(index);
    }

    fn start_at(&mut self, index: usize) {
        self.misses = 0;
        self.clear_buffer();
        self.shared.paused.store(false, Ordering::Relaxed);
        self.active = self.open_from(index);
    }

    fn stop(&mut self) {
        self.active = false;
        self.source = None;
        self.lost = None;
        self.clear_buffer();
    }

    fn restart_at(&mut self, index: usize, seconds: f64) -> bool {
        self.misses = 0;
        self.clear_buffer();
        if !self.open_from(index) {
            return false;
        }
        if self.markers.back().is_some_and(|m| m.index == index) {
            let actual = self.source.as_mut().unwrap().seek(seconds);
            if let Some(m) = self.markers.back_mut() {
                m.base = actual;
            }
        }
        true
    }

    fn audible(&self) -> Option<&Marker> {
        let consumed = self.shared.consumed.load(Ordering::Relaxed);
        self.markers.iter().rev().find(|m| m.start <= consumed).or(self.markers.front())
    }

    fn position_in(&self, m: &Marker) -> f64 {
        let consumed = self.shared.consumed.load(Ordering::Relaxed);
        let played = consumed.saturating_sub(m.start) as f64 / self.out_rate as f64;
        (m.base + played).min(if m.duration > 0.0 { m.duration } else { f64::MAX })
    }

    fn position(&self) -> Option<f64> {
        self.audible().map(|m| self.position_in(m))
    }

    fn seek(&mut self, seconds: f64) {
        let Some(index) = self.audible().map(|m| m.index) else { return };
        if index != self.decoding || self.source.is_none() {
            self.clear_buffer();
            if !self.open_from(index) {
                return;
            }
        } else {
            self.clear_buffer();
            let meta = self.source.as_ref().unwrap().meta.clone();
            let duration = self.source.as_ref().unwrap().duration;
            self.markers.push_back(Marker { start: self.produced, index, base: 0.0, duration, meta });
            self.push_eq_slot(index);
        }
        let actual = self.source.as_mut().unwrap().seek(seconds);
        if let Some(m) = self.markers.back_mut() {
            m.base = actual;
        }
    }

    fn fill(&mut self) -> bool {
        if !self.active {
            return false;
        }
        if self.staged_pos < self.staged.len() {
            self.staged_pos += self.ring.push(&self.staged[self.staged_pos..]) * 2;
            if self.staged_pos < self.staged.len() {
                return false;
            }
            self.staged.clear();
            self.staged_pos = 0;
            return true;
        }
        if self.ring.len() as u64 > self.out_rate as u64 {
            return false;
        }
        let Some(src) = self.source.as_mut() else { return false };
        match src.next_block() {
            Some(mut frames) => {
                src.apply_gain(&mut frames, self.shared.replaygain());
                self.put(frames);
            }
            None => self.next_song(),
        }
        true
    }

    fn crossfade_frames(&self) -> usize {
        (self.shared.crossfade_ms.load(Ordering::Relaxed) as u64 * self.out_rate as u64 / 1000) as usize
    }

    fn put(&mut self, mut frames: Vec<f32>) {
        if frames.is_empty() {
            return;
        }
        self.misses = 0;
        self.mix_fade(&mut frames);
        let mut keep = self.crossfade_frames() * 2;
        if keep > 0 && self.hold.is_empty() && self.far_from_end(frames.len() / 2) {
            keep = 0;
        }
        if keep == 0 && self.hold.is_empty() {
            self.emit(frames);
            return;
        }
        self.hold.extend(frames);
        if self.hold.len() > keep {
            let out: Vec<f32> = self.hold.drain(..self.hold.len() - keep).collect();
            self.emit(out);
        }
    }

    fn far_from_end(&self, more: usize) -> bool {
        let Some(m) = self.markers.back().filter(|m| m.index == self.decoding && m.duration > 0.0) else { return false };
        let done = m.base + (self.produced + more as u64).saturating_sub(m.start) as f64 / self.out_rate as f64;
        let fade = self.shared.crossfade_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        m.duration - done > fade + 2.0
    }

    fn release_hold(&mut self) {
        if !self.hold.is_empty() {
            let held = std::mem::take(&mut self.hold);
            self.emit(held);
        }
    }

    fn mix_fade(&mut self, frames: &mut [f32]) {
        let total = self.fade.len() / 2;
        if total == 0 {
            return;
        }
        for f in frames.chunks_exact_mut(2) {
            if self.fade_at >= total {
                break;
            }
            let t = (self.fade_at as f32 + 0.5) / total as f32;
            let g = (t * std::f32::consts::FRAC_PI_2).sin();
            f[0] = f[0] * g + self.fade[2 * self.fade_at];
            f[1] = f[1] * g + self.fade[2 * self.fade_at + 1];
            self.fade_at += 1;
        }
        if self.fade_at >= total {
            self.fade.clear();
            self.fade_at = 0;
        }
    }

    fn begin_fade(&mut self, end: &[f32]) {
        let n = end.len() / 2;
        self.fade.clear();
        self.fade.reserve(end.len());
        for (i, f) in end.chunks_exact(2).enumerate() {
            let t = (i as f32 + 0.5) / n as f32;
            let g = (t * std::f32::consts::FRAC_PI_2).cos();
            self.fade.push(f[0] * g);
            self.fade.push(f[1] * g);
        }
        self.fade_at = 0;
        let mut eq = self.shared.eq.lock().unwrap();
        if let Some(slot) = eq.slots.back_mut() {
            slot.start = self.produced + n as u64 / 2;
        }
        eq.version += 1;
    }

    fn cross_to(&mut self, mut prev: Source, next: Option<usize>) {
        if let Some(mut frames) = prev.flush() {
            prev.apply_gain(&mut frames, self.shared.replaygain());
            self.put(frames);
        }
        if !self.fade.is_empty() {
            let rest = vec![0.0; self.fade.len() - 2 * self.fade_at];
            self.put(rest);
        }
        let end = std::mem::take(&mut self.hold);
        if next.is_some_and(|i| self.open_from(i)) {
            self.begin_fade(&end);
            self.last_fade = Some((self.produced, end));
        } else {
            self.last_fade = None;
            self.emit(end);
        }
    }

    fn emit(&mut self, frames: Vec<f32>) {
        self.produced += frames.len() as u64 / 2;
        self.misses = 0;
        if self.staged_pos < self.staged.len() {
            self.staged.extend(frames);
            return;
        }
        let n = self.ring.push(&frames);
        if n * 2 < frames.len() {
            self.staged = frames;
            self.staged_pos = n * 2;
        }
    }

    fn next_song(&mut self) {
        let Some(mut prev) = self.source.take() else { return };
        if crate::trace::on() {
            if let Some(m) = self.markers.back().filter(|m| m.index == self.decoding) {
                let made = self.produced + (self.hold.len() / 2) as u64;
                let done = m.base + made.saturating_sub(m.start) as f64 / self.out_rate.max(1) as f64;
                let name = self.queue.get(self.decoding).map(|p| song_name(p)).unwrap_or_default();
                let early = m.duration > 0.0 && m.duration - done > 2.0;
                tlog!(
                    "decoder finished song {}: {name} at {done:.1} s of {:.1} s{}",
                    self.decoding + 1,
                    m.duration,
                    if early { "  <-- ENDED EARLY" } else { "" }
                );
            }
        }
        let next = if prev.own == 0 {
            self.misses += 1;
            self.step(self.decoding)
        } else {
            self.follower(self.decoding)
        };
        if self.crossfade_frames() > 0 {
            self.cross_to(prev, next);
            return;
        }
        self.release_hold();
        self.last_fade = None;
        let opened = next.is_some_and(|i| self.open_from(i));
        let mut ahead = 0;
        let handed = opened && self.source.as_ref().is_some_and(|s| prev.can_hand_to(s));
        if handed {
            ahead = self.source.as_mut().unwrap().adopt(&mut prev);
        } else {
            if let Some(mut frames) = prev.flush() {
                prev.apply_gain(&mut frames, self.shared.replaygain());
                let before = self.produced;
                self.put(frames);
                ahead = self.produced - before;
            }
            if let Some(src) = self.source.as_mut() {
                src.carry = std::mem::take(&mut prev.tail);
            }
        }
        if !opened || ahead == 0 {
            return;
        }
        let start = if handed { self.produced + ahead } else { self.produced };
        if let Some(m) = self.markers.back_mut() {
            m.start = start;
        }
        {
            let mut eq = self.shared.eq.lock().unwrap();
            if let Some(slot) = eq.slots.back_mut() {
                slot.start = start;
            }
            eq.version += 1;
        }
        let mode = self.shared.replaygain();
        while self.produced < start {
            let Some(src) = self.source.as_mut() else { break };
            match src.next_block() {
                Some(mut frames) => {
                    src.apply_gain(&mut frames, mode);
                    self.put(frames);
                }
                None => break,
            }
        }
    }

    fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
        self.reselect_follower();
    }

    fn reselect_follower(&mut self) {
        if !self.active {
            return;
        }
        let ring = self.ring.clone();
        let decision = ring.exclusive(|waiting| -> Option<(u64, Option<usize>)> {
            let consumed = self.shared.consumed.load(Ordering::Relaxed);
            let index = self.audible().map(|m| m.index)?;
            let follower = self.follower(index);
            let cut = match self.markers.iter().find(|m| m.start > consumed) {
                Some(next) if Some(next.index) != follower => next.start,
                None if self.source.is_none() && follower.is_some() => self.produced,
                _ => return None,
            };
            if cut < consumed + waiting.len() as u64 {
                waiting.truncate((cut - consumed) as usize);
            }
            Some((cut, follower))
        });
        let Some((cut, follower)) = decision else { return };
        let staged_frames = ((self.staged.len() - self.staged_pos) / 2) as u64;
        let ring_end = self.produced - staged_frames;
        if cut >= ring_end {
            self.staged.truncate(self.staged_pos + 2 * (cut - ring_end) as usize);
        } else {
            self.staged.clear();
        }
        if self.staged_pos >= self.staged.len() {
            self.staged.clear();
            self.staged_pos = 0;
        }
        self.produced = cut;
        self.markers.retain(|m| m.start < cut);
        {
            let mut eq = self.shared.eq.lock().unwrap();
            eq.slots.retain(|s| s.start < cut);
            eq.version += 1;
        }
        self.source = None;
        self.hold.clear();
        self.fade.clear();
        self.fade_at = 0;
        let redo = self.last_fade.take().filter(|(at, _)| *at == cut).map(|(_, end)| end);
        let opened = follower.is_some_and(|next| self.open_from(next));
        if let Some(end) = redo {
            if opened {
                self.begin_fade(&end);
                self.last_fade = Some((cut, end));
            } else {
                self.emit(end);
            }
        }
    }

    fn current_index(&self) -> Option<usize> {
        if self.active {
            self.audible().map(|m| m.index)
        } else {
            self.lost.as_ref().map(|r| r.index)
        }
    }

    fn edit_queue(&mut self, queue: Vec<PathBuf>, moved: &dyn Fn(usize) -> Option<usize>) {
        self.queue = Arc::new(queue);
        for m in &mut self.markers {
            m.index = moved(m.index).unwrap_or(usize::MAX);
        }
        self.decoding = moved(self.decoding).unwrap_or(usize::MAX);
        if let Some(r) = self.lost.as_mut() {
            r.index = moved(r.index).unwrap_or(0).min(self.queue.len().saturating_sub(1));
        }
        self.reselect_follower();
        if self.decoding == usize::MAX {
            self.decoding = self.audible().map_or(0, |m| m.index);
        }
        self.last_audible = if self.active { self.audible().map(|m| m.index) } else { None };
    }

    fn enqueue(&mut self, paths: Vec<PathBuf>, next: bool, can_start: bool) {
        if paths.is_empty() {
            return;
        }
        let n = paths.len();
        let len = self.queue.len();
        let current = self.current_index();
        let at = match current {
            Some(c) if next => (c + 1 + self.next_run).min(len),
            _ => len,
        };
        if next && current.is_some() {
            self.next_run += n;
        }
        let mut queue = (*self.queue).clone();
        queue.splice(at..at, paths);
        let moved = move |i: usize| Some(if i >= at { i + n } else { i });
        let idle = !self.active && self.lost.is_none();
        self.edit_queue(queue, &moved);
        if idle && can_start {
            self.next_run = 0;
            self.start_at(at);
        }
    }

    fn jump(&mut self, index: usize, song: &Path) {
        if self.queue.get(index).map(PathBuf::as_path) != Some(song) {
            return;
        }
        self.next_run = 0;
        self.start_at(index);
    }

    fn remove(&mut self, index: usize, song: &Path) {
        if self.queue.get(index).map(PathBuf::as_path) != Some(song) {
            return;
        }
        let current = self.current_index();
        let mut queue = (*self.queue).clone();
        queue.remove(index);
        if current == Some(index) {
            self.next_run = 0;
            let after = if index < queue.len() {
                Some(index)
            } else if self.repeat == Repeat::All && !queue.is_empty() {
                Some(0)
            } else {
                None
            };
            let paused = self.shared.paused.load(Ordering::Relaxed);
            self.queue = Arc::new(queue);
            if self.active {
                match after {
                    Some(i) => {
                        self.start_at(i);
                        self.shared.paused.store(paused, Ordering::Relaxed);
                    }
                    None => self.stop(),
                }
            } else {
                let saved = self.lost.as_ref().is_some_and(|r| r.saved);
                self.lost = after.map(|index| Resume { saved, ..Resume::saved(&self.queue, index, 0.0) });
            }
            self.last_audible = None;
            return;
        }
        if let Some(c) = current {
            if index > c && index <= c + self.next_run {
                self.next_run -= 1;
            }
        }
        let moved = move |i: usize| match i.cmp(&index) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i - 1),
        };
        self.edit_queue(queue, &moved);
    }

    fn move_song(&mut self, from: usize, to: usize, song: &Path) {
        if from == to || to >= self.queue.len() || self.queue.get(from).map(PathBuf::as_path) != Some(song) {
            return;
        }
        self.next_run = 0;
        let mut queue = (*self.queue).clone();
        let item = queue.remove(from);
        queue.insert(to, item);
        let moved = move |i: usize| {
            Some(if i == from {
                to
            } else if from < to && i > from && i <= to {
                i - 1
            } else if to < from && i >= to && i < from {
                i + 1
            } else {
                i
            })
        };
        self.edit_queue(queue, &moved);
    }

    fn clear(&mut self) {
        self.next_run = 0;
        let keep = self.current_index();
        let queue = keep.map(|c| vec![self.queue[c].clone()]).unwrap_or_default();
        self.edit_queue(queue, &move |i| (Some(i) == keep).then_some(0));
    }

    fn shuffle_upcoming(&mut self, seed: u64) {
        let from = self.current_index().map_or(0, |c| c + 1);
        if self.queue.len().saturating_sub(from) < 2 {
            return;
        }
        let mut order: Vec<usize> = (0..self.queue.len() - from).collect();
        crate::queue::shuffle(&mut order, seed);
        self.put_upcoming(from, &order);
    }

    fn reorder(&mut self, upcoming: &[PathBuf], order: &[usize]) {
        let from = self.current_index().map_or(0, |c| c + 1);
        let now = self.queue.get(from..).unwrap_or_default();
        let mut check: Vec<usize> = order.to_vec();
        check.sort_unstable();
        if now != upcoming || check.iter().copied().ne(0..upcoming.len()) {
            return;
        }
        self.put_upcoming(from, order);
    }

    fn put_upcoming(&mut self, from: usize, order: &[usize]) {
        self.next_run = 0;
        let order: Vec<usize> = order.iter().map(|o| from + o).collect();
        let mut queue = (*self.queue).clone();
        let mut now_at = std::collections::HashMap::new();
        for (place, &old) in (from..).zip(&order) {
            queue[place] = self.queue[old].clone();
            now_at.insert(old, place);
        }
        self.edit_queue(queue, &move |i| Some(now_at.get(&i).copied().unwrap_or(i)));
    }

    fn load(&mut self, queue: Vec<PathBuf>, at: Option<(usize, f64)>) {
        if self.active || self.lost.is_some() || !self.queue.is_empty() {
            return;
        }
        self.next_run = 0;
        self.queue = Arc::new(queue);
        if let Some((index, seconds)) = at.filter(|&(i, _)| i < self.queue.len()) {
            self.lost = Some(Resume::saved(&self.queue, index, seconds));
        }
    }

    fn resume_held(&mut self) {
        if let Some(r) = self.lost.take() {
            self.active = self.restart_at(r.index, r.seconds);
        }
    }

    fn move_held(&mut self, cmd: &Cmd) {
        let len = self.queue.len();
        let Some(r) = self.lost.as_mut() else { return };
        let to = match *cmd {
            Cmd::Seek(s) => {
                r.seconds = s.max(0.0);
                return;
            }
            Cmd::SeekBy(d) => {
                r.seconds = (r.seconds + d).max(0.0);
                return;
            }
            Cmd::Next if r.index + 1 < len => r.index + 1,
            Cmd::Previous if r.seconds > 3.0 || r.index == 0 => {
                r.seconds = 0.0;
                return;
            }
            Cmd::Previous => r.index - 1,
            _ => return,
        };
        let saved = r.saved;
        let mut moved = Resume::saved(&self.queue, to, 0.0);
        moved.saved = saved;
        self.lost = Some(moved);
    }

    fn detach(&mut self) {
        if self.active {
            if let (Some(index), Some(seconds)) = (self.audible().map(|m| m.index), self.position()) {
                self.lost = Some(Resume { index, seconds, saved: false, meta: None, duration: 0.0 });
            }
        }
        self.active = false;
        self.source = None;
        self.clear_buffer();
        self.device.clear();
        self.error = None;
    }

    fn attach(&mut self, ring: Arc<Ring>, rate: u32, device: String) {
        self.shared.tap.set_rate(rate);
        self.shared.rate.store(rate, Ordering::Relaxed);
        self.ring = ring;
        self.out_rate = rate;
        self.device = device;
        self.error = None;
        self.staged.clear();
        self.staged_pos = 0;
        if let Some(r) = self.lost.take() {
            self.active = self.restart_at(r.index, r.seconds);
        }
    }

    fn publish(&mut self) {
        let consumed = self.shared.consumed.load(Ordering::Relaxed);
        let mut played_out = false;
        while self.markers.len() > 1 && self.markers[1].start <= consumed {
            self.markers.pop_front();
            played_out = true;
        }
        if played_out {
            let mut eq = self.shared.eq.lock().unwrap();
            while eq.slots.len() > 1 && eq.slots[1].start <= consumed {
                eq.slots.pop_front();
            }
        }
        if self.active && self.source.is_none() && self.ring.is_empty() && self.staged.is_empty() {
            self.active = false;
            self.markers.clear();
        }
        let now = if self.active { self.audible().map(|m| m.index) } else { None };
        if now != self.last_audible {
            let stepped = matches!((self.last_audible, now), (Some(a), Some(b)) if b == a + 1);
            self.next_run = if stepped { self.next_run.saturating_sub(1) } else { 0 };
            self.last_audible = now;
        }
        let paused = self.shared.paused.load(Ordering::Relaxed);
        let audible = self.audible().map(|m| (m, self.position_in(m)));
        let mut st = self.shared.status.lock().unwrap();
        if !Arc::ptr_eq(&st.queue, &self.queue) {
            st.queue = self.queue.clone();
        }
        if st.device != self.device {
            st.device = self.device.clone();
        }
        if st.error != self.error {
            st.error = self.error.clone();
        }
        st.acked = self.shared.handled.load(Ordering::Acquire);
        st.plays = self.plays;
        st.output_lost = !self.active && self.lost.as_ref().is_some_and(|r| !r.saved);
        if !self.active {
            match &self.lost {
                Some(r) => {
                    st.state = State::Paused;
                    st.index = r.index;
                    st.position = r.seconds;
                    if let Some(meta) = r.meta.as_ref().filter(|m| **m != st.meta) {
                        st.meta = meta.clone();
                    }
                    if r.meta.is_some() {
                        st.duration = r.duration;
                    }
                }
                None => {
                    st.state = State::Stopped;
                    st.position = 0.0;
                }
            }
            return;
        }
        st.state = if paused { State::Paused } else { State::Playing };
        if let Some((m, position)) = audible {
            st.index = m.index;
            st.position = position;
            st.duration = m.duration;
            if st.meta != m.meta {
                st.meta = m.meta.clone();
            }
        }
    }
}

struct Renderer {
    shared: Arc<Shared>,
    ring: Arc<Ring>,
    channels: usize,
    eq: EqState,
    gain: f32,
    ease: f32,
}

struct EqState {
    proc: Processor,
    slots: Vec<(u64, Filter)>,
    cut: bool,
    version: u64,
    enabled: bool,
    slot: usize,
    flat: Filter,
}

const MAX_SLOTS: usize = 16;

impl EqState {
    fn new(rate: u32) -> EqState {
        EqState {
            proc: Processor::new(rate),
            slots: Vec::with_capacity(MAX_SLOTS),
            cut: false,
            version: u64::MAX,
            enabled: false,
            slot: usize::MAX,
            flat: Filter::flat(),
        }
    }

    fn refresh(&mut self, shared: &Shared) {
        let Ok(state) = shared.eq.try_lock() else { return };
        if state.version == self.version {
            return;
        }
        self.version = state.version;
        self.enabled = state.enabled;
        self.slots.clear();
        self.slots.extend(state.slots.iter().take(MAX_SLOTS).map(|s| (s.start, s.filter)));
        self.cut = state.slots.len() > MAX_SLOTS;
        self.slot = usize::MAX;
    }

    #[inline]
    fn select(&mut self, pos: u64) {
        let mut idx = if self.slot == usize::MAX { 0 } else { self.slot };
        while idx + 1 < self.slots.len() && self.slots[idx + 1].0 <= pos {
            idx += 1;
        }
        if idx != self.slot {
            self.slot = idx;
            let filter = match self.slots.get(idx) {
                Some((_, f)) if self.enabled => *f,
                _ => self.flat,
            };
            self.proc.set_filter(&filter);
            if self.cut && idx + 1 == self.slots.len() {
                self.version = u64::MAX;
            }
        }
    }
}

impl Renderer {
    fn new(shared: Arc<Shared>, ring: Arc<Ring>, channels: usize, rate: u32) -> Renderer {
        let gain = Renderer::target_gain(&shared);
        Renderer {
            shared,
            ring,
            channels: channels.max(1),
            eq: EqState::new(rate),
            gain,
            ease: 1.0 - (-1.0 / (0.005 * rate as f32)).exp(),
        }
    }

    fn target_gain(shared: &Shared) -> f32 {
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        volume * volume
    }

    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, out: &mut [T]) {
        let Renderer { shared, ring, channels, eq, gain, ease } = self;
        eq.refresh(shared);
        let target = Renderer::target_gain(shared);
        let paused = shared.paused.load(Ordering::Relaxed);
        let mut reader = if paused { None } else { ring.reader() };
        if !paused && reader.is_none() {
            shared.gate_busy.fetch_add(1, Ordering::Relaxed);
        }
        let mut dry = 0u64;
        let mut tap = shared.tap.writer();
        let first = shared.consumed.load(Ordering::Relaxed);
        let mut pos = first;
        for frame in out.chunks_mut(*channels) {
            let (l, r) = if let Some((l, r)) = reader.as_mut().and_then(|r| r.pop()) {
                eq.select(pos);
                pos += 1;
                let (l, r) = eq.proc.process(l, r);
                tap.push_stereo((l + r) * 0.5, l, r);
                *gain += (target - *gain) * *ease;
                (l * *gain, r * *gain)
            } else {
                if reader.is_some() {
                    dry += 1;
                }
                tap.push(0.0);
                (0.0, 0.0)
            };
            match frame {
                [mono] => *mono = T::from_sample((l + r) * 0.5),
                _ => {
                    for (c, s) in frame.iter_mut().enumerate() {
                        *s = T::from_sample(match c {
                            0 => l,
                            1 => r,
                            _ => 0.0,
                        });
                    }
                }
            }
        }
        shared.consumed.fetch_add(pos - first, Ordering::Relaxed);
        if dry > 0 {
            shared.starved.fetch_add(dry, Ordering::Relaxed);
        }
        drop(reader);
    }
}

struct Output {
    _stream: cpal::Stream,
    ring: Arc<Ring>,
    rate: u32,
    name: String,
    id: Option<cpal::DeviceId>,
    lost: Arc<AtomicBool>,
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mut renderer: Renderer, lost: Arc<AtomicBool>) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let lost_here = lost.clone();
    let on_error = move |e: cpal::Error| {
        if matches!(
            e.kind(),
            cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::DeviceChanged
        ) {
            lost.store(true, Ordering::Relaxed);
        }
    };
    let data = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        if lost_here.load(Ordering::Relaxed) {
            out.fill(T::EQUILIBRIUM);
        } else {
            renderer.fill(out);
        }
    };
    Ok(device.build_output_stream(config, data, on_error, None)?)
}

fn open_output(host: &cpal::Host, shared: &Arc<Shared>) -> Result<Output> {
    let began = Instant::now();
    let opened = open_output_now(host, shared);
    let ms = began.elapsed().as_millis();
    match &opened {
        Ok(o) => tlog!("output opened: {} at {} Hz, took {ms} ms", o.name, o.rate),
        Err(e) => tlog!("output failed to open after {ms} ms: {e}"),
    }
    opened
}

fn open_output_now(host: &cpal::Host, shared: &Arc<Shared>) -> Result<Output> {
    let default = host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?;
    let id = default.id().ok();
    let device = id
        .as_ref()
        .and_then(|id| host.output_devices().ok()?.find(|d| d.id().ok().as_ref() == Some(id)))
        .unwrap_or(default);
    let name = device.description().map_or_else(|_| "default output".into(), |d| d.name().to_string());
    let supported = keep_cd_rate(&device, device.default_output_config()?);
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    tlog!("output config: {channels} channels, {format:?}, buffer size {:?}", config.buffer_size);

    let ring = new_ring(rate);
    let lost = Arc::new(AtomicBool::new(false));
    let renderer = Renderer::new(shared.clone(), ring.clone(), channels, rate);
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, renderer, lost.clone())?,
        cpal::SampleFormat::I16 => build::<i16>(&device, config, renderer, lost.clone())?,
        cpal::SampleFormat::U16 => build::<u16>(&device, config, renderer, lost.clone())?,
        cpal::SampleFormat::I32 => build::<i32>(&device, config, renderer, lost.clone())?,
        cpal::SampleFormat::I24 => build::<cpal::I24>(&device, config, renderer, lost.clone())?,
        other => return Err(anyhow!("unsupported output sample format {other:?}")),
    };
    stream.play()?;
    #[cfg(test)]
    shared.opens.fetch_add(1, Ordering::Relaxed);
    Ok(Output { _stream: stream, ring, rate, name, id, lost })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn keep_cd_rate(device: &cpal::Device, default: cpal::SupportedStreamConfig) -> cpal::SupportedStreamConfig {
    let Ok(mut ranges) = device.supported_output_configs() else { return default };
    ranges
        .find(|r| {
            r.sample_format() == default.sample_format() && r.channels() == default.channels() && r.contains_rate(cpal::SAMPLE_RATE_CD)
        })
        .map_or(default, |r| r.with_sample_rate(cpal::SAMPLE_RATE_CD))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn keep_cd_rate(_device: &cpal::Device, default: cpal::SupportedStreamConfig) -> cpal::SupportedStreamConfig {
    default
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
const DEVICE_CHECK: Duration = Duration::from_secs(1);
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const DEVICE_CHECK: Duration = Duration::from_secs(5);
const DEFAULT_POLL: Duration = Duration::from_millis(250);
const UNLISTED: Duration = Duration::from_secs(2);
const STALL: Duration = Duration::from_secs(2);

struct Watchdog {
    consumed: u64,
    since: Instant,
}

impl Watchdog {
    fn new() -> Watchdog {
        Watchdog { consumed: 0, since: Instant::now() }
    }

    fn stalled(&mut self, p: &Player) -> bool {
        let consumed = p.shared.consumed.load(Ordering::Relaxed);
        let should_move = p.active && !p.shared.paused.load(Ordering::Relaxed) && !p.ring.is_empty();
        if consumed != self.consumed || !should_move {
            self.consumed = consumed;
            self.since = Instant::now();
            return false;
        }
        self.since.elapsed() > STALL
    }
}

const POLL_PLAYING: Duration = Duration::from_millis(25);
const POLL_PAUSED: Duration = Duration::from_millis(100);
const POLL_IDLE: Duration = Duration::from_millis(250);
const POLL_ASLEEP: Duration = Duration::from_secs(1);

const IDLE_RELEASE: Duration = Duration::from_secs(10);

const LOSS_ECHO: Duration = Duration::from_secs(5);

fn poll_wait(p: &Player, has_output: bool) -> Duration {
    if !has_output {
        POLL_ASLEEP
    } else if !p.active {
        POLL_IDLE
    } else if p.shared.paused.load(Ordering::Relaxed) {
        POLL_PAUSED
    } else {
        POLL_PLAYING
    }
}

fn default_output_id(host: &cpal::Host) -> Option<cpal::DeviceId> {
    host.default_output_device()?.id().ok()
}

fn listed_outputs(host: &cpal::Host) -> Option<(Vec<cpal::DeviceId>, bool)> {
    let devices = host.output_devices().ok()?;
    let mut ids = Vec::new();
    let mut unreadable = false;
    for d in devices {
        match d.id() {
            Ok(id) => ids.push(id),
            Err(_) => unreadable = true,
        }
    }
    Some((ids, unreadable))
}

enum Look {
    Stay,
    Vanished,
    Follow(String),
    Unlisted(String),
}

fn look_around(
    host: &cpal::Host,
    current: Option<&cpal::DeviceId>,
    follower: &mut Follower<cpal::DeviceId>,
    noted: &mut String,
) -> Look {
    let Some((present, unreadable)) = listed_outputs(host) else { return Look::Stay };
    if crate::trace::on() {
        let now = Instant::now();
        for id in present.iter().filter(|id| follower.seen_for(id, now).is_none()) {
            tlog!("output connected: {} [{id}]", output_name(host, id));
        }
        for id in follower.listed().filter(|id| !present.contains(id)) {
            tlog!("output left the device list: [{id}]");
        }
    }
    if current.is_some_and(|id| !unreadable && !present.contains(id)) {
        return Look::Vanished;
    }
    let default = host.default_output_device();
    let default_id = default.as_ref().and_then(|d| d.id().ok());
    let now = Instant::now();
    match follower.check(&present, default_id.as_ref(), current, now) {
        Some(_) => {
            let name = default
                .and_then(|d| d.description().ok().map(|d| d.name().to_string()))
                .unwrap_or_else(|| "the new default output".into());
            Look::Follow(name)
        }
        None => {
            if crate::trace::on() {
                note_not_following(host, default_id.as_ref(), current, follower, now, noted);
            }
            match (default_id.as_ref(), current) {
                (Some(d), Some(c)) if d != c && !present.contains(d) => Look::Unlisted(
                    default
                        .and_then(|dev| dev.description().ok().map(|desc| desc.name().to_string()))
                        .unwrap_or_else(|| d.to_string()),
                ),
                _ => Look::Stay,
            }
        }
    }
}

fn left_behind(
    host: &cpal::Host,
    left: &mut Option<(cpal::DeviceId, Instant, String)>,
    last_look: &mut Instant,
) -> Option<String> {
    let (id, at, name) = left.as_ref()?;
    if at.elapsed() >= follower::LEFT_BEHIND {
        *left = None;
        return None;
    }
    if last_look.elapsed() < DEFAULT_POLL {
        return None;
    }
    *last_look = Instant::now();
    let (present, unreadable) = listed_outputs(host)?;
    if unreadable || present.contains(id) {
        return None;
    }
    let why = format!("{name} went away just after the system moved the output off it");
    *left = None;
    Some(why)
}

fn output_name(host: &cpal::Host, id: &cpal::DeviceId) -> String {
    host.output_devices()
        .ok()
        .and_then(|mut all| all.find(|d| d.id().ok().as_ref() == Some(id)))
        .and_then(|d| d.description().ok().map(|d| d.name().to_string()))
        .unwrap_or_else(|| id.to_string())
}

fn trace_outputs(host: &cpal::Host) {
    let default = default_output_id(host);
    for id in listed_outputs(host).map(|(ids, _)| ids).unwrap_or_default() {
        let mark = if Some(&id) == default.as_ref() { "  <-- system default" } else { "" };
        tlog!("output at startup: {} [{id}]{mark}", output_name(host, &id));
    }
    if default.is_none() {
        tlog!("no system default output at startup");
    }
}

fn note_not_following(
    host: &cpal::Host,
    default: Option<&cpal::DeviceId>,
    current: Option<&cpal::DeviceId>,
    follower: &Follower<cpal::DeviceId>,
    now: Instant,
    noted: &mut String,
) {
    let note = match (default, current) {
        (Some(d), Some(c)) if d != c => {
            let why = match follower.seen_for(d, now) {
                None => "not moving there, it isn't in the output list",
                Some(Some(age)) if age <= follower::NEW_DEVICE => "it was just connected, moving there",
                Some(_) => "it was already connected, moving there in 1 s if this output is still listed",
            };
            format!("the system default output is {} [{d}]; {why}", output_name(host, d))
        }
        _ => String::new(),
    };
    if note != *noted {
        if !note.is_empty() {
            tlog!("{note}");
        }
        *noted = note;
    }
}

fn engine_thread(shared: Arc<Shared>, rx: Receiver<Cmd>) {
    let mut p = Player::new(shared.clone(), 48_000, None);
    let mut out: Option<Output> = None;
    let mut last_device_check = Instant::now();
    let host = cpal::default_host();
    let mut follower = Follower::new(listed_outputs(&host).map(|(ids, _)| ids).unwrap_or_default());
    if crate::trace::on() {
        trace_outputs(&host);
    }
    let mut noted = String::new();
    let mut last_default_poll = Instant::now();
    let mut last_default = default_output_id(&host);
    let mut left: Option<(cpal::DeviceId, Instant, String)> = None;
    let mut last_left_look = Instant::now();
    let mut unlisted_since: Option<Instant> = None;
    let mut watch = Watchdog::new();
    let mut idle_since: Option<Instant> = None;
    let mut lost_at: Option<Instant> = None;
    let mut quiet_until = Instant::now();

    match open_output(&host, &shared) {
        Ok(o) => {
            p.attach(o.ring.clone(), o.rate, o.name.clone());
            out = Some(o);
        }
        Err(e) => p.error = Some(format!("audio output unavailable: {e}")),
    }

    loop {
        let mut gone: Option<String> = None;
        let mut follow: Option<String> = None;
        if let Some(o) = out.as_ref() {
            if o.lost.load(Ordering::Relaxed) {
                gone = Some(format!("{} reported it is no longer available", o.name));
            } else if shared.take_forced_loss() {
                gone = Some("test: output lost on purpose".into());
            } else if watch.stalled(&p) {
                gone = Some(format!(
                    "{} took no sound for {} s while playing ({:.2} s waiting in the buffer)",
                    o.name,
                    STALL.as_secs(),
                    p.ring.len() as f64 / p.out_rate.max(1) as f64
                ));
            } else if let Some(why) = left_behind(&host, &mut left, &mut last_left_look) {
                gone = Some(why);
            } else if last_device_check.elapsed() >= DEVICE_CHECK || {
                let now = Instant::now();
                now.duration_since(last_default_poll) >= DEFAULT_POLL && {
                    last_default_poll = now;
                    let default = default_output_id(&host);
                    let changed = default != last_default;
                    if changed && crate::trace::on() {
                        let name = default.as_ref().map_or_else(|| "none".into(), |id| output_name(&host, id));
                        tlog!("the system default output changed to {name}");
                    }
                    last_default = default;
                    changed || follower.confirm_due(now)
                }
            } {
                last_device_check = Instant::now();
                let began = Instant::now();
                let look = look_around(&host, o.id.as_ref(), &mut follower, &mut noted);
                let ms = began.elapsed().as_millis();
                if ms >= 50 {
                    tlog!(
                        "checking the device list took {ms} ms ({:.2} s in the buffer){}",
                        p.ring.len() as f64 / p.out_rate.max(1) as f64,
                        if ms >= 200 { "  <-- SLOW" } else { "" }
                    );
                }
                match look {
                    Look::Vanished => gone = Some(format!("{} is no longer in the device list", o.name)),
                    Look::Follow(to) => {
                        unlisted_since = None;
                        follow = Some(to);
                    }
                    Look::Unlisted(name) => {
                        let since = *unlisted_since.get_or_insert_with(Instant::now);
                        if since.elapsed() >= UNLISTED && p.error.is_none() {
                            tlog!("the system default output {name} isn't in the output list; staying on {}", o.name);
                            p.error = Some(format!("can't switch to {name}: it isn't in the list of outputs"));
                        }
                    }
                    Look::Stay => unlisted_since = None,
                }
            }
        }
        if let Some(why) = &gone {
            tlog!(
                "OUTPUT GONE, playback held: {why} (song {:?} at {:?} s)",
                p.audible().map(|m| m.index + 1),
                p.position().map(|s| (s * 10.0).round() / 10.0)
            );
            out = None;
            p.detach();
            lost_at = Some(Instant::now());
        }
        if let Some(to) = follow {
            let from = out.as_ref().map(|o| o.name.clone()).unwrap_or_default();
            let from_id = out.as_ref().and_then(|o| o.id.clone());
            let paused = shared.paused.load(Ordering::Relaxed);
            if p.active {
                tlog!(
                    "FOLLOWING THE NEW DEFAULT OUTPUT: {from} -> {to} (song {:?} at {:?} s, paused: {paused})",
                    p.audible().map(|m| m.index + 1),
                    p.position().map(|s| (s * 10.0).round() / 10.0)
                );
                quiet_until = Instant::now() + Duration::from_millis(400);
                match open_output(&host, &shared) {
                    Ok(o) => {
                        p.detach();
                        p.attach(o.ring.clone(), o.rate, o.name.clone());
                        out = Some(o);
                        watch = Watchdog::new();
                        last_device_check = Instant::now();
                        quiet_until = Instant::now() + Duration::from_millis(400);
                        left = from_id.map(|id| (id, Instant::now(), from.clone()));
                        last_left_look = Instant::now();
                        if p.error.as_deref().is_some_and(|e| e.starts_with("can't switch to")) {
                            p.error = None;
                        }
                    }
                    Err(e) => {
                        tlog!("couldn't open {to}: {e}; staying on {from}");
                        p.error = Some(format!("can't switch to {to}: {e}"));
                    }
                }
            } else {
                tlog!("new default output {to}; closing the idle {from}");
                out = None;
                idle_since = None;
            }
        }

        match rx.recv_timeout(poll_wait(&p, out.is_some())) {
            Ok(cmd) => {
                let began = Instant::now();
                let what = if crate::trace::on() { cmd.describe() } else { String::new() };
                tlog!("command: {what} (playing: {}, paused: {})", p.active, p.shared.paused.load(Ordering::Relaxed));
                let cmd = match cmd {
                    Cmd::MediaToggle => {
                        let since = lost_at.map(|t| t.elapsed()).filter(|_| out.is_none() && p.lost.is_some());
                        match since {
                            Some(ago) if ago < LOSS_ECHO => {
                                tlog!("media Play/Pause ignored: the output went away {:.1} s ago", ago.as_secs_f64());
                                Cmd::MediaToggle
                            }
                            _ => Cmd::TogglePause,
                        }
                    }
                    other => other,
                };
                if cmd.cuts() {
                    quiet_until = Instant::now() + Duration::from_millis(400);
                }
                let resuming = matches!(cmd, Cmd::TogglePause) && p.lost.is_some();
                let had_output = out.is_some();
                if out.is_none() && (matches!(cmd, Cmd::Play(..) | Cmd::Jump(..)) || resuming) {
                    match open_output(&host, &shared) {
                        Ok(o) => {
                            p.attach(o.ring.clone(), o.rate, o.name.clone());
                            out = Some(o);
                            watch = Watchdog::new();
                            last_device_check = Instant::now();
                            quiet_until = Instant::now() + Duration::from_millis(400);
                            lost_at = None;
                        }
                        Err(e) => p.error = Some(format!("audio output unavailable: {e}")),
                    }
                }
                if resuming && out.is_some() {
                    if had_output {
                        p.resume_held();
                    }
                    p.shared.paused.store(false, Ordering::Relaxed);
                } else if out.is_some() {
                    handle(&mut p, cmd);
                } else {
                    handle_offline(&mut p, cmd);
                }
                shared.handled.fetch_add(1, Ordering::AcqRel);
                let ms = began.elapsed().as_millis();
                if ms >= 100 {
                    tlog!("command {what} took {ms} ms  <-- SLOW");
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let began = Instant::now();
        let before = p.ring.len();
        while p.fill() {
            if p.ring.len() as u64 > p.out_rate as u64 / 4 {
                break;
            }
        }
        let fill_ms = began.elapsed().as_millis();
        if fill_ms >= 100 {
            tlog!(
                "topping up the buffer took {fill_ms} ms, starting from {:.2} s in it  <-- SLOW",
                before as f64 / p.out_rate.max(1) as f64
            );
        }
        p.publish();

        let starved = shared.starved.swap(0, Ordering::Relaxed);
        let busy = shared.gate_busy.swap(0, Ordering::Relaxed);
        let playing = out.is_some() && p.active && p.source.is_some() && !shared.paused.load(Ordering::Relaxed);
        if playing && (starved > 0 || busy > 0) && Instant::now() >= quiet_until {
            tlog!(
                "UNDERRUN: {:.0} ms of silence{} during song {:?} at {:?} s; {:.2} s in the buffer now",
                starved as f64 * 1000.0 / p.out_rate.max(1) as f64,
                if busy > 0 { format!(" (+{busy} silent buffers while the engine held the ring)") } else { String::new() },
                p.audible().map(|m| m.index + 1),
                p.position().map(|s| (s * 10.0).round() / 10.0),
                p.ring.len() as f64 / p.out_rate.max(1) as f64
            );
        }

        if out.is_some() && !p.active {
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= shared.idle_release() {
                out = None;
                idle_since = None;
            }
        } else {
            idle_since = None;
        }
    }
}

fn handle_offline(p: &mut Player, cmd: Cmd) {
    match cmd {
        Cmd::Play(q, _) => {
            p.plays += 1;
            p.queue = Arc::new(q);
            p.stop();
        }
        Cmd::Stop => p.stop(),
        Cmd::Repeat(r) => p.repeat = r,
        Cmd::Enqueue { paths, next } => p.enqueue(paths, next, false),
        Cmd::Remove(i, song) => p.remove(i, &song),
        Cmd::Move { from, to, song } => p.move_song(from, to, &song),
        Cmd::Clear => p.clear(),
        Cmd::ShuffleUpcoming(seed) => p.shuffle_upcoming(seed),
        Cmd::Reorder { upcoming, order } => p.reorder(&upcoming, &order),
        Cmd::Load(q, at) => p.load(q, at),
        Cmd::Next | Cmd::Previous | Cmd::Seek(_) | Cmd::SeekBy(_) => p.move_held(&cmd),
        _ => {}
    }
}

fn handle(p: &mut Player, cmd: Cmd) {
    match cmd {
        Cmd::Play(q, i) => {
            p.plays += 1;
            p.start(q, i);
        }
        Cmd::TogglePause => {
            if p.active {
                let now = !p.shared.paused.load(Ordering::Relaxed);
                p.shared.paused.store(now, Ordering::Relaxed);
            }
        }
        Cmd::Stop => p.stop(),
        Cmd::Next => {
            if let Some(i) = p.audible().map(|m| m.index) {
                match p.step(i) {
                    Some(next) => p.start_at(next),
                    None => p.stop(),
                }
            }
        }
        Cmd::Previous => {
            let Some(i) = p.audible().map(|m| m.index) else { return };
            let played = p.position().unwrap_or(0.0);
            let before = match i {
                0 if p.repeat == Repeat::All => p.queue.len().checked_sub(1),
                0 => None,
                _ => Some(i - 1),
            };
            match before {
                Some(b) if played <= 3.0 => p.start_at(b),
                _ => p.seek(0.0),
            }
        }
        Cmd::Seek(s) => p.seek(s),
        Cmd::SeekBy(d) => {
            if let Some(now) = p.position() {
                p.seek((now + d).max(0.0));
            }
        }
        Cmd::Restart => {
            if p.active {
                p.shared.paused.store(false, Ordering::Relaxed);
                p.seek(0.0);
            }
        }
        Cmd::Repeat(r) => p.set_repeat(r),
        Cmd::Enqueue { paths, next } => p.enqueue(paths, next, true),
        Cmd::Jump(i, song) => p.jump(i, &song),
        Cmd::Remove(i, song) => p.remove(i, &song),
        Cmd::Move { from, to, song } => p.move_song(from, to, &song),
        Cmd::Clear => p.clear(),
        Cmd::ShuffleUpcoming(seed) => p.shuffle_upcoming(seed),
        Cmd::Reorder { upcoming, order } => p.reorder(&upcoming, &order),
        Cmd::Load(q, at) => p.load(q, at),
        Cmd::MediaToggle => {}
    }
}

#[cfg(test)]
pub struct TestPlayer {
    p: Player,
    out: Renderer,
}

#[cfg(test)]
impl TestPlayer {
    pub fn new(queue: Vec<PathBuf>, index: usize, repeat: Repeat) -> TestPlayer {
        let shared = Arc::new(Shared::new(1.0, Arc::new(Mutex::new(Store::default())), false));
        let mut p = Player::new(shared.clone(), 48_000, None);
        let out = Renderer::new(shared, p.ring.clone(), 2, 48_000);
        p.repeat = repeat;
        p.start(queue, index);
        TestPlayer { p, out }
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        handle(&mut self.p, Cmd::Repeat(repeat));
    }
    pub fn next(&mut self) {
        handle(&mut self.p, Cmd::Next);
    }
    pub fn previous(&mut self) {
        handle(&mut self.p, Cmd::Previous);
    }
    pub fn seek_by(&mut self, seconds: f64) {
        handle(&mut self.p, Cmd::SeekBy(seconds));
    }
    pub fn restart(&mut self) {
        handle(&mut self.p, Cmd::Restart);
    }
    pub fn toggle_pause(&mut self) {
        handle(&mut self.p, Cmd::TogglePause);
    }
    pub fn enqueue(&mut self, paths: Vec<PathBuf>, next: bool) {
        handle(&mut self.p, Cmd::Enqueue { paths, next });
    }
    pub fn jump(&mut self, index: usize) {
        let song = self.p.queue[index].clone();
        handle(&mut self.p, Cmd::Jump(index, song));
    }
    pub fn remove(&mut self, index: usize) {
        let song = self.p.queue[index].clone();
        handle(&mut self.p, Cmd::Remove(index, song));
    }
    pub fn move_song(&mut self, from: usize, to: usize) {
        let song = self.p.queue[from].clone();
        handle(&mut self.p, Cmd::Move { from, to, song });
    }
    pub fn clear_queue(&mut self) {
        handle(&mut self.p, Cmd::Clear);
    }
    pub fn shuffle_upcoming(&mut self, seed: u64) {
        handle(&mut self.p, Cmd::ShuffleUpcoming(seed));
    }
    pub fn queue(&self) -> Vec<PathBuf> {
        (*self.p.queue).clone()
    }
    pub fn playing(&self) -> bool {
        self.p.active
    }
    pub fn position(&self) -> Option<f64> {
        self.p.position()
    }
    pub fn status(&mut self) -> Status {
        self.p.publish();
        self.p.shared.status.lock().unwrap().clone()
    }
    pub fn lose_output(&mut self) {
        self.p.detach();
    }
    pub fn regain_output(&mut self, rate: u32) {
        let ring = new_ring(rate);
        self.out = Renderer::new(self.p.shared.clone(), ring.clone(), 2, rate);
        self.p.shared.rate.store(rate, Ordering::Relaxed);
        self.p.attach(ring, rate, "test output".into());
    }

    pub fn listen(&mut self, seconds: f64) -> Vec<usize> {
        self.run(seconds, |_| {})
    }

    pub fn run(&mut self, seconds: f64, mut sound: impl FnMut(&[f32])) -> Vec<usize> {
        let mut heard = Vec::new();
        let mut current = None;
        let mut buf = Vec::new();
        for _ in 0..(seconds * 100.0).round() as usize {
            let p = &mut self.p;
            let step = p.out_rate as usize / 100;
            while p.fill() {
                if p.ring.len() > p.out_rate as usize / 4 {
                    break;
                }
            }
            p.publish();
            if !p.active {
                break;
            }
            if let Some(m) = p.audible().filter(|m| current != Some(m.start)) {
                current = Some(m.start);
                heard.push(m.index);
            }
            buf.clear();
            buf.resize(step * 2, 0.0f32);
            self.out.fill(&mut buf[..]);
            sound(&buf);
        }
        heard
    }
}

#[cfg(test)]
pub struct TestSource(Source);

#[cfg(test)]
impl TestSource {
    pub fn next(&mut self) -> Option<Vec<f32>> {
        self.0.next_frames()
    }
}

#[cfg(test)]
pub fn test_open(path: &Path, out_rate: u32) -> Result<TestSource> {
    Source::open(path, out_rate).map(TestSource)
}

#[cfg(test)]
mod crossfade_tests;
#[cfg(test)]
mod renderer_tests;
