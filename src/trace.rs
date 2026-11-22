use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ON: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn init() -> Option<PathBuf> {
    let v = std::env::var_os("URCHIN_TRACE")?;
    if v.is_empty() || v == "0" {
        return None;
    }
    let dir = crate::settings::config_dir();
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("trace.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    ON.store(true, Ordering::Relaxed);
    line(format_args!("==== Urchin {} started, pid {}", env!("CARGO_PKG_VERSION"), std::process::id()));
    Some(path)
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn line(args: fmt::Arguments) {
    if !on() {
        return;
    }
    let stamp = stamp();
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "{stamp} {args}");
    }
}

macro_rules! tlog {
    ($($t:tt)*) => {
        if $crate::trace::on() {
            $crate::trace::line(format_args!($($t)*))
        }
    };
}
pub(crate) use tlog;

fn stamp() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let ms = now.subsec_millis();
    #[cfg(unix)]
    {
        let t = now.as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&t, &mut tm) };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            ms
        )
    }
    #[cfg(not(unix))]
    {
        let s = now.as_secs();
        format!("{:02}:{:02}:{:02}.{:03} UTC", (s / 3600) % 24, (s / 60) % 60, s % 60, ms)
    }
}
