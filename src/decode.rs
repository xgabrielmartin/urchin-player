#[cfg(any(feature = "tracker", feature = "chiptune"))]
use std::ffi::{CStr, CString};
use std::fs::File;
#[cfg(any(feature = "tracker", feature = "chiptune"))]
#[allow(unused_imports)]
use std::os::raw::{c_char, c_double, c_int, c_short, c_void};
use std::path::Path;
#[cfg(feature = "ffmpeg")]
use std::sync::Once;

use anyhow::{anyhow, Result};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTag, Tag};
use symphonia::core::units::{Time, TimeBase};

use crate::audio::TrackMeta;

pub trait Backend {
    fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool;
    fn seek(&mut self, seconds: f64) -> f64;
}

pub struct Opened {
    pub backend: Box<dyn Backend>,
    pub rate: u32,
    pub duration: f64,
    pub meta: TrackMeta,
    pub gain: Gain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gain {
    pub track: Option<f32>,
    pub track_peak: Option<f32>,
    pub album: Option<f32>,
    pub album_peak: Option<f32>,
}

impl Gain {
    pub fn read(&mut self, key: &str, value: &str) {
        let key = key.to_ascii_lowercase();
        let ends = |name: &str| key.ends_with(name);
        if ends("replaygain_track_gain") {
            self.track = parse_db(value).or(self.track);
        } else if ends("replaygain_album_gain") {
            self.album = parse_db(value).or(self.album);
        } else if ends("replaygain_track_peak") {
            self.track_peak = parse_peak(value).or(self.track_peak);
        } else if ends("replaygain_album_peak") {
            self.album_peak = parse_peak(value).or(self.album_peak);
        } else if ends("r128_track_gain") {
            self.track = parse_r128(value).or(self.track);
        } else if ends("r128_album_gain") {
            self.album = parse_r128(value).or(self.album);
        }
    }
}

fn parse_db(value: &str) -> Option<f32> {
    let v = value.trim();
    let v = v.strip_suffix("dB").or_else(|| v.strip_suffix("db")).or_else(|| v.strip_suffix("DB")).unwrap_or(v);
    let db: f32 = v.trim().trim_start_matches('+').parse().ok()?;
    (db.is_finite() && db.abs() <= 60.0).then_some(db)
}

fn parse_peak(value: &str) -> Option<f32> {
    let p: f32 = value.trim().parse().ok()?;
    (p.is_finite() && p > 0.0 && p < 100.0).then_some(p)
}

fn parse_r128(value: &str) -> Option<f32> {
    let q: i32 = value.trim().parse().ok()?;
    Some(q as f32 / 256.0 + 5.0)
}

pub const EXTENSIONS: &[&str] = &[
    "mp3", "mp2", "mp1", "flac", "ogg", "oga", "m4a", "m4b", "mp4", "aac", "alac", "wav", "wave",
    "aif", "aiff", "aifc", "caf", "mka", "webm",
];

pub const FFMPEG_EXTENSIONS: &[&str] =
    &["opus", "wma", "wv", "ape", "mpc", "mp+", "mpp", "dsf", "dff", "tta", "ac3"];

pub const GME_EXTENSIONS: &[&str] =
    &["ay", "gbs", "gym", "hes", "kss", "nsf", "nsfe", "sap", "spc", "vgm", "vgz"];

pub fn is_playable(ext: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
        || (cfg!(feature = "ffmpeg") && FFMPEG_EXTENSIONS.contains(&ext.as_str()))
        || (cfg!(feature = "chiptune") && GME_EXTENSIONS.contains(&ext.as_str()))
        || openmpt::supports(&ext)
}

#[cfg(feature = "tracker")]
const MAX_MODULE_BYTES: u64 = 128 << 20;

#[cfg(feature = "chiptune")]
const MAX_CHIPTUNE_BYTES: u64 = 16 << 20;

fn regular_file(path: &Path) -> Result<u64> {
    let m = std::fs::metadata(path)?;
    if !m.is_file() {
        return Err(anyhow!("not a regular file"));
    }
    Ok(m.len())
}

fn guard<T>(what: &str, path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        crate::trace::tlog!("decoder panicked in {what}: {}", path.display());
        Err(anyhow!("the decoder crashed on this file"))
    })
}

