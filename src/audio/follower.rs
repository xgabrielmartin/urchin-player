use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

pub(super) const NEW_DEVICE: Duration = Duration::from_secs(5);
pub(super) const CONFIRM: Duration = Duration::from_millis(300);
pub(super) const SETTLE: Duration = Duration::from_secs(1);
pub(super) const LEFT_BEHIND: Duration = Duration::from_secs(3);

pub(super) struct Follower<K> {
    first_seen: HashMap<K, Option<Instant>>,
    pending: Option<(K, Instant, Duration)>,
}

impl<K: Clone + Eq + Hash> Follower<K> {
    pub(super) fn new(present: impl IntoIterator<Item = K>) -> Follower<K> {
        Follower { first_seen: present.into_iter().map(|k| (k, None)).collect(), pending: None }
    }

    pub(super) fn check(&mut self, present: &[K], default: Option<&K>, current: Option<&K>, now: Instant) -> Option<K> {
        self.first_seen.retain(|k, _| present.contains(k));
        for k in present {
            self.first_seen.entry(k.clone()).or_insert(Some(now));
        }
        let (Some(default), Some(current)) = (default, current) else {
            self.pending = None;
            return None;
        };
        if default == current || !present.contains(current) || !present.contains(default) {
            self.pending = None;
            return None;
        }
        if let Some((k, since, hold)) = &self.pending {
            if k == default {
                if now.duration_since(*since) < *hold {
                    return None;
                }
                self.pending = None;
                return Some(default.clone());
            }
        }
        let new = matches!(self.first_seen.get(default), Some(Some(t)) if now.duration_since(*t) <= NEW_DEVICE);
        self.pending = Some((default.clone(), now, if new { CONFIRM } else { SETTLE }));
        None
    }

    pub(super) fn seen_for(&self, k: &K, now: Instant) -> Option<Option<Duration>> {
        self.first_seen.get(k).map(|t| t.map(|t| now.duration_since(t)))
    }

    pub(super) fn listed(&self) -> impl Iterator<Item = &K> {
        self.first_seen.keys()
    }

    pub(super) fn confirm_due(&self, now: Instant) -> bool {
        self.pending.as_ref().is_some_and(|(_, since, hold)| now.duration_since(*since) >= *hold)
    }
}

#[cfg(test)]
#[path = "follower_tests.rs"]
mod tests;
