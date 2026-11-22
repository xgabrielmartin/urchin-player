use std::path::Path;

pub const VERSION: u32 = 2;

const SECONDS: f64 = 2.0;
const MAX_SILENCE_SECONDS: f64 = 30.0;
const SILENT: i16 = 16;

pub fn of(path: &Path) -> Option<String> {
    let mut opened = crate::decode::open(path).ok()?;
    let rate = opened.rate.max(1);
    let want = (SECONDS * rate as f64) as usize;
    let give_up = (MAX_SILENCE_SECONDS * rate as f64) as usize;
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let mut hash = Fnv128::new();
    hash.write(&rate.to_le_bytes());
    let (mut seen, mut taken, mut started) = (0usize, 0usize, false);
    loop {
        l.clear();
        r.clear();
        let more = opened.backend.read(&mut l, &mut r);
        for (a, b) in l.iter().zip(&r) {
            let s = (((a + b) * 0.5).clamp(-1.0, 1.0) * 32767.0).round() as i16;
            seen += 1;
            if !started {
                if s.unsigned_abs() < SILENT as u16 {
                    continue;
                }
                started = true;
            }
            hash.write(&s.to_le_bytes());
            taken += 1;
            if taken >= want {
                return Some(hash.hex());
            }
        }
        if !more || (!started && seen >= give_up) {
            break;
        }
    }
    (taken > 0).then(|| hash.hex())
}

struct Fnv128(u128);

impl Fnv128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;

    fn new() -> Fnv128 {
        Fnv128(Self::OFFSET)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u128;
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn hex(&self) -> String {
        format!("{:032x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn wav(path: &Path, rate: u32, silence: f64, seconds: f64, freq: f64) {
        let mut data = Vec::new();
        let quiet = (silence * rate as f64) as usize;
        let n = (seconds * rate as f64) as usize;
        for i in 0..quiet + n {
            let v = if i < quiet { 0 } else { ((i as f64 / rate as f64 * freq * std::f64::consts::TAU).sin() * 9000.0) as i16 };
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        for v in [16u32.to_le_bytes(), [1, 0, 2, 0], rate.to_le_bytes(), (rate * 4).to_le_bytes(), [4, 0, 16, 0]] {
            w.extend_from_slice(&v);
        }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, w).unwrap();
    }

    #[test]
    fn the_same_sound_anywhere_is_the_same_fingerprint_and_other_sound_is_not() {
        let dir = std::env::temp_dir().join(format!("urchin_print_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = dir.join("one/a.wav");
        wav(&a, 44_100, 0.0, 3.0, 440.0);
        let print = of(&a).unwrap();
        assert_eq!(print.len(), 32);

        let moved = dir.join("two/deeper/renamed.wav");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&a, &moved).unwrap();
        assert_eq!(of(&moved).unwrap(), print);

        let b = dir.join("b.wav");
        wav(&b, 44_100, 0.0, 3.0, 445.0);
        assert_ne!(of(&b).unwrap(), print);
        let c = dir.join("c.wav");
        wav(&c, 48_000, 0.0, 3.0, 440.0);
        assert_ne!(of(&c).unwrap(), print);

        let d = dir.join("d.wav");
        let e = dir.join("e.wav");
        wav(&d, 44_100, 2.5, 3.0, 440.0);
        wav(&e, 44_100, 2.5, 3.0, 660.0);
        assert_ne!(of(&d).unwrap(), of(&e).unwrap());

        let short = dir.join("short.wav");
        wav(&short, 44_100, 0.0, 0.5, 440.0);
        assert!(of(&short).is_some());
        let silent = dir.join("silent.wav");
        wav(&silent, 44_100, 3.0, 0.0, 440.0);
        assert_eq!(of(&silent), None);
        let junk = dir.join("junk.wav");
        fs::write(&junk, b"not audio").unwrap();
        assert_eq!(of(&junk), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hash_never_changes() {
        let hash = |bytes: &[u8]| {
            let mut h = Fnv128::new();
            h.write(bytes);
            h.hex()
        };
        assert_eq!(hash(b"a"), "d228cb696f1a8caf78912b704e4a8964");
        assert_eq!(hash(b"urchin"), "27ce57c3323c64bf6f3a8f607b31f566");
    }
}