struct Guarded {
    inner: Box<dyn Backend>,
    dead: bool,
}

impl Backend for Guarded {
    fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
        if self.dead {
            return false;
        }
        let (nl, nr) = (l.len(), r.len());
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.read(l, r))) {
            Ok(more) => more,
            Err(_) => {
                l.truncate(nl);
                r.truncate(nr);
                self.dead = true;
                crate::trace::tlog!("decoder panicked while reading; song ends early");
                false
            }
        }
    }

    fn seek(&mut self, seconds: f64) -> f64 {
        if self.dead {
            return seconds;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.seek(seconds))).unwrap_or_else(|_| {
            self.dead = true;
            crate::trace::tlog!("decoder panicked while seeking; song ends early");
            seconds
        })
    }
}

pub fn open(path: &Path) -> Result<Opened> {
    let size = regular_file(path)?;
    let o = guard("open", path, || open_unguarded(path, size))?;
    Ok(Opened { backend: Box::new(Guarded { inner: o.backend, dead: false }), ..o })
}

#[cfg_attr(not(any(feature = "tracker", feature = "chiptune")), allow(unused_variables))]
fn open_unguarded(path: &Path, size: u64) -> Result<Opened> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    #[cfg(feature = "chiptune")]
    if GME_EXTENSIONS.contains(&ext.as_str()) {
        if size > MAX_CHIPTUNE_BYTES {
            return Err(anyhow!("too big for a chiptune"));
        }
        return gme::open(path);
    }
    #[cfg(feature = "tracker")]
    if openmpt::supports(&ext) {
        if size > MAX_MODULE_BYTES {
            return Err(anyhow!("too big for a tracker module"));
        }
        return openmpt::open(path);
    }
    match open_symphonia(path) {
        Ok(o) => Ok(o),
        Err(e) => fallback(path, &ext, e),
    }
}

#[cfg(feature = "ffmpeg")]
fn fallback(path: &Path, ext: &str, e: anyhow::Error) -> Result<Opened> {
    let Some(demuxers) = ffmpeg::demuxers_for(ext) else { return Err(e) };
    ffmpeg::open(path, demuxers).map_err(|f| anyhow!("{e}; ffmpeg: {f}"))
}

#[cfg(not(feature = "ffmpeg"))]
fn fallback(_path: &Path, _ext: &str, e: anyhow::Error) -> Result<Opened> {
    Err(e)
}

struct Sym {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    rate: u32,
    planes: Vec<Vec<f32>>,
}

pub fn probe(path: &Path) -> Result<Box<dyn FormatReader>> {
    regular_file(path)?;
    guard("probe", path, || probe_unguarded(path))
}

fn probe_unguarded(path: &Path) -> Result<Box<dyn FormatReader>> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    Ok(symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())?)
}

pub fn each_tag(format: &mut dyn FormatReader, path: &Path, mut f: impl FnMut(&Tag)) {
    let mut log = format.metadata();
    loop {
        if let Some(rev) = log.current() {
            rev.media.tags.iter().chain(rev.per_track.iter().flat_map(|t| t.metadata.tags.iter())).for_each(&mut f);
        }
        if log.pop().is_none() {
            break;
        }
    }
    if format.format_info().short_name == "wave" {
        std::panic::catch_unwind(|| crate::wav_info::tags(path)).unwrap_or_default().iter().for_each(&mut f);
    }
}

