use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq, Eq)]
pub enum Shown {
    Selected,
    #[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
    Folder,
    NoDesktop,
    Failed,
}

pub fn label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in File Manager"
    }
}

fn over_ssh() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}

pub fn show(path: &Path) -> Shown {
    if over_ssh() {
        return Shown::NoDesktop;
    }
    #[cfg(target_os = "macos")]
    return show_macos(path);
    #[cfg(windows)]
    return show_windows(path);
    #[cfg(not(any(target_os = "macos", windows)))]
    return show_freedesktop(path);
}

pub fn open_url(url: &str) -> bool {
    if over_ssh() {
        return false;
    }
    let Some(mut cmd) = opener() else { return false };
    cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("urchin-open".into()).spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn opener() -> Option<Command> {
    Some(Command::new("open"))
}

#[cfg(windows)]
fn opener() -> Option<Command> {
    Some(Command::new("explorer"))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn opener() -> Option<Command> {
    if in_wsl() {
        Some(Command::new("explorer.exe"))
    } else if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Some(Command::new("xdg-open"))
    } else {
        None
    }
}

fn ran(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(target_os = "macos")]
fn show_macos(path: &Path) -> Shown {
    if ran(Command::new("open").arg("-R").arg(path)) {
        Shown::Selected
    } else {
        Shown::Failed
    }
}

#[cfg(windows)]
fn show_windows(path: &Path) -> Shown {
    use std::os::windows::process::CommandExt;
    let arg = format!("/select,\"{}\"", path.display());
    match Command::new("explorer").raw_arg(arg).stdin(Stdio::null()).status() {
        Ok(_) => Shown::Selected,
        Err(_) => Shown::Failed,
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn show_freedesktop(path: &Path) -> Shown {
    let folder = path.parent().unwrap_or(path);
    if in_wsl() {
        let Some(win) = Command::new("wslpath").arg("-w").arg(folder).stderr(Stdio::null()).output().ok().filter(|o| o.status.success()) else {
            return Shown::Failed;
        };
        let win = String::from_utf8_lossy(&win.stdout).trim().to_string();
        return match Command::new("explorer.exe").arg(win).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(_) => Shown::Folder,
            Err(_) => Shown::Failed,
        };
    }
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Shown::NoDesktop;
    }
    if show_items(path).is_ok() {
        return Shown::Selected;
    }
    if ran(Command::new("xdg-open").arg(folder)) {
        Shown::Folder
    } else {
        Shown::Failed
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn show_items(path: &Path) -> zbus::Result<()> {
    let uris = vec![file_uri(path)];
    zbus::block_on(async {
        let bus = zbus::Connection::session().await?;
        bus.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(uris, ""),
        )
        .await?;
        Ok(())
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn in_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease").is_ok_and(|s| s.to_lowercase().contains("microsoft"))
}

#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
pub fn file_uri(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();
    let mut uri = String::from("file://");
    if !bytes.starts_with(b"/") {
        uri.push('/');
    }
    for b in bytes {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

pub fn clipboard_escape(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_right() {
        for (plain, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
        assert_eq!(clipboard_escape("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn file_uris_escape_what_they_must() {
        assert_eq!(file_uri(Path::new("/Music/R&B/Beyoncé - Halo #1 (100%).flac")), "file:///Music/R%26B/Beyonc%C3%A9%20-%20Halo%20%231%20%28100%25%29.flac");
        assert_eq!(file_uri(Path::new("/a/b.mp3")), "file:///a/b.mp3");
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn over_ssh_or_without_a_display_nothing_is_opened() {
        let _g = crate::tests::serial();
        let keep: Vec<(&str, Option<std::ffi::OsString>)> =
            ["SSH_CONNECTION", "SSH_TTY", "DISPLAY", "WAYLAND_DISPLAY", "WSL_DISTRO_NAME"].iter().map(|k| (*k, std::env::var_os(k))).collect();
        let path = Path::new("/Music/a.flac");
        std::env::set_var("SSH_CONNECTION", "10.0.0.2 51000 10.0.0.3 22");
        assert_eq!(show(path), Shown::NoDesktop);
        std::env::remove_var("SSH_CONNECTION");
        std::env::remove_var("SSH_TTY");
        std::env::remove_var("DISPLAY");
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WSL_DISTRO_NAME");
        if !in_wsl() {
            assert_eq!(show(path), Shown::NoDesktop);
        }
        for (k, v) in keep {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    #[ignore]
    fn show_items_reaches_a_file_manager_on_the_session_bus() {
        use std::io::{BufRead, BufReader};
        use std::sync::{Arc, Mutex};
        let _g = crate::tests::serial();
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();
        let keep: Vec<(&str, Option<std::ffi::OsString>)> = ["DBUS_SESSION_BUS_ADDRESS", "DISPLAY", "SSH_CONNECTION", "SSH_TTY"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect();
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim());
        std::env::set_var("DISPLAY", ":0");
        std::env::remove_var("SSH_CONNECTION");
        std::env::remove_var("SSH_TTY");

        struct FakeFiles(Arc<Mutex<Vec<String>>>);
        #[zbus::interface(name = "org.freedesktop.FileManager1")]
        impl FakeFiles {
            fn show_items(&self, uris: Vec<String>, _startup_id: String) {
                self.0.lock().unwrap().extend(uris);
            }
        }
        let asked = Arc::new(Mutex::new(Vec::new()));
        let path = Path::new("/Music/R&B/Beyoncé - Halo.flac");
        assert!(show_items(path).is_err());
        let (ready, is_ready) = std::sync::mpsc::channel();
        let fake = FakeFiles(asked.clone());
        std::thread::spawn(move || {
            zbus::block_on(async move {
                let _files = zbus::connection::Builder::session()
                    .name("org.freedesktop.FileManager1")
                    .serve_at("/org/freedesktop/FileManager1", fake)
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
        is_ready.recv().unwrap();
        assert_eq!(show(path), Shown::Selected);
        assert_eq!(*asked.lock().unwrap(), [file_uri(path)]);

        for (k, v) in keep {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        let _ = daemon.kill();
        let _ = daemon.wait();
    }
}
