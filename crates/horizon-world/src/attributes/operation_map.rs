//! The pinned operation buckets use open-addressed maps. Preserve their slot
//! iteration order because reassociating floating-point modifiers changes values.
use super::Modifier;
#[derive(Clone, Debug)]
pub(super) struct OperationMap {
    slots: Vec<Option<Modifier>>,
    len: usize,
}
impl Default for OperationMap {
    fn default() -> Self {
        Self {
            slots: vec![None; 32],
            len: 0,
        }
    }
}
fn mixed(m: &Modifier) -> usize {
    let h = m.0.hash.wrapping_mul(0x9e37_79b9);
    (h ^ (h >> 16)) as usize
}
fn capacity(len: usize) -> usize {
    ((len as f64 / 0.75).ceil() as usize)
        .max(2)
        .next_power_of_two()
}
impl OperationMap {
    pub(super) fn values(&self) -> impl Iterator<Item = &Modifier> {
        self.slots.iter().rev().filter_map(Option::as_ref)
    }
    fn max_fill(&self) -> usize {
        self.slots.len() * 3 / 4
    }
    fn find(&self, m: &Modifier) -> usize {
        let mask = self.slots.len() - 1;
        let mut p = mixed(m) & mask;
        while self.slots[p].as_ref().is_some_and(|old| old.id() != m.id()) {
            p = (p + 1) & mask;
        }
        p
    }
    pub(super) fn put(&mut self, m: Modifier) {
        let p = self.find(&m);
        let new = self.slots[p].is_none();
        self.slots[p] = Some(m);
        if new {
            let old = self.len;
            self.len += 1;
            if old >= self.max_fill() {
                self.rehash(capacity(self.len + 1));
            }
        }
    }
    fn rehash(&mut self, n: usize) {
        let old = std::mem::replace(&mut self.slots, vec![None; n]);
        for m in old.into_iter().rev().flatten() {
            let p = self.find(&m);
            self.slots[p] = Some(m);
        }
    }
    pub(super) fn remove(&mut self, m: &Modifier) {
        let p = self.find(m);
        if self.slots[p].is_none() {
            return;
        }
        self.len -= 1;
        let mask = self.slots.len() - 1;
        let mut hole = p;
        loop {
            let mut p = (hole + 1) & mask;
            loop {
                let Some(key) = self.slots[p].as_ref() else {
                    self.slots[hole] = None;
                    if self.slots.len() > 32 && self.len < self.max_fill() / 4 {
                        self.rehash(self.slots.len() / 2);
                    }
                    return;
                };
                let home = mixed(key) & mask;
                let moves = if hole <= p {
                    hole >= home || home > p
                } else {
                    hole >= home && home > p
                };
                if moves {
                    break;
                }
                p = (p + 1) & mask;
            }
            self.slots[hole] = self.slots[p].take();
            hole = p;
        }
    }
    pub(super) fn copy_for_replace(&self) -> Self {
        let mut result = Self::default();
        let n = capacity(self.len);
        if n > result.slots.len() {
            result.rehash(n);
        }
        for m in self.values() {
            result.put(m.clone());
        }
        result
    }
}