pub fn tag_text(t: &Tag) -> String {
    match &t.std {
        Some(
            StandardTag::TrackTitle(v)
            | StandardTag::Artist(v)
            | StandardTag::AlbumArtist(v)
            | StandardTag::Album(v)
            | StandardTag::Genre(v)
            | StandardTag::ReplayGainTrackGain(v)
            | StandardTag::ReplayGainTrackPeak(v)
            | StandardTag::ReplayGainAlbumGain(v)
            | StandardTag::ReplayGainAlbumPeak(v),
        ) => v.to_string(),
        _ => t.raw.value.to_string(),
    }
}

fn open_symphonia(path: &Path) -> Result<Opened> {
    let mut format = probe_unguarded(path)?;
    let mut meta = TrackMeta::default();
    let mut gain = Gain::default();
    each_tag(format.as_mut(), path, |t| {
        let text = || tag_text(t);
        match &t.std {
            Some(StandardTag::TrackTitle(_)) => meta.title = text(),
            Some(StandardTag::Artist(_)) => meta.artist = text(),
            Some(StandardTag::Album(_)) => meta.album = text(),
            Some(StandardTag::ReplayGainTrackGain(_)) => gain.read("replaygain_track_gain", &text()),
            Some(StandardTag::ReplayGainTrackPeak(_)) => gain.read("replaygain_track_peak", &text()),
            Some(StandardTag::ReplayGainAlbumGain(_)) => gain.read("replaygain_album_gain", &text()),
            Some(StandardTag::ReplayGainAlbumPeak(_)) => gain.read("replaygain_album_peak", &text()),
            _ => gain.read(&t.raw.key, &text()),
        }
    });
    let track = format.first_track_known_codec(TrackType::Audio).ok_or_else(|| anyhow!("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("no audio track"))?
        .clone();
    let track_id = track.id;
    let time_base = track.time_base;
    let rate = params.sample_rate.unwrap_or(44_100);
    let duration = match (time_base, track.duration) {
        (Some(tb), Some(d)) => tb.calc_duration(d).map(|t| t.as_secs_f64()),
        _ => None,
    }
    .or_else(|| track.num_frames.map(|n| n as f64 / rate as f64))
    .or_else(|| {
        let info = format.media_info();
        info.time_base?.calc_duration(info.duration?).map(|t| t.as_secs_f64())
    })
    .unwrap_or(0.0);
    let decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default())?;
    let sym = Sym { format, decoder, track_id, time_base, rate, planes: Vec::new() };
    Ok(Opened { backend: Box::new(sym), rate, duration, meta, gain })
}

impl Backend for Sym {
    fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return false,
                Err(e) => {
                    crate::trace::tlog!("symphonia: read error ends the song early: {e}");
                    return false;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(e)) => {
                    crate::trace::tlog!("symphonia: bad packet skipped: {e}");
                    continue;
                }
                Err(e) => {
                    crate::trace::tlog!("symphonia: decoder error ends the song early: {e}");
                    return false;
                }
            };
            decoded.copy_to_vecs_planar::<f32>(&mut self.planes);
            match self.planes.as_slice() {
                [] => {}
                [mono] => {
                    l.extend_from_slice(mono);
                    r.extend_from_slice(mono);
                }
                [left, right, ..] => {
                    l.extend_from_slice(left);
                    r.extend_from_slice(right);
                }
            }
            return true;
        }
    }

    fn seek(&mut self, seconds: f64) -> f64 {
        let Some(time) = Time::try_from_secs_f64(seconds.max(0.0)) else { return seconds };
        let to = SeekTo::Time { time, track_id: Some(self.track_id) };
        let actual = match self.format.seek(SeekMode::Accurate, to) {
            Ok(s) => match self.time_base {
                Some(tb) => tb.calc_time(s.actual_ts).map_or(seconds, |t| t.as_secs_f64()),
                None => s.actual_ts.get() as f64 / self.rate as f64,
            },
            Err(_) => seconds,
        };
        self.decoder.reset();
        actual
    }
}

