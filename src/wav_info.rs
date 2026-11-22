use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use symphonia::core::meta::well_known::METADATA_ID_WAVE;
use symphonia::core::meta::{MetadataBuilder, MetadataInfo, Tag};
use symphonia_metadata::embedded::riff::parse_riff_info_chunk;

const INFO: MetadataInfo = MetadataInfo { metadata: METADATA_ID_WAVE, short_name: "wave", long_name: "Waveform Audio File Format" };

const MAX_LIST: u32 = 1 << 20;

pub fn tags(path: &Path) -> Vec<Tag> {
    let Ok(file) = File::open(path) else { return Vec::new() };
    let mut r = BufReader::new(file);
    let mut builder = MetadataBuilder::new(INFO);
    if read(&mut r, &mut builder).is_none() {
        return Vec::new();
    }
    builder.build().media.tags
}

fn read(r: &mut (impl Read + Seek), builder: &mut MetadataBuilder) -> Option<()> {
    let mut head = [0u8; 12];
    r.read_exact(&mut head).ok()?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    loop {
        let mut chunk = [0u8; 8];
        if r.read_exact(&mut chunk).is_err() {
            return Some(());
        }
        let id = &chunk[0..4];
        let len = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        let padded = u64::from(len) + u64::from(len % 2);
        if id == b"LIST" && (4..=MAX_LIST).contains(&len) {
            let mut list = vec![0u8; len as usize];
            r.read_exact(&mut list).ok()?;
            if &list[0..4] == b"INFO" {
                info_entries(&list[4..], builder);
            }
            if len % 2 == 1 {
                r.seek(SeekFrom::Current(1)).ok()?;
            }
        } else {
            r.seek(SeekFrom::Current(i64::try_from(padded).ok()?)).ok()?;
        }
    }
}

fn info_entries(mut buf: &[u8], builder: &mut MetadataBuilder) {
    while buf.len() >= 8 {
        let id = [buf[0], buf[1], buf[2], buf[3]];
        let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let Some(text) = buf.get(8..8 + len) else { return };
        let _ = parse_riff_info_chunk(id, text, builder);
        let next = (8 + len + len % 2).min(buf.len());
        buf = &buf[next..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::wav_with_tags;
    use symphonia::core::meta::StandardTag;

    fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("urchin_wav_info_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn info_tags_come_out_as_symphonia_names_them() {
        let p = scratch("std", &wav_with_tags(&[("INAM", "Time"), ("IART", "Pink Floyd"), ("IPRD", "Dark Side")]));
        let t = tags(&p);
        assert!(t.iter().any(|t| matches!(&t.std, Some(StandardTag::TrackTitle(v)) if v.trim_end_matches('\0') == "Time")));
        assert!(t.iter().any(|t| matches!(&t.std, Some(StandardTag::Artist(v)) if v.trim_end_matches('\0') == "Pink Floyd")));
        assert!(t.iter().any(|t| matches!(&t.std, Some(StandardTag::Album(v)) if v.trim_end_matches('\0') == "Dark Side")));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn tags_after_the_audio_are_found_and_broken_files_give_none() {
        let wav = wav_with_tags(&[("INAM", "Late")]);
        let list_at = wav.windows(4).position(|w| w == b"LIST").unwrap();
        let data_at = wav.windows(4).position(|w| w == b"data").unwrap();
        let mut moved = wav[..list_at].to_vec();
        moved.extend_from_slice(&wav[data_at..]);
        moved.extend_from_slice(&wav[list_at..data_at]);
        let p = scratch("late", &moved);
        assert!(tags(&p).iter().any(|t| matches!(&t.std, Some(StandardTag::TrackTitle(_)))));

        for (name, bytes) in [("cut", wav[..list_at + 20].to_vec()), ("mp3", b"ID3\x04not a wav".to_vec()), ("empty", Vec::new())] {
            let p = scratch(name, &bytes);
            assert!(tags(&p).is_empty(), "{name}");
            let _ = std::fs::remove_dir_all(p.parent().unwrap());
        }
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
