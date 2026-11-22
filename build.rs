use std::{env, fmt::Write as _, fs, path::Path};

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim().trim_start_matches('#'), 16).ok()
}

fn main() {
    for (feature, lib, min) in [("CARGO_FEATURE_TRACKER", "libopenmpt", "0.6.0"), ("CARGO_FEATURE_CHIPTUNE", "libgme", "0.6.3")] {
        if env::var_os(feature).is_none() {
            continue;
        }
        if let Err(e) = pkg_config::Config::new().atleast_version(min).probe(lib) {
            panic!("{lib} {min} or newer not found ({e}). macOS: brew install pkg-config libopenmpt game-music-emu; Debian/Pi: apt install pkg-config libopenmpt-dev libgme-dev; or build without the `tracker`/`chiptune` features");
        }
    }
    let src = "themes/iterm2_color_schemes.txt";
    println!("cargo:rerun-if-changed={src}");
    let text = fs::read_to_string(src).expect("themes file");

    let mut out = String::from("pub static THEMES: &[ThemeData] = &[\n");
    let mut name: Option<String> = None;
    let mut ansi = [None::<u32>; 16];
    let (mut bg, mut fg, mut sel_bg, mut sel_fg) = (None, None, None, None);

    let flush = |name: &Option<String>, ansi: &[Option<u32>; 16], bg, fg, sel_bg: Option<u32>, sel_fg: Option<u32>, out: &mut String| {
        if let (Some(n), Some(bg), Some(fg)) = (name, bg, fg) {
            if ansi.iter().all(std::option::Option::is_some) {
                let a: Vec<String> = ansi.iter().map(|c| format!("0x{:06x}", c.unwrap())).collect();
                let opt = |c: Option<u32>| c.map_or("None".into(), |v| format!("Some(0x{v:06x})"));
                let _ = writeln!(
                    out,
                    "    ThemeData {{ name: {:?}, bg: 0x{:06x}, fg: 0x{:06x}, sel_bg: {}, sel_fg: {}, ansi: [{}] }},",
                    n, bg, fg, opt(sel_bg), opt(sel_fg), a.join(", ")
                );
            }
        }
    };

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            flush(&name, &ansi, bg, fg, sel_bg, sel_fg, &mut out);
            name = Some(line[1..line.len() - 1].to_string());
            ansi = [None; 16];
            (bg, fg, sel_bg, sel_fg) = (None, None, None, None);
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "palette" => {
                if let Some((i, c)) = v.split_once('=') {
                    if let (Ok(i), Some(c)) = (i.trim().parse::<usize>(), hex(c)) {
                        if i < 16 {
                            ansi[i] = Some(c);
                        }
                    }
                }
            }
            "background" => bg = hex(v),
            "foreground" => fg = hex(v),
            "selection-background" => sel_bg = hex(v),
            "selection-foreground" => sel_fg = hex(v),
            _ => {}
        }
    }
    flush(&name, &ansi, bg, fg, sel_bg, sel_fg, &mut out);
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("themes_data.rs");
    fs::write(dest, out).unwrap();
}