#[cfg(feature = "ffmpeg")]
mod ffmpeg {
    use super::*;
    use ffmpeg_next as ff;
    use ff::software::resampling;
    use ff::util::channel_layout::ChannelLayout;
    use ff::util::format::sample::{Sample, Type as SampleType};

    static INIT: Once = Once::new();

    pub struct Ff {
        input: ff::format::context::Input,
        decoder: ff::decoder::Audio,
        stream: usize,
        resampler: Option<(resampling::Context, Sample, u64, u32)>,
        eof: bool,
    }

    pub fn demuxers_for(ext: &str) -> Option<&'static str> {
        Some(match ext {
            "mp3" | "mp2" | "mp1" => "mp3",
            "flac" => "flac",
            "ogg" | "oga" | "opus" => "ogg",
            "m4a" | "m4b" | "mp4" | "alac" => "mov",
            "aac" => "aac,mov",
            "wav" | "wave" => "wav,w64",
            "aif" | "aiff" | "aifc" => "aiff",
            "caf" => "caf",
            "mka" | "webm" => "matroska",
            "wma" => "asf",
            "wv" => "wv",
            "ape" => "ape",
            "mpc" | "mp+" | "mpp" => "mpc,mpc8",
            "dsf" => "dsf",
            "dff" => "iff",
            "tta" => "tta",
            "ac3" => "ac3",
            _ => return None,
        })
    }

    pub fn open(path: &Path, demuxers: &str) -> Result<Opened> {
        INIT.call_once(|| {
            let _ = ff::init();
            ff::util::log::set_level(ff::util::log::Level::Quiet);
        });
        let mut options = ff::Dictionary::new();
        options.set("protocol_whitelist", "file");
        options.set("format_whitelist", demuxers);
        let input = ff::format::input_with_dictionary(path, options)?;
        let stream = input.streams().best(ff::media::Type::Audio).ok_or_else(|| anyhow!("no audio track"))?;
        let index = stream.index();
        let tb = stream.time_base();
        let ctx = ff::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = ctx.decoder().audio()?;
        let rate = decoder.rate().max(1);
        let duration = if stream.duration() > 0 {
            stream.duration() as f64 * f64::from(tb)
        } else if input.duration() > 0 {
            input.duration() as f64 / ff::ffi::AV_TIME_BASE as f64
        } else {
            0.0
        };
        let mut meta = TrackMeta::default();
        let mut gain = super::Gain::default();
        for dict in [input.metadata(), stream.metadata()] {
            for (k, v) in dict.iter() {
                match k.to_ascii_lowercase().as_str() {
                    "title" if meta.title.is_empty() => meta.title = v.to_string(),
                    "artist" if meta.artist.is_empty() => meta.artist = v.to_string(),
                    "album" if meta.album.is_empty() => meta.album = v.to_string(),
                    _ => gain.read(k, v),
                }
            }
        }
        let ff = Ff { input, decoder, stream: index, resampler: None, eof: false };
        Ok(Opened { backend: Box::new(ff), rate, duration, meta, gain })
    }

    impl Ff {
        fn convert(&mut self, frame: &mut ff::frame::Audio, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
            if frame.samples() == 0 {
                return true;
            }
            if frame.channel_layout().is_empty() || frame.channel_layout().channels() == 0 {
                frame.set_channel_layout(ChannelLayout::default(frame.channels() as i32));
            }
            let key = (frame.format(), frame.channel_layout().bits(), frame.rate());
            let stale = match &self.resampler {
                Some((_, f, b, rt)) => (*f, *b, *rt) != key,
                None => true,
            };
            if stale {
                match resampling::Context::get(
                    frame.format(),
                    frame.channel_layout(),
                    frame.rate(),
                    Sample::F32(SampleType::Planar),
                    ChannelLayout::STEREO,
                    frame.rate(),
                ) {
                    Ok(c) => self.resampler = Some((c, key.0, key.1, key.2)),
                    Err(_) => return false,
                }
            }
            let (swr, ..) = self.resampler.as_mut().unwrap();
            let mut out = ff::frame::Audio::empty();
            if swr.run(frame, &mut out).is_err() {
                return false;
            }
            let n = out.samples();
            l.extend_from_slice(&out.plane::<f32>(0)[..n]);
            r.extend_from_slice(&out.plane::<f32>(1)[..n]);
            true
        }

        fn drain_frames(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> Option<bool> {
            let mut got = false;
            let mut frame = ff::frame::Audio::empty();
            while self.decoder.receive_frame(&mut frame).is_ok() {
                if !self.convert(&mut frame, l, r) {
                    return Some(false);
                }
                got = true;
            }
            if got {
                Some(true)
            } else {
                None
            }
        }
    }

    impl Backend for Ff {
        fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
            loop {
                if let Some(ok) = self.drain_frames(l, r) {
                    return ok;
                }
                if self.eof {
                    return false;
                }
                let mut packet = ff::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) => {
                        if packet.stream() == self.stream {
                            let _ = self.decoder.send_packet(&packet);
                        }
                    }
                    Err(ff::Error::Eof) => {
                        let _ = self.decoder.send_eof();
                        self.eof = true;
                    }
                    Err(e) => {
                        crate::trace::tlog!("ffmpeg: read error ends the song early: {e}");
                        let _ = self.decoder.send_eof();
                        self.eof = true;
                    }
                }
            }
        }

        fn seek(&mut self, seconds: f64) -> f64 {
            let ts = (seconds.max(0.0) * ff::ffi::AV_TIME_BASE as f64) as i64;
            let _ = self.input.seek(ts, ..ts);
            self.decoder.flush();
            self.eof = false;
            seconds
        }
    }
}

