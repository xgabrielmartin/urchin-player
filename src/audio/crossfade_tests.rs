use super::*;
use std::fs;

const RATE: u32 = 48_000;

fn write_level(path: &Path, value: f32, secs: f32) {
    let frames = (RATE as f32 * secs) as usize;
    let v = (value * 32767.0).round() as i16;
    let data: Vec<u8> = (0..frames * 2).flat_map(|_| v.to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for (v, n) in [(16u32, 4), (1, 2), (2, 2), (RATE, 4), (RATE * 4, 4), (4, 2), (16, 2)] {
        out.extend_from_slice(&v.to_le_bytes()[..n]);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend(data);
    fs::write(path, out).unwrap();
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("urchin_xf_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

struct Rig {
    p: Player,
    r: Renderer,
    sh: Arc<Shared>,
}

fn rig(crossfade_s: u32, songs: Vec<PathBuf>) -> Rig {
    let sh = Arc::new(Shared::new(1.0, Arc::new(Mutex::new(Store::default())), false));
    sh.crossfade_ms.store(crossfade_s * 1000, Ordering::Relaxed);
    let mut p = Player::new(sh.clone(), RATE, None);
    let r = Renderer::new(sh.clone(), p.ring.clone(), 2, RATE);
    p.start(songs, 0);
    Rig { p, r, sh }
}

impl Rig {
    fn step(&mut self) -> Option<(Vec<f32>, usize)> {
        while self.p.fill() {
            if self.p.ring.len() as u32 > RATE / 4 {
                break;
            }
        }
        self.p.publish();
        if !self.p.active {
            return None;
        }
        let index = self.p.audible().map(|m| m.index).unwrap();
        let mut buf = vec![0.0f32; 2 * RATE as usize / 100];
        self.r.fill(&mut buf);
        Some((buf.iter().step_by(2).copied().collect(), index))
    }

    fn play_out(&mut self) -> (Vec<f32>, Vec<usize>) {
        let (mut out, mut heard) = (Vec::new(), Vec::new());
        for _ in 0..6000 {
            let Some((s, i)) = self.step() else { break };
            out.extend(s);
            heard.push(i);
        }
        while out.last() == Some(&0.0) {
            out.pop();
        }
        (out, heard)
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.002
}

fn at(out: &[f32], secs: f32) -> f32 {
    out[(secs * RATE as f32) as usize]
}

#[test]
fn songs_overlap_by_the_crossfade_and_the_old_one_fades_as_the_new_one_rises() {
    let d = dir("basic");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    write_level(&a, 0.5, 3.0);
    write_level(&b, 0.25, 3.0);
    let mut rig = rig(1, vec![a, b]);
    let (out, heard) = rig.play_out();
    assert_eq!(out.len(), 5 * RATE as usize, "the two songs share one second");
    assert!(near(at(&out, 1.0), 0.5) && near(at(&out, 4.0), 0.25));
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!(near(at(&out, 2.5), 0.5 * half + 0.25 * half), "{}", at(&out, 2.5));
    let worst = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(worst < 0.001, "a step of {worst}");
    let switch = heard.iter().position(|&i| i == 1).unwrap();
    assert_eq!(switch, 200, "song 2 shows as playing from 2.0 s");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_long_crossfade_does_not_hold_up_the_start_of_a_song() {
    let d = dir("start");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    write_level(&a, 0.5, 20.0);
    write_level(&b, 0.25, 3.0);
    let mut rig = rig(MAX_CROSSFADE, vec![a, b]);
    let (first, _) = rig.step().unwrap();
    assert!(first.iter().all(|&s| near(s, 0.5)), "sound from the very first buffer");
    assert!(rig.p.hold.is_empty() && rig.p.produced < 2 * RATE as u64, "nothing held back, little decoded ahead");
    let (out, _) = rig.play_out();
    assert_eq!(out.len() + RATE as usize / 100, 20 * RATE as usize);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn with_crossfade_off_songs_still_join_without_a_gap() {
    let d = dir("off");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    write_level(&a, 0.5, 1.0);
    write_level(&b, 0.25, 1.0);
    let mut rig = rig(0, vec![a, b]);
    let (out, _) = rig.play_out();
    assert_eq!(out.len(), 2 * RATE as usize);
    assert!(out[..RATE as usize].iter().all(|&s| near(s, 0.5)));
    assert!(out[RATE as usize..].iter().all(|&s| near(s, 0.25)));
    assert!(rig.p.hold.is_empty() && rig.p.fade.is_empty());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_last_song_in_the_queue_ends_in_full_without_fading() {
    let d = dir("last");
    let a = d.join("a.wav");
    write_level(&a, 0.5, 2.0);
    let mut rig = rig(4, vec![a]);
    let (out, _) = rig.play_out();
    assert_eq!(out.len(), 2 * RATE as usize, "a song shorter than the crossfade, alone, plays whole");
    assert!(out.iter().all(|&s| near(s, 0.5)));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_song_shorter_than_the_crossfade_is_faded_through_without_losing_time() {
    let d = dir("short");
    let songs: Vec<PathBuf> = [("a", 0.5, 3.0), ("b", 0.3, 0.5), ("c", 0.1, 3.0)]
        .into_iter()
        .map(|(n, v, s)| {
            let p = d.join(format!("{n}.wav"));
            write_level(&p, v, s);
            p
        })
        .collect();
    let mut rig = rig(1, songs);
    let (out, heard) = rig.play_out();
    assert_eq!(out.len(), 5 * RATE as usize);
    assert_eq!(heard.last(), Some(&2));
    assert!(near(at(&out, 4.5), 0.1));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn changing_the_song_lined_up_during_a_crossfade_fades_into_the_new_one() {
    let d = dir("reselect");
    let songs: Vec<PathBuf> = [("a", 0.5, 3.0), ("b", 0.3, 3.0), ("c", 0.1, 3.0)]
        .into_iter()
        .map(|(n, v, s)| {
            let p = d.join(format!("{n}.wav"));
            write_level(&p, v, s);
            p
        })
        .collect();
    let mut rig = rig(1, songs.clone());
    let mut out = Vec::new();
    for _ in 0..190 {
        out.extend(rig.step().unwrap().0);
    }
    assert!(rig.p.last_fade.is_some(), "a's end should be fading into b by now");
    rig.p.remove(1, &songs[1]);
    let (rest, heard) = rig.play_out();
    out.extend(rest);
    assert_eq!(out.len(), 5 * RATE as usize);
    assert!(!heard.contains(&1) || rig.p.queue.len() == 2);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!(near(at(&out, 2.5), 0.5 * half + 0.1 * half), "a fades into c: {}", at(&out, 2.5));
    assert!(near(at(&out, 4.0), 0.1));
    assert!(out.iter().all(|&s| s < 0.55 && s > 0.05), "never b's level on its own, never a gap");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn seeking_or_skipping_mid_fade_starts_clean() {
    let d = dir("seek");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    write_level(&a, 0.5, 3.0);
    write_level(&b, 0.25, 3.0);
    let mut rig = rig(1, vec![a, b]);
    for _ in 0..230 {
        rig.step();
    }
    assert_eq!(rig.p.audible().unwrap().index, 1, "mid-fade, song 2 is the one playing");
    rig.p.seek(1.0);
    let (s, i) = rig.step().unwrap();
    assert_eq!(i, 1);
    assert!(s.iter().all(|&v| near(v, 0.25)), "after a seek only song 2 plays");
    let (out, _) = rig.play_out();
    let rest = out.len() + RATE as usize / 100;
    assert!(rest.abs_diff(2 * RATE as usize) <= 1024, "{rest} frames");
    assert!(out.iter().all(|&v| near(v, 0.25)));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn replaygain_sets_each_songs_level_from_its_tags() {
    use crate::decode::Gain;
    let g = Gain { track: Some(-6.0206), track_peak: None, album: Some(-12.0412), album_peak: None };
    assert!((ReplayGain::Off.factor(&g) - 1.0).abs() < 1e-6);
    assert!((ReplayGain::Track.factor(&g) - 0.5).abs() < 1e-3);
    assert!((ReplayGain::Album.factor(&g) - 0.25).abs() < 1e-3);
    let only_track = Gain { track: Some(-6.0206), ..Gain::default() };
    assert!((ReplayGain::Album.factor(&only_track) - 0.5).abs() < 1e-3);
    assert_eq!(ReplayGain::Track.factor(&Gain::default()), 1.0);
    let loud = Gain { track: Some(6.0206), track_peak: Some(0.8), ..Gain::default() };
    assert!((ReplayGain::Track.factor(&loud) - 1.25).abs() < 1e-3);

    let d = dir("rg");
    let a = d.join("a.wav");
    write_level(&a, 0.5, 3.0);
    let mut rig = rig(0, vec![a]);
    rig.sh.replaygain.store(ReplayGain::Track.code(), Ordering::Relaxed);
    rig.p.source.as_mut().unwrap().gain = g;
    let mut out = Vec::new();
    for _ in 0..100 {
        out.extend(rig.step().unwrap().0);
    }
    rig.sh.replaygain.store(ReplayGain::Album.code(), Ordering::Relaxed);
    let (rest, _) = rig.play_out();
    out.extend(rest);
    assert!(near(at(&out, 0.5), 0.25), "track gain halves it: {}", at(&out, 0.5));
    assert!(near(at(&out, 2.9), 0.125), "album gain quarters it: {}", at(&out, 2.9));
    let worst = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(worst < 0.001, "a step of {worst}");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn replaygain_tags_are_read_under_every_name_they_go_by() {
    use crate::decode::Gain;
    let mut g = Gain::default();
    g.read("REPLAYGAIN_TRACK_GAIN", "-7.89 dB");
    g.read("com.apple.iTunes:replaygain_album_gain", "+1.5 dB");
    g.read("TXXX:REPLAYGAIN_TRACK_PEAK", "0.988");
    g.read("replaygain_album_peak", "nonsense");
    assert_eq!(g, Gain { track: Some(-7.89), track_peak: Some(0.988), album: Some(1.5), album_peak: None });
    let mut opus = Gain::default();
    opus.read("R128_TRACK_GAIN", "-512");
    assert!((opus.track.unwrap() - 3.0).abs() < 1e-6);

    let d = dir("tags");
    let path = d.join("t.mp3");
    let txxx = |desc: &str, value: &str| {
        let mut body = vec![0u8];
        body.extend_from_slice(desc.as_bytes());
        body.push(0);
        body.extend_from_slice(value.as_bytes());
        let mut f = b"TXXX".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend(body);
        f
    };
    let mut tag = txxx("REPLAYGAIN_TRACK_GAIN", "-4.20 dB");
    tag.extend(txxx("REPLAYGAIN_ALBUM_GAIN", "-5.10 dB"));
    tag.extend(txxx("REPLAYGAIN_TRACK_PEAK", "0.950000"));
    let n = tag.len();
    let mut file = b"ID3\x03\x00\x00".to_vec();
    file.extend_from_slice(&[(n >> 21 & 0x7f) as u8, (n >> 14 & 0x7f) as u8, (n >> 7 & 0x7f) as u8, (n & 0x7f) as u8]);
    file.extend(tag);
    for _ in 0..40 {
        file.extend_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        file.resize(file.len() + 413, 0u8);
    }
    fs::write(&path, file).unwrap();
    let gain = crate::decode::open(&path).unwrap().gain;
    assert_eq!(gain.track, Some(-4.2));
    assert_eq!(gain.album, Some(-5.1));
    assert_eq!(gain.track_peak, Some(0.95));
    let _ = fs::remove_dir_all(&d);
}
