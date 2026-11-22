use super::*;
use std::fs;

fn shared(volume: f32, eq: bool) -> Arc<Shared> {
    Arc::new(Shared::new(volume, Arc::new(Mutex::new(Store::default())), eq))
}

fn constant(frames: usize, l: f32, r: f32) -> Vec<f32> {
    (0..frames).flat_map(|_| [l, r]).collect()
}

fn play(r: &mut Renderer, frames: usize, block: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut left = frames;
    while left > 0 {
        let n = left.min(block);
        let mut buf = vec![0.0f32; n * 2];
        r.fill(&mut buf);
        out.extend(buf);
        left -= n;
    }
    out
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("urchin_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_callback_counts_the_frames_it_plays_and_goes_quiet_when_dry_or_paused() {
    let sh = shared(1.0, false);
    let ring = Arc::new(Ring::new(1024));
    let mut r = Renderer::new(sh.clone(), ring.clone(), 2, 48_000);
    ring.push(&constant(100, 0.5, -0.25));
    let out = play(&mut r, 160, 64);
    assert_eq!(sh.consumed.load(Ordering::Relaxed), 100);
    assert!(out[..200].chunks(2).all(|f| f == [0.5, -0.25]));
    assert!(out[200..].iter().all(|s| *s == 0.0), "a dry ring plays silence");
    assert!(ring.is_empty());

    ring.push(&constant(50, 0.5, 0.5));
    sh.paused.store(true, Ordering::Relaxed);
    let out = play(&mut r, 64, 64);
    assert!(out.iter().all(|s| *s == 0.0));
    assert_eq!(ring.len(), 50);
    assert_eq!(sh.consumed.load(Ordering::Relaxed), 100);
}

#[test]
fn the_callback_counts_silence_from_a_dry_ring_but_not_while_paused() {
    let sh = shared(1.0, false);
    let ring = Arc::new(Ring::new(1024));
    let mut r = Renderer::new(sh.clone(), ring.clone(), 2, 48_000);
    ring.push(&constant(100, 0.5, 0.5));
    play(&mut r, 160, 64);
    assert_eq!(sh.starved.load(Ordering::Relaxed), 60, "60 frames had nothing to play");
    assert_eq!(sh.gate_busy.load(Ordering::Relaxed), 0);

    sh.paused.store(true, Ordering::Relaxed);
    play(&mut r, 128, 64);
    assert_eq!(sh.starved.load(Ordering::Relaxed), 60, "paused silence is not an underrun");

    sh.paused.store(false, Ordering::Relaxed);
    ring.exclusive(|_| play(&mut r, 64, 64));
    assert_eq!(sh.gate_busy.load(Ordering::Relaxed), 1);
}

#[test]
fn a_songs_eq_starts_on_its_first_frame_not_at_the_next_buffer() {
    let sh = shared(1.0, true);
    let ring = Arc::new(Ring::new(1024));
    let quieter = Filter::compile(&Curve { pre: -6.0, ..Curve::default() }, 48_000);
    {
        let mut eq = sh.eq.lock().unwrap();
        for (start, filter) in [(0, Filter::flat()), (100, quieter)] {
            eq.slots.push_back(EqSlot { start, song: PathBuf::new(), curve: Curve::default(), filter });
        }
        eq.version += 1;
    }
    let mut r = Renderer::new(sh, ring.clone(), 2, 48_000);
    ring.push(&constant(200, 0.5, 0.5));
    let out = play(&mut r, 200, 64);
    assert!(out[..200].iter().all(|s| (*s - 0.5).abs() < 1e-6), "{:?}", &out[190..200]);
    let quiet = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!(out[200..].iter().all(|s| (*s - quiet).abs() < 1e-6), "{:?}", &out[196..206]);
}

#[test]
fn changing_the_volume_eases_instead_of_stepping() {
    let sh = shared(1.0, false);
    let ring = Arc::new(Ring::new(16_384));
    let mut r = Renderer::new(sh.clone(), ring.clone(), 2, 48_000);
    ring.push(&constant(12_000, 1.0, 1.0));
    play(&mut r, 100, 100);
    sh.volume.store(0.0f32.to_bits(), Ordering::Relaxed);
    let out = play(&mut r, 6_000, 256);
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    assert!(left[0] > 0.97, "no jump at the start: {}", left[0]);
    assert!(left.windows(2).all(|w| w[1] <= w[0]), "it falls steadily");
    assert!(left.windows(2).all(|w| w[0] - w[1] < 0.01), "in small steps");
    assert!(left[5_999] < 0.001, "and gets there: {}", left[5_999]);
}

#[test]
fn mono_devices_get_the_mix_and_extra_channels_stay_silent() {
    let sh = shared(1.0, false);
    let ring = Arc::new(Ring::new(256));
    ring.push(&constant(4, 1.0, 0.0));
    let mut mono = Renderer::new(sh.clone(), ring.clone(), 1, 48_000);
    let mut out = [0.0f32; 4];
    mono.fill(&mut out);
    assert_eq!(out, [0.5; 4]);

    ring.push(&constant(4, 1.0, 0.5));
    let mut surround = Renderer::new(sh, ring, 4, 48_000);
    let mut out = [9.0f32; 16];
    surround.fill(&mut out);
    for frame in out.chunks(4) {
        assert_eq!(frame, [1.0, 0.5, 0.0, 0.0]);
    }
}

fn split_tone(dir: &Path, rate: u32, lengths: &[usize]) -> Vec<PathBuf> {
    fs::create_dir_all(dir).unwrap();
    let mut at = 0usize;
    let mut paths = Vec::new();
    for (n, &len) in lengths.iter().enumerate() {
        let mut data = Vec::with_capacity(len * 4);
        for i in at..at + len {
            let v = ((i as f64 / rate as f64 * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        at += len;
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
        let path = dir.join(format!("{n}.wav"));
        fs::write(&path, out).unwrap();
        paths.push(path);
    }
    paths
}

fn play_through(songs: Vec<PathBuf>, out_rate: u32) -> Vec<f32> {
    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), out_rate, None);
    let mut r = Renderer::new(sh.clone(), p.ring.clone(), 2, out_rate);
    p.start(songs, 0);
    let mut got = Vec::new();
    for _ in 0..10_000 {
        while p.fill() {}
        p.publish();
        if !p.active && p.ring.is_empty() {
            break;
        }
        let before = sh.consumed.load(Ordering::Relaxed);
        let mut buf = vec![0.0f32; 2 * 512];
        r.fill(&mut buf);
        let n = (sh.consumed.load(Ordering::Relaxed) - before) as usize;
        got.extend(buf[..n * 2].chunks(2).map(|f| f[0]));
    }
    got
}

#[test]
fn resampled_songs_play_back_to_back_with_no_gap_or_delay() {
    let dir = temp_dir("gapless_resampled");
    let lengths = [13_337, 9_001, 20_011];
    let songs = split_tone(&dir, 44_100, &lengths);

    for (song, &len) in songs.iter().zip(&lengths) {
        let mut src = Source::open(song, 48_000).unwrap();
        let mut frames = 0;
        while let Some(f) = src.next_frames() {
            frames += f.len() / 2;
        }
        let want = (len as f64 * 48_000.0 / 44_100.0).round() as usize;
        assert!(frames.abs_diff(want) <= 1, "{frames} frames out, wanted {want}");
    }

    let got = play_through(songs, 48_000);
    let total: usize = lengths.iter().sum();
    let want = (total as f64 * 48_000.0 / 44_100.0).round() as usize;
    assert!(got.len().abs_diff(want) <= 3, "{} frames heard, wanted {want}", got.len());
    let amp = 8000.0 / 32768.0;
    let ideal = |k: usize| (amp * (k as f64 / 48_000.0 * 440.0 * std::f64::consts::TAU).sin()) as f32;
    let edge = 64;
    let (worst, at) = (edge..got.len() - edge)
        .map(|k| ((got[k] - ideal(k)).abs(), k))
        .fold((0.0f32, 0), |a, b| if b.0 > a.0 { b } else { a });
    let joins: Vec<usize> = lengths
        .iter()
        .scan(0, |a, &l| {
            *a += l;
            Some((*a as f64 * 48_000.0 / 44_100.0) as usize)
        })
        .collect();
    assert!(worst < 0.002, "off by {worst} at frame {at} (songs join at {joins:?})");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn resample_report() {
    use std::fmt::Write as _;
    let out = std::env::var("URCHIN_RESAMPLE_REPORT").expect("set URCHIN_RESAMPLE_REPORT");
    let dump = std::env::var("URCHIN_RESAMPLE_DUMP").ok();
    let dir = temp_dir("resample_report");
    let mut report = String::new();
    let amp = 8000.0 / 32768.0;
    let hash = |v: &[f32]| v.iter().fold(0xcbf29ce484222325u64, |h, s| {
        s.to_bits().to_le_bytes().iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
    });
    for (from, to) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000), (22_050, 48_000), (88_200, 44_100), (32_000, 48_000)] {
        let lengths: Vec<usize> = [13_337usize, 9_001, 20_011].iter().map(|l| l * from as usize / 44_100).collect();
        let songs = split_tone(&dir.join(format!("{from}_{to}")), from, &lengths);
        let got = play_through(songs, to);
        let ideal = |k: usize| (amp * (k as f64 / to as f64 * 440.0 * std::f64::consts::TAU).sin()) as f32;
        let mut joins = Vec::new();
        let mut fed = 0usize;
        for l in &lengths[..2] {
            fed += l;
            let j = (fed as u64 * to as u64 + from as u64 / 2) / from as u64;
            let j = j as usize;
            let worst = (j.saturating_sub(64)..(j + 64).min(got.len())).map(|k| (got[k] - ideal(k)).abs()).fold(0.0f32, f32::max);
            joins.push(format!("{:.1}", 20.0 * (worst.max(1e-9) as f64 / amp).log10()));
        }
        let worst = (64..got.len().saturating_sub(64)).map(|k| (got[k] - ideal(k)).abs()).fold(0.0f32, f32::max);
        let _ = writeln!(
            report,
            "{from} -> {to}: {} frames, worst anywhere {:.1} dB, joins {:?} dB, samples {:016x}",
            got.len(),
            20.0 * (worst.max(1e-9) as f64 / amp).log10(),
            joins,
            hash(&got)
        );
        if let Some(d) = &dump {
            let bytes: Vec<u8> = got.iter().flat_map(|s| s.to_le_bytes()).collect();
            fs::write(Path::new(d).join(format!("tone_{from}_{to}.f32")), bytes).unwrap();
        }
    }
    let a = split_tone(&dir.join("mix_a"), 44_100, &[13_337]).remove(0);
    let b = split_tone(&dir.join("mix_b"), 48_000, &[9_001]).remove(0);
    let c = split_tone(&dir.join("mix_c"), 44_100, &[10_007]).remove(0);
    let got = play_through(vec![a, b, c], 48_000);
    let lens = [13_337usize * 48_000 / 44_100, 9_001];
    let mut at = 0;
    let mut joins = Vec::new();
    for (n, len) in lens.iter().enumerate() {
        at += len;
        let start_of_next = at;
        let ideal_next = |k: usize| (amp * ((k - start_of_next) as f64 / 48_000.0 * 440.0 * std::f64::consts::TAU).sin()) as f32;
        let worst = (at..(at + 64).min(got.len())).map(|k| (got[k] - ideal_next(k)).abs()).fold(0.0f32, f32::max);
        joins.push(format!("song {} starts {:.1} dB off", n + 1, 20.0 * (worst.max(1e-9) as f64 / amp).log10()));
    }
    let _ = writeln!(report, "44100/48000/44100 -> 48000: {} frames, {joins:?}, samples {:016x}", got.len(), hash(&got));
    let song = dir.join("minute.wav");
    crate::tests::write_wav(&song, 44_100, 60.0);
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let mut src = Source::open(&song, 48_000).unwrap();
        let mut n = 0;
        while let Some(f) = src.next_frames() {
            n += f.len();
        }
        assert!(n > 0);
        best = best.min(t.elapsed().as_secs_f64());
    }
    let _ = writeln!(report, "a minute of stereo, decode + resample 44100 -> 48000: {:.1} ms (best of 5)", best * 1000.0);
    fs::write(&out, &report).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_next_song_and_its_eq_start_exactly_at_the_join() {
    let dir = temp_dir("gapless_markers");
    let lengths = [13_337, 9_001, 20_011];
    let songs = split_tone(&dir, 44_100, &lengths);
    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 48_000, None);
    p.ring = Arc::new(Ring::new(1 << 17));
    p.start(songs, 0);
    while p.fill() {}
    let starts: Vec<u64> = p.markers.iter().map(|m| m.start).collect();
    let eq: Vec<u64> = sh.eq.lock().unwrap().slots.iter().map(|s| s.start).collect();
    let mut want = vec![0u64];
    let mut fed = 0;
    for len in &lengths[..2] {
        fed += *len as u64;
        want.push((fed * 48_000 + 22_050) / 44_100);
    }
    assert_eq!(starts, want);
    assert_eq!(eq, want);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_change_of_rate_between_songs_leaves_no_gap_either() {
    let dir = temp_dir("gapless_mixed");
    let a = split_tone(&dir.join("a"), 44_100, &[13_337]).remove(0);
    let b = split_tone(&dir.join("b"), 48_000, &[9_001]).remove(0);
    let c = split_tone(&dir.join("c"), 44_100, &[10_007]).remove(0);
    let got = play_through(vec![a, b, c], 48_000);
    let want = [13_337usize * 48_000 / 44_100, 9_001, 10_007 * 48_000 / 44_100];
    let total: usize = want.iter().sum();
    assert!(got.len().abs_diff(total) <= 3, "{} frames heard, wanted {total}", got.len());
    let amp = 8000.0 / 32768.0;
    let mut at = 0;
    for (n, len) in want.iter().enumerate() {
        let ideal = |k: usize| (amp * (k as f64 / 48_000.0 * 440.0 * std::f64::consts::TAU).sin()) as f32;
        let worst = (at + 600..at + len - 600).map(|k| (got[k] - ideal(k - at)).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.002, "song {n} off by {worst}");
        let quiet = (at.saturating_sub(64)..(at + 64).min(got.len()))
            .collect::<Vec<_>>()
            .windows(40)
            .any(|w| w.iter().all(|&k| got[k].abs() < 1e-4));
        assert!(!quiet, "a gap at the start of song {n}");
        at += len;
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audio_the_ring_has_no_room_for_waits_its_turn_and_is_not_lost() {
    let dir = temp_dir("staged");
    let song = dir.join("a.wav");
    crate::tests::write_wav(&song, 44_100, 0.5);
    let mut expected = Vec::new();
    let mut src = Source::open(&song, 48_000).unwrap();
    while let Some(f) = src.next_frames() {
        expected.extend(f);
    }

    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 48_000, None);
    p.ring = Arc::new(Ring::new(1000));
    let mut r = Renderer::new(sh.clone(), p.ring.clone(), 2, 48_000);
    p.start(vec![song], 0);
    let mut got = Vec::new();
    let mut waited = false;
    for _ in 0..2000 {
        while p.fill() {}
        waited |= !p.staged.is_empty();
        p.publish();
        if !p.active {
            break;
        }
        let before = sh.consumed.load(Ordering::Relaxed);
        let mut buf = vec![0.0f32; 2 * 300];
        r.fill(&mut buf);
        let n = (sh.consumed.load(Ordering::Relaxed) - before) as usize;
        got.extend_from_slice(&buf[..n * 2]);
    }
    assert!(waited, "the test never made the ring overflow");
    assert_eq!(got.len(), expected.len());
    assert!(got == expected, "audio was lost, repeated or reordered at the ring's edge");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn changing_repeat_cuts_back_audio_still_waiting_for_room() {
    let dir = temp_dir("staged_repeat");
    let songs: Vec<PathBuf> = (0..3)
        .map(|i| {
            let s = dir.join(format!("{i}.wav"));
            crate::tests::write_wav(&s, 48_000, 0.25);
            s
        })
        .collect();
    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 48_000, None);
    p.ring = Arc::new(Ring::new(2000));
    let mut r = Renderer::new(sh, p.ring.clone(), 2, 48_000);
    p.start(songs, 0);

    let mut reached = false;
    for _ in 0..200 {
        while p.fill() {}
        p.publish();
        if p.markers.len() >= 2 && !p.staged.is_empty() {
            reached = true;
            break;
        }
        let mut buf = vec![0.0f32; 2 * 480];
        r.fill(&mut buf);
    }
    assert!(reached, "never got a song lined up behind the audible one");

    p.set_repeat(Repeat::One);
    assert!(p.staged.is_empty(), "the second song's audio is still waiting to go in");

    let mut heard = Vec::new();
    let mut current = None;
    for _ in 0..80 {
        while p.fill() {}
        p.publish();
        if !p.active {
            break;
        }
        if let Some(m) = p.audible().filter(|m| current != Some(m.start)) {
            current = Some(m.start);
            heard.push(m.index);
        }
        let mut buf = vec![0.0f32; 2 * 480];
        r.fill(&mut buf);
    }
    assert!(!heard.contains(&1) && !heard.contains(&2), "{heard:?}");
    assert!(heard.iter().filter(|&&i| i == 0).count() >= 2, "{heard:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_queue_brought_back_is_held_where_it_was_and_carries_on_from_there() {
    let dir = temp_dir("held");
    let songs: Vec<PathBuf> = (0..3).map(|i| dir.join(format!("{i}.wav"))).collect();
    for s in &songs {
        crate::tests::write_wav(s, 44_100, 3.0);
    }
    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 48_000, None);
    handle_offline(&mut p, Cmd::Load(songs.clone(), Some((1, 1.5))));
    p.publish();
    let st = sh.status.lock().unwrap().clone();
    assert_eq!((st.state, st.index, st.position), (State::Paused, 1, 1.5));
    assert!(!st.output_lost, "not a lost output: the last run's place");
    assert_eq!(st.meta.title, "1");
    assert!((st.duration - 3.0).abs() < 0.01, "{}", st.duration);
    assert!(!p.fill(), "nothing is decoded until play is pressed");

    handle_offline(&mut p, Cmd::SeekBy(-0.5));
    handle_offline(&mut p, Cmd::Next);
    handle_offline(&mut p, Cmd::Seek(2.0));
    p.publish();
    let st = sh.status.lock().unwrap().clone();
    assert_eq!((st.index, st.position, st.meta.title.as_str()), (2, 2.0, "2"));
    handle_offline(&mut p, Cmd::Previous);
    handle_offline(&mut p, Cmd::Previous);
    p.publish();
    assert_eq!(sh.status.lock().unwrap().index, 0);
    handle_offline(&mut p, Cmd::Seek(1.0));
    handle_offline(&mut p, Cmd::Next);
    handle_offline(&mut p, Cmd::Seek(1.5));

    let ring = new_ring(48_000);
    let mut r = Renderer::new(sh.clone(), ring.clone(), 2, 48_000);
    p.attach(ring, 48_000, "speakers".into());
    assert!(p.active && p.lost.is_none());
    p.publish();
    let st = sh.status.lock().unwrap().clone();
    assert_eq!((st.state, st.index), (State::Playing, 1));
    assert!((p.position().unwrap() - 1.5).abs() < 0.03, "{:?}", p.position());
    for _ in 0..25 {
        while p.fill() {}
        p.publish();
        let mut buf = vec![0.0f32; 2 * 960];
        r.fill(&mut buf);
    }
    assert!((p.position().unwrap() - 2.0).abs() < 0.04, "{:?}", p.position());

    let mut q = Player::new(sh.clone(), 48_000, None);
    handle(&mut q, Cmd::Load(songs.clone(), Some((2, 0.5))));
    assert!(q.lost.is_some() && !q.active);
    q.resume_held();
    assert!(q.active && (q.position().unwrap() - 0.5).abs() < 0.03);

    let mut z = Player::new(sh, 48_000, None);
    handle_offline(&mut z, Cmd::Load(songs, Some((7, 1.0))));
    assert!(z.lost.is_none() && z.queue.len() == 3);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_lost_output_stops_the_player_where_it_was_and_the_next_one_resumes_there() {
    let dir = temp_dir("lost");
    let song = dir.join("a.wav");
    crate::tests::write_wav(&song, 44_100, 3.0);
    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 48_000, None);
    let mut r = Renderer::new(sh.clone(), p.ring.clone(), 2, 48_000);
    p.start(vec![song], 0);
    for _ in 0..50 {
        while p.fill() {}
        p.publish();
        let mut buf = vec![0.0f32; 2 * 480];
        r.fill(&mut buf);
    }
    let at = p.position().unwrap();
    assert!((at - 0.5).abs() < 0.03, "{at}");

    p.detach();
    assert!(!p.active && p.lost.is_some());
    assert!(!p.fill(), "nothing is decoded with nowhere to play it");
    p.publish();
    let st = sh.status.lock().unwrap().clone();
    assert!(st.state == State::Paused && st.output_lost, "held, and said to be a lost output: {st:?}");

    let ring = new_ring(96_000);
    let mut r = Renderer::new(sh.clone(), ring.clone(), 2, 96_000);
    p.attach(ring, 96_000, "new".into());
    assert!(p.active && p.lost.is_none() && p.error.is_none());
    p.publish();
    assert!(!sh.status.lock().unwrap().output_lost);
    let resumed = p.position().unwrap();
    assert!((resumed - at).abs() < 0.03, "{at} -> {resumed}");
    for _ in 0..50 {
        while p.fill() {}
        p.publish();
        let mut buf = vec![0.0f32; 2 * 960];
        r.fill(&mut buf);
    }
    let later = p.position().unwrap();
    assert!((later - (at + 0.5)).abs() < 0.04, "{at} -> {later}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_ring_with_little_room_over_a_second_plays_every_frame_once() {
    for rate in [8_000u32, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000, 96_000, 192_000] {
        let cap = new_ring(rate).capacity();
        assert!(cap >= rate as usize && cap < rate as usize * 2, "{rate} Hz: {cap} frames");
    }

    let dir = temp_dir("tight_ring");
    let song = dir.join("t.wav");
    crate::tests::write_wav(&song, 44_100, 2.0);
    let mut src = test_open(&song, 32_000).unwrap();
    let mut want = 0usize;
    while let Some(block) = src.next() {
        want += block.len() / 2;
    }

    let sh = shared(1.0, false);
    let mut p = Player::new(sh.clone(), 32_000, None);
    assert_eq!(p.ring.capacity(), 32_768);
    let mut r = Renderer::new(sh.clone(), p.ring.clone(), 2, 32_000);
    p.start(vec![song], 0);
    let ahead = 32_000;
    assert_eq!(p.ring.push(&constant(ahead, 0.0, 0.0)), ahead);
    let mut waited = false;
    for _ in 0..500 {
        while p.fill() {}
        waited |= p.staged_pos < p.staged.len();
        p.publish();
        let mut buf = vec![0.0f32; 2 * 320];
        r.fill(&mut buf);
    }
    assert!(waited, "the test should have made a block wait for room");
    assert_eq!(p.produced as usize, want, "everything was decoded once");
    assert_eq!(sh.consumed.load(Ordering::Relaxed) as usize, ahead + want, "and every frame of it was played");
    assert!(p.staged_pos >= p.staged.len(), "nothing is left waiting");
    let _ = fs::remove_dir_all(&dir);
}