#[cfg(feature = "tracker")]
mod openmpt {
    use super::*;

    const RATE: u32 = 48_000;

    #[repr(C)]
    struct Module {
        _p: [u8; 0],
    }

    #[link(name = "openmpt")]
    extern "C" {
        fn openmpt_is_extension_supported(ext: *const c_char) -> c_int;
        fn openmpt_module_create_from_memory2(
            data: *const c_void,
            size: usize,
            logfunc: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
            loguser: *mut c_void,
            errfunc: Option<unsafe extern "C" fn(c_int, *mut c_void) -> c_int>,
            erruser: *mut c_void,
            error: *mut c_int,
            error_message: *mut *const c_char,
            ctls: *const c_void,
        ) -> *mut Module;
        fn openmpt_module_destroy(m: *mut Module);
        fn openmpt_module_read_interleaved_float_stereo(m: *mut Module, rate: i32, count: usize, out: *mut f32) -> usize;
        fn openmpt_module_set_position_seconds(m: *mut Module, seconds: c_double) -> c_double;
        fn openmpt_module_get_duration_seconds(m: *mut Module) -> c_double;
        fn openmpt_module_get_metadata(m: *mut Module, key: *const c_char) -> *const c_char;
        fn openmpt_free_string(s: *const c_char);
        fn openmpt_log_func_silent(message: *const c_char, user: *mut c_void);
        fn openmpt_error_func_ignore(error: c_int, user: *mut c_void) -> c_int;
    }

    pub fn supports(ext: &str) -> bool {
        if ext.is_empty() {
            return false;
        }
        let Ok(c) = CString::new(ext) else { return false };
        unsafe { openmpt_is_extension_supported(c.as_ptr()) != 0 }
    }

    struct Mpt(*mut Module);

    impl Drop for Mpt {
        fn drop(&mut self) {
            unsafe { openmpt_module_destroy(self.0) }
        }
    }

    fn metadata(m: *mut Module, key: &str) -> String {
        let k = CString::new(key).unwrap();
        unsafe {
            let p = openmpt_module_get_metadata(m, k.as_ptr());
            if p.is_null() {
                return String::new();
            }
            let s = CStr::from_ptr(p).to_string_lossy().trim().to_string();
            openmpt_free_string(p);
            s
        }
    }

