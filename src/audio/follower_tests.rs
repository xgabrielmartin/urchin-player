use super::{Follower, CONFIRM, NEW_DEVICE, SETTLE};
use std::time::{Duration, Instant};

const SPEAKERS: u32 = 1;
const BLUETOOTH: u32 = 2;
const SEC: Duration = Duration::from_secs(1);

#[test]
fn follows_a_speaker_switched_on() {
    let t = Instant::now();
    let mut f = Follower::new([SPEAKERS]);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t), None);
    let soon = t + CONFIRM / 2;
    assert!(!f.confirm_due(soon));
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), soon), None);
    assert!(f.confirm_due(t + CONFIRM));
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + CONFIRM), Some(BLUETOOTH));
    let mut f = Follower::new([SPEAKERS]);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&SPEAKERS), t), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SEC), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + 2 * SEC), Some(BLUETOOTH));
}

#[test]
fn never_falls_back_to_the_laptop_speakers() {
    let t = Instant::now();
    let mut f = Follower::new([SPEAKERS, BLUETOOTH]);
    let half = SETTLE / 2;
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&BLUETOOTH), t), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&BLUETOOTH), t + half), None);
    assert!(!f.confirm_due(t + half));
    assert_eq!(f.check(&[SPEAKERS], Some(&SPEAKERS), Some(&BLUETOOTH), t + half), None);
    assert!(!f.confirm_due(t + half + SETTLE));
    let mut f = Follower::new([SPEAKERS]);
    f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SEC), Some(BLUETOOTH));
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&BLUETOOTH), t + 2 * SEC), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&BLUETOOTH), t + 2 * SEC + half), None);
    assert_eq!(f.check(&[SPEAKERS], Some(&SPEAKERS), Some(&BLUETOOTH), t + 2 * SEC + half), None);
    assert!(!f.confirm_due(t + 2 * SEC + half + SETTLE));
}

#[test]
fn a_default_that_was_already_connected_is_followed_once_it_has_settled() {
    let t = Instant::now();
    let mut f = Follower::new([SPEAKERS, BLUETOOTH]);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SETTLE / 2), None);
    assert!(!f.confirm_due(t + SETTLE / 2));
    assert!(f.confirm_due(t + SETTLE));
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SETTLE), Some(BLUETOOTH));
    let mut f = Follower::new([SPEAKERS]);
    f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&SPEAKERS), t);
    let later = t + NEW_DEVICE + SEC;
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), later), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), later + SETTLE / 2), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), later + SETTLE), Some(BLUETOOTH));
    let mut f = Follower::new([SPEAKERS, BLUETOOTH]);
    f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t);
    f.check(&[SPEAKERS, BLUETOOTH], Some(&SPEAKERS), Some(&SPEAKERS), t + SETTLE / 2);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SETTLE), None);
    let mut f = Follower::new([SPEAKERS]);
    assert_eq!(f.check(&[SPEAKERS], Some(&BLUETOOTH), Some(&SPEAKERS), t), None);
    assert!(!f.confirm_due(t + SETTLE));
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SEC), None);
    assert_eq!(f.check(&[SPEAKERS, BLUETOOTH], Some(&BLUETOOTH), Some(&SPEAKERS), t + SEC + CONFIRM), Some(BLUETOOTH));
}
