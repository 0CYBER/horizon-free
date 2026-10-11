//! Iteration order of fastutil's `Object2ObjectOpenHashMap<String, _>` (open
//! addressing with linear probing, backward-shift deletion and rehashing at
//! the load factor), for the vanilla collections whose order commands show:
//! the scoreboard's objectives and teams.

/// The keys of an open hash map in their table slots.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenOrder {
    keys: Vec<Option<String>>,
    size: usize,
    min_n: usize,
    load: f32,
}
impl Default for OpenOrder {
    /// `new Object2ObjectOpenHashMap<>()`: 16 expected entries, load 0.75.
    fn default() -> Self {
        Self::new(16, 0.75)
    }
}
/// `String.hashCode`.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}
/// `HashCommon.arraySize`.
fn array_size(expected: usize, f: f32) -> usize {
    let wanted = (expected as f64 / f64::from(f)).ceil() as usize;
    wanted.max(2).next_power_of_two()
}
impl OpenOrder {
    pub fn new(expected: usize, load: f32) -> Self {
        let n = array_size(expected, load);
        Self {
            keys: vec![None; n],
            size: 0,
            min_n: n,
            load,
        }
    }
    fn max_fill(&self) -> usize {
        ((self.keys.len() as f64 * f64::from(self.load)).ceil() as usize).min(self.keys.len() - 1)
    }
    fn slot(&self, key: &str) -> usize {
        mix(java_hash(key)) & (self.keys.len() - 1)
    }
    pub fn insert(&mut self, key: &str) {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        while let Some(k) = &self.keys[pos] {
            if k == key {
                return;
            }
            pos = (pos + 1) & mask;
        }
        self.keys[pos] = Some(key.to_owned());
        self.size += 1;
        if self.size >= self.max_fill() {
            self.rehash(array_size(self.size + 1, self.load));
        }
    }
    pub fn remove(&mut self, key: &str) {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        loop {
            match &self.keys[pos] {
                None => return,
                Some(k) if k == key => break,
                Some(_) => pos = (pos + 1) & mask,
            }
        }
        self.size -= 1;
        self.shift_keys(pos);
        let n = self.keys.len();
        if n > self.min_n && self.size < self.max_fill() / 4 && n > 16 {
            self.rehash(n / 2);
        }
    }
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.keys.len() - 1;
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(curr) = self.keys[pos].clone() else {
                    self.keys[last] = None;
                    return;
                };
                let slot = self.slot(&curr);
                let stop = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stop {
                    self.keys[last] = Some(curr);
                    break;
                }
                pos = (pos + 1) & mask;
            }
        }
    }
    fn rehash(&mut self, new_n: usize) {
        let old = std::mem::replace(&mut self.keys, vec![None; new_n]);
        let mask = new_n - 1;
        for key in old.into_iter().rev().flatten() {
            let mut pos = mix(java_hash(&key)) & mask;
            while self.keys[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = Some(key);
        }
    }
    /// The iteration order (`keySet().iterator()`: from the last slot down).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().rev().flatten().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_order() {
        assert_eq!(
            java_hash("harness"),
            "harness"
                .chars()
                .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
        );
        let mut o = OpenOrder::new(16, 0.5);
        for k in ["harness", "harness_h"] {
            o.insert(k);
        }
        let keys: Vec<&str> = o.keys().collect();
        assert_eq!(keys.len(), 2);
        o.remove("harness");
        assert_eq!(o.keys().collect::<Vec<_>>(), vec!["harness_h"]);
        for i in 0..40 {
            o.insert(&format!("k{i}"));
        }
        assert_eq!(o.keys().count(), 41);
        for i in 0..40 {
            o.remove(&format!("k{i}"));
        }
        assert_eq!(o.keys().collect::<Vec<_>>(), vec!["harness_h"]);
    }
}
