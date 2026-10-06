//! Deadline index whose traversal matches the pinned fastutil long map. Heap
//! insertion order decides exact ties between chunk heads after save loading.
#[derive(Clone, Debug)]
pub(super) struct DeadlineIndex {
    keys: Vec<i64>,
    values: Vec<i64>,
    zero: bool,
    size: usize,
}
fn mix(key: i64) -> usize {
    let mut value = (key as u64).wrapping_mul(0x9e3779b97f4a7c15);
    value ^= value >> 32;
    (value ^ (value >> 16)) as usize
}
impl Default for DeadlineIndex {
    fn default() -> Self {
        Self {
            keys: vec![0; 33],
            values: vec![0; 33],
            zero: false,
            size: 0,
        }
    }
}
impl DeadlineIndex {
    fn capacity(&self) -> usize {
        self.keys.len() - 1
    }
    fn find(&self, key: i64) -> usize {
        if key == 0 {
            return self.capacity();
        }
        let mask = self.capacity() - 1;
        let mut slot = mix(key) & mask;
        while self.keys[slot] != 0 && self.keys[slot] != key {
            slot = (slot + 1) & mask;
        }
        slot
    }
    pub(super) fn put(&mut self, key: i64, value: i64) {
        let slot = self.find(key);
        let exists = if key == 0 {
            self.zero
        } else {
            self.keys[slot] != 0
        };
        self.values[slot] = value;
        if exists {
            return;
        }
        self.keys[slot] = key;
        if key == 0 {
            self.zero = true;
        }
        let old_size = self.size;
        self.size += 1;
        if old_size >= self.capacity() * 3 / 4 {
            self.rehash(self.capacity() * 2);
        }
    }
    fn rehash(&mut self, capacity: usize) {
        let old_keys = std::mem::replace(&mut self.keys, vec![0; capacity + 1]);
        let old_values = std::mem::replace(&mut self.values, vec![0; capacity + 1]);
        for slot in (0..old_keys.len() - 1).rev() {
            if old_keys[slot] == 0 {
                continue;
            }
            let destination = self.find(old_keys[slot]);
            self.keys[destination] = old_keys[slot];
            self.values[destination] = old_values[slot];
        }
        self.values[capacity] = *old_values.last().unwrap();
    }
    fn shift_keys(&mut self, mut hole: usize, wrapped: &mut Vec<i64>) {
        let mask = self.capacity() - 1;
        loop {
            let mut slot = (hole + 1) & mask;
            loop {
                let key = self.keys[slot];
                if key == 0 {
                    self.keys[hole] = 0;
                    return;
                }
                let home = mix(key) & mask;
                let crosses = if hole <= slot {
                    hole >= home || home > slot
                } else {
                    hole >= home && home > slot
                };
                if crosses {
                    break;
                }
                slot = (slot + 1) & mask;
            }
            if slot < hole {
                wrapped.push(self.keys[slot]);
            }
            self.keys[hole] = self.keys[slot];
            self.values[hole] = self.values[slot];
            hole = slot;
        }
    }
    pub(super) fn remove(&mut self, key: i64) {
        let slot = self.find(key);
        if key == 0 {
            if !self.zero {
                return;
            }
            self.zero = false;
        } else {
            if self.keys[slot] == 0 {
                return;
            }
            self.shift_keys(slot, &mut Vec::new());
        }
        self.size -= 1;
        if self.capacity() > 32 && self.size < self.capacity() * 3 / 16 {
            self.rehash(self.capacity() / 2);
        }
    }
    /// Visit in vanilla iterator order. The callback updates the deadline and
    /// returns true to remove the entry. Regular iterator removal does not shrink.
    pub(super) fn visit(&mut self, mut action: impl FnMut(i64, &mut i64) -> bool) {
        let mut remaining = self.size;
        let mut position = self.capacity() as isize;
        let mut wrapped = Vec::new();
        let mut zero = self.zero;
        while remaining > 0 {
            remaining -= 1;
            let slot = if zero {
                zero = false;
                self.capacity()
            } else {
                loop {
                    position -= 1;
                    if position < 0 {
                        break self.find(wrapped[(-position - 1) as usize]);
                    }
                    if self.keys[position as usize] != 0 {
                        break position as usize;
                    }
                }
            };
            let key = self.keys[slot];
            if !action(key, &mut self.values[slot]) {
                continue;
            }
            if slot == self.capacity() {
                self.zero = false;
                self.size -= 1;
            } else if position >= 0 {
                self.shift_keys(slot, &mut wrapped);
                self.size -= 1;
            } else {
                self.remove(key);
            }
        }
    }
}
