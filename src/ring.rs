use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

pub struct Ring {
    buf: Box<[AtomicU32]>,
    mask: u64,
    head: AtomicU64,
    tail: AtomicU64,
    gate: Mutex<()>,
}

impl Ring {
    pub fn new(min_frames: usize) -> Ring {
        let frames = min_frames.max(2).next_power_of_two();
        Ring {
            buf: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(),
            mask: frames as u64 - 1,
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            gate: Mutex::new(()),
        }
    }

    pub fn capacity(&self) -> usize {
        self.mask as usize + 1
    }

    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.saturating_sub(tail) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push(&self, samples: &[f32]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let free = self.capacity() - head.saturating_sub(tail) as usize;
        let frames = (samples.len() / 2).min(free);
        for (i, pair) in samples[..frames * 2].chunks(2).enumerate() {
            let slot = (((head + i as u64) & self.mask) * 2) as usize;
            self.buf[slot].store(pair[0].to_bits(), Ordering::Relaxed);
            self.buf[slot + 1].store(pair[1].to_bits(), Ordering::Relaxed);
        }
        self.head.store(head + frames as u64, Ordering::Release);
        frames
    }

    pub fn reader(&self) -> Option<Reader<'_>> {
        let gate = match self.gate.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        Some(Reader { ring: self, tail, head, _gate: gate })
    }

    pub fn exclusive<R>(&self, f: impl FnOnce(&mut Exclusive<'_>) -> R) -> R {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut Exclusive { ring: self })
    }

    pub fn clear(&self) {
        self.exclusive(|x| x.clear());
    }
}

pub struct Reader<'a> {
    ring: &'a Ring,
    tail: u64,
    head: u64,
    _gate: MutexGuard<'a, ()>,
}

impl Reader<'_> {
    #[inline]
    pub fn pop(&mut self) -> Option<(f32, f32)> {
        if self.tail >= self.head {
            return None;
        }
        let slot = ((self.tail & self.ring.mask) * 2) as usize;
        self.tail += 1;
        Some((
            f32::from_bits(self.ring.buf[slot].load(Ordering::Relaxed)),
            f32::from_bits(self.ring.buf[slot + 1].load(Ordering::Relaxed)),
        ))
    }
}

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        self.ring.tail.store(self.tail, Ordering::Release);
    }
}

pub struct Exclusive<'a> {
    ring: &'a Ring,
}

impl Exclusive<'_> {
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn clear(&mut self) {
        let head = self.ring.head.load(Ordering::Relaxed);
        self.ring.tail.store(head, Ordering::Release);
    }

    pub fn truncate(&mut self, keep: usize) {
        let tail = self.ring.tail.load(Ordering::Relaxed);
        let head = self.ring.head.load(Ordering::Relaxed);
        if head.saturating_sub(tail) > keep as u64 {
            self.ring.head.store(tail + keep as u64, Ordering::Release);
        }
    }
}
