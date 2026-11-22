use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::library::Tags;
use crate::search::fold;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Tagged { album: String, artist: String },
    TaggedIn { album: String, dir: PathBuf },
    Folder(PathBuf),
}

pub fn is_disc_folder(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let rest = ["disc", "disk", "cd"].iter().find_map(|p| lower.strip_prefix(p));
    let Some(rest) = rest else { return false };
    let rest = rest.trim_start_matches([' ', '_', '-', '.']);
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].chars().next().is_none_or(|c| !c.is_alphanumeric())
}

pub fn album_dir(song: &Path) -> PathBuf {
    let Some(dir) = song.parent() else { return PathBuf::new() };
    let disc = dir.file_name().is_some_and(|n| is_disc_folder(&n.to_string_lossy()));
    match dir.parent() {
        Some(up) if disc => up.to_path_buf(),
        _ => dir.to_path_buf(),
    }
}

pub fn key(song: &Path, tags: &Tags) -> Key {
    let album = tags.album.trim();
    if album.is_empty() {
        return Key::Folder(album_dir(song));
    }
    let artist = tags.album_artist.trim();
    if artist.is_empty() {
        Key::TaggedIn { album: fold(album), dir: album_dir(song) }
    } else {
        Key::Tagged { album: fold(album), artist: fold(artist) }
    }
}

pub fn keys(songs: &[PathBuf], cache: Option<&Path>) -> Vec<Key> {
    let mut tags = crate::library::tags_of(songs, cache);
    if tags.len() != songs.len() {
        tags = vec![Tags::default(); songs.len()];
    }
    songs.iter().zip(&tags).map(|(s, t)| key(s, t)).collect()
}

pub fn group<T: Clone>(songs: &[T], keys: &[Key]) -> Vec<Vec<T>> {
    let mut place: HashMap<&Key, usize> = HashMap::new();
    let mut albums: Vec<Vec<T>> = Vec::new();
    for (song, k) in songs.iter().zip(keys) {
        let at = *place.entry(k).or_insert_with(|| {
            albums.push(Vec::new());
            albums.len() - 1
        });
        albums[at].push(song.clone());
    }
    albums
}

pub fn shuffled_order(keys: &[Key], seed: u64) -> (Vec<usize>, usize) {
    let places: Vec<usize> = (0..keys.len()).collect();
    let mut albums = group(&places, keys);
    crate::queue::shuffle(&mut albums, seed);
    let count = albums.len();
    (albums.into_iter().flatten().collect(), count)
}

pub fn shuffle(songs: &mut Vec<PathBuf>, keys: &[Key], seed: u64) -> usize {
    let (order, count) = shuffled_order(keys, seed);
    *songs = order.into_iter().map(|i| songs[i].clone()).collect();
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(album: &str, album_artist: &str) -> Tags {
        Tags { album: album.into(), album_artist: album_artist.into(), ..Tags::default() }
    }

    #[test]
    fn disc_folders_are_recognized() {
        for yes in ["CD1", "cd 2", "Disc_03", "Disk-4", "disc 1 - bonus", "CD.2", "DISC10"] {
            assert!(is_disc_folder(yes), "{yes}");
        }
        for no in ["CDs", "Discography", "cd", "Disco 2000", "CD1A", "Disk Jockey", "Hard Disk 2"] {
            assert!(!is_disc_folder(no), "{no}");
        }
    }

    #[test]
    fn untagged_songs_group_by_folder_and_discs_join_their_album() {
        let a = Path::new("/m/punk/Minor Threat/01 Filler.mp3");
        let b = Path::new("/m/punk/Minor Threat/02 Seeing Red.mp3");
        let c = Path::new("/m/rock/Some Album/CD1/01 a.flac");
        let d = Path::new("/m/rock/Some Album/CD2/01 b.flac");
        let e = Path::new("/m/rock/Other/01 c.flac");
        let none = Tags::default();
        assert_eq!(key(a, &none), key(b, &none));
        assert_eq!(key(c, &none), key(d, &none));
        assert_eq!(key(c, &none), Key::Folder(PathBuf::from("/m/rock/Some Album")));
        assert_ne!(key(c, &none), key(e, &none));
    }

    #[test]
    fn tags_join_albums_across_folders_and_split_folders_with_several_albums() {
        let x = Path::new("/m/a/x.mp3");
        let y = Path::new("/m/b/y.mp3");
        assert_eq!(key(x, &tags("Hits", "ABBA")), key(y, &tags("hits", "abba")));
        assert_ne!(key(x, &tags("One", "ABBA")), key(x, &tags("Two", "ABBA")));
        assert_ne!(key(x, &tags("Greatest Hits", "")), key(y, &tags("Greatest Hits", "")));
        let z = Path::new("/m/a/z.mp3");
        let (mut tx, mut tz) = (tags("Now 42", ""), tags("Now 42", ""));
        tx.artist = "One".into();
        tz.artist = "Other".into();
        assert_eq!(key(x, &tx), key(z, &tz));
    }

    #[test]
    fn shuffling_keeps_each_album_whole_and_in_order() {
        let songs: Vec<PathBuf> = ["A/1", "A/2", "A/3", "B/1", "B/2", "C/1", "D/1", "D/2"].iter().map(|s| PathBuf::from(format!("/m/{s}.mp3"))).collect();
        let ks: Vec<Key> = songs.iter().map(|s| key(s, &Tags::default())).collect();
        let mut seen_orders = std::collections::HashSet::new();
        for seed in 1..40 {
            let mut mixed = songs.clone();
            assert_eq!(shuffle(&mut mixed, &ks, seed), 4);
            assert_eq!(mixed.len(), songs.len());
            let mixed_keys: Vec<Key> = mixed.iter().map(|s| key(s, &Tags::default())).collect();
            let albums = group(&mixed, &mixed_keys);
            assert_eq!(albums.len(), 4, "an album was split: {mixed:?}");
            for album in &albums {
                let mut sorted = album.clone();
                sorted.sort();
                assert_eq!(*album, sorted, "songs out of order");
            }
            seen_orders.insert(albums.iter().map(|a| a[0].clone()).collect::<Vec<_>>());
        }
        assert!(seen_orders.len() > 5, "album order hardly changes");
    }
}