    pub fn open(path: &Path) -> Result<Opened> {
        let data = std::fs::read(path)?;
        let mut err: c_int = 0;
        let m = unsafe {
            openmpt_module_create_from_memory2(
                data.as_ptr() as *const c_void,
                data.len(),
                Some(openmpt_log_func_silent),
                std::ptr::null_mut(),
                Some(openmpt_error_func_ignore),
                std::ptr::null_mut(),
                &mut err,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if m.is_null() {
            return Err(anyhow!("libopenmpt can't read this module (error {err})"));
        }
        let duration = unsafe { openmpt_module_get_duration_seconds(m) };
        let meta = TrackMeta { title: metadata(m, "title"), artist: metadata(m, "artist"), ..Default::default() };
        Ok(Opened { backend: Box::new(Mpt(m)), rate: RATE, duration, meta, gain: Default::default() })
    }

    impl Backend for Mpt {
        fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
            let mut buf = vec![0f32; 1024 * 2];
            let n = unsafe { openmpt_module_read_interleaved_float_stereo(self.0, RATE as i32, 1024, buf.as_mut_ptr()) };
            for f in buf[..n * 2].chunks(2) {
                l.push(f[0]);
                r.push(f[1]);
            }
            n > 0
        }

        fn seek(&mut self, seconds: f64) -> f64 {
            unsafe { openmpt_module_set_position_seconds(self.0, seconds) }
        }
    }
}

#[cfg(feature = "chiptune")]
mod gme {
    use super::*;

    const RATE: u32 = 44_100;
    const FADE_MS: i64 = 8_000;

    #[repr(C)]
    struct Emu {
        _p: [u8; 0],
    }

    #[repr(C)]
    struct Info {
        length: c_int,
        intro_length: c_int,
        loop_length: c_int,
        play_length: c_int,
        _i: [c_int; 12],
        system: *const c_char,
        game: *const c_char,
        song: *const c_char,
        author: *const c_char,
        _rest: [*const c_char; 12],
    }

    #[link(name = "gme")]
    extern "C" {
        fn gme_open_file(path: *const c_char, out: *mut *mut Emu, rate: c_int) -> *const c_char;
        fn gme_delete(e: *mut Emu);
        fn gme_track_count(e: *const Emu) -> c_int;
        fn gme_start_track(e: *mut Emu, index: c_int) -> *const c_char;
        fn gme_play(e: *mut Emu, count: c_int, out: *mut c_short) -> *const c_char;
        fn gme_track_ended(e: *const Emu) -> c_int;
        fn gme_seek(e: *mut Emu, msec: c_int) -> *const c_char;
        fn gme_set_fade(e: *mut Emu, start_msec: c_int);
        fn gme_track_info(e: *const Emu, out: *mut *mut Info, track: c_int) -> *const c_char;
        fn gme_free_info(i: *mut Info);
    }

