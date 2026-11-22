use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Marks<K: Ord + Clone = usize> {
    set: BTreeSet<K>,
    pub anchor: Option<K>,
}

impl<K: Ord + Clone> Default for Marks<K> {
    fn default() -> Self {
        Marks { set: BTreeSet::new(), anchor: None }
    }
}

impl<K: Ord + Clone> Marks<K> {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn contains(&self, k: &K) -> bool {
        self.set.contains(k)
    }

    pub fn set(&mut self, k: K, on: bool) {
        if on {
            self.set.insert(k.clone());
        } else {
            self.set.remove(&k);
        }
        self.anchor = Some(k);
    }

    pub fn clear(&mut self) {
        self.set.clear();
        self.anchor = None;
    }

    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.set.iter()
    }
}

impl Marks<usize> {
    pub fn set_range(&mut self, a: usize, b: usize, on: bool) {
        for i in a.min(b)..=a.max(b) {
            if on {
                self.set.insert(i);
            } else {
                self.set.remove(&i);
            }
        }
        self.anchor = Some(b);
    }

    pub fn extend_to(&mut self, i: usize) {
        match self.anchor {
            Some(a) => self.set_range(a, i, true),
            None => self.set(i, true),
        }
    }

    pub fn moved(&mut self, from: usize, to: usize) {
        let place = |k: usize| {
            if k == from {
                to
            } else if from < to && k > from && k <= to {
                k - 1
            } else if to < from && k >= to && k < from {
                k + 1
            } else {
                k
            }
        };
        self.set = self.set.iter().map(|&k| place(k)).collect();
        self.anchor = self.anchor.map(place);
    }

    pub fn sorted(&self) -> Vec<usize> {
        self.set.iter().copied().collect()
    }

}

pub fn block_moves(ticked: &[usize], delta: isize, len: usize, can: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut marks: BTreeSet<usize> = ticked.iter().copied().collect();
    let mut order: Vec<usize> = ticked.to_vec();
    if delta > 0 {
        order.reverse();
    }
    let mut moves = Vec::new();
    for i in order {
        let Some(to) = i.checked_add_signed(delta).filter(|&t| t < len) else { continue };
        if marks.contains(&to) || !can(to) {
            continue;
        }
        marks.remove(&i);
        marks.insert(to);
        moves.push((i, to));
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_follow_their_rows() {
        let mut m = Marks::default();
        m.set(2, true);
        m.set(4, true);
        m.extend_to(6);
        assert_eq!(m.sorted(), [2, 4, 5, 6]);
        m.moved(6, 0);
        assert_eq!(m.sorted(), [0, 3, 5, 6]);
        m.set_range(5, 3, false);
        assert_eq!(m.sorted(), [0, 6]);
        assert_eq!(m.anchor, Some(3));
    }

    #[test]
    fn a_block_moves_as_one_and_stops_at_the_edge() {
        let all = |_| true;
        assert_eq!(block_moves(&[2, 3], -1, 6, all), [(2, 1), (3, 2)]);
        assert_eq!(block_moves(&[2, 3], 1, 6, all), [(3, 4), (2, 3)]);
        assert!(block_moves(&[0, 1], -1, 6, all).is_empty());
        assert_eq!(block_moves(&[0, 3], -1, 6, all), [(3, 2)]);
        assert!(block_moves(&[2], -1, 6, |i| i >= 2).is_empty());
    }
}