    fn cstr(p: *const c_char) -> String {
        if p.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(p).to_string_lossy().trim().to_string() }
        }
    }

    struct Gme {
        emu: *mut Emu,
        lengths: Vec<i64>,
        track: usize,
        done: bool,
    }

    impl Drop for Gme {
        fn drop(&mut self) {
            unsafe { gme_delete(self.emu) }
        }
    }

    impl Gme {
        fn start(&mut self, track: usize) -> bool {
            self.track = track;
            unsafe {
                if !gme_start_track(self.emu, track as c_int).is_null() {
                    return false;
                }
                gme_set_fade(self.emu, (self.lengths[track] - FADE_MS).max(0) as c_int);
            }
            true
        }
    }

    pub fn open(path: &Path) -> Result<Opened> {
        let c = CString::new(path.to_string_lossy().as_bytes())?;
        let mut emu: *mut Emu = std::ptr::null_mut();
        let err = unsafe { gme_open_file(c.as_ptr(), &mut emu, RATE as c_int) };
        if !err.is_null() || emu.is_null() {
            return Err(anyhow!("Game Music Emu: {}", cstr(err)));
        }
        let count = unsafe { gme_track_count(emu) }.max(1) as usize;
        let mut lengths = Vec::with_capacity(count);
        let mut meta = TrackMeta::default();
        for t in 0..count {
            let mut info: *mut Info = std::ptr::null_mut();
            let ok = unsafe { gme_track_info(emu, &mut info, t as c_int) }.is_null() && !info.is_null();
            let mut ms = 150_000i64;
            if ok {
                let i = unsafe { &*info };
                if i.length > 0 {
                    ms = i.length as i64;
                } else if i.play_length > 0 {
                    ms = i.play_length as i64 + FADE_MS;
                }
                if t == 0 {
                    meta.album = cstr(i.game);
                    meta.artist = cstr(i.author);
                    meta.title = if count > 1 { meta.album.clone() } else { cstr(i.song) };
                    if meta.title.is_empty() {
                        meta.title = cstr(i.song);
                    }
                }
                unsafe { gme_free_info(info) };
            }
            lengths.push(ms);
        }
        let duration = lengths.iter().sum::<i64>() as f64 / 1000.0;
        let mut g = Gme { emu, lengths, track: 0, done: false };
        if !g.start(0) {
            return Err(anyhow!("Game Music Emu can't start this file"));
        }
        Ok(Opened { backend: Box::new(g), rate: RATE, duration, meta, gain: Default::default() })
    }

    impl Backend for Gme {
        fn read(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) -> bool {
            if self.done {
                return false;
            }
            if unsafe { gme_track_ended(self.emu) } != 0 {
                if self.track + 1 >= self.lengths.len() || !self.start(self.track + 1) {
                    self.done = true;
                    return false;
                }
            }
            let mut buf = vec![0i16; 1024 * 2];
            if !unsafe { gme_play(self.emu, buf.len() as c_int, buf.as_mut_ptr()) }.is_null() {
                self.done = true;
                return false;
            }
            for f in buf.chunks(2) {
                l.push(f[0] as f32 / 32768.0);
                r.push(f[1] as f32 / 32768.0);
            }
            true
        }

        fn seek(&mut self, seconds: f64) -> f64 {
            let mut ms = (seconds.max(0.0) * 1000.0) as i64;
            let mut track = 0;
            while track + 1 < self.lengths.len() && ms >= self.lengths[track] {
                ms -= self.lengths[track];
                track += 1;
            }
            self.done = !self.start(track);
            unsafe { gme_seek(self.emu, ms as c_int) };
            seconds
        }
    }
}

#[cfg(not(feature = "tracker"))]
mod openmpt {
    pub fn supports(_ext: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bomb;

    impl Backend for Bomb {
        fn read(&mut self, l: &mut Vec<f32>, _r: &mut Vec<f32>) -> bool {
            l.push(0.5);
            panic!("crafted file");
        }

        fn seek(&mut self, _seconds: f64) -> f64 {
            panic!("crafted file");
        }
    }

    #[test]
    fn a_panicking_decoder_ends_the_song_not_the_thread() {
        let mut g = Guarded { inner: Box::new(Bomb), dead: false };
        let (mut l, mut r) = (Vec::new(), Vec::new());
        assert!(!g.read(&mut l, &mut r));
        assert!(l.is_empty() && r.is_empty(), "half a block left behind");
        assert!(!g.read(&mut l, &mut r));
        assert_eq!(g.seek(3.0), 3.0);
        assert!(guard("open", Path::new("x"), || -> Result<()> { panic!("crafted file") }).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_songs() {
        let dir = std::env::temp_dir().join(format!("urchin_decode_special_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("trap.mp3");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        std::os::unix::fs::symlink("/dev/zero", dir.join("zero.flac")).unwrap();
        assert!(open(&sock).is_err());
        assert!(probe(&dir.join("zero.flac")).is_err());
        assert!(crate::fsx::list(&dir, false).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
