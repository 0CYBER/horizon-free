//! Array heap with the pinned JDK PriorityQueue's equal-key and iterator-removal
//! behavior. Equal comparator keys are observable in saved and loaded ticks.
use std::{cmp::Ordering, collections::VecDeque};

#[derive(Clone, Debug)]
pub(super) struct Heap<T> {
    pub(super) values: Vec<T>,
}
impl<T> Default for Heap<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}
impl<T: Copy + PartialEq> Heap<T> {
    pub(super) fn peek(&self) -> Option<T> {
        self.values.first().copied()
    }
    pub(super) fn push(&mut self, value: T, compare: impl Fn(T, T) -> Ordering) {
        self.values.push(value);
        self.up(self.values.len() - 1, value, &compare);
    }
    fn up(&mut self, mut index: usize, value: T, compare: &impl Fn(T, T) -> Ordering) -> usize {
        while index > 0 {
            let parent = (index - 1) / 2;
            if compare(value, self.values[parent]) != Ordering::Less {
                break;
            }
            self.values[index] = self.values[parent];
            index = parent;
        }
        self.values[index] = value;
        index
    }
    fn down(&mut self, mut index: usize, value: T, compare: &impl Fn(T, T) -> Ordering) -> usize {
        let half = self.values.len() / 2;
        while index < half {
            let mut child = index * 2 + 1;
            let right = child + 1;
            if right < self.values.len()
                && compare(self.values[child], self.values[right]) == Ordering::Greater
            {
                child = right;
            }
            if compare(value, self.values[child]) != Ordering::Greater {
                break;
            }
            self.values[index] = self.values[child];
            index = child;
        }
        self.values[index] = value;
        index
    }
    pub(super) fn pop(&mut self, compare: impl Fn(T, T) -> Ordering) -> Option<T> {
        let first = self.peek()?;
        let last = self.values.pop().unwrap();
        if !self.values.is_empty() {
            self.down(0, last, &compare);
        }
        Some(first)
    }
    fn remove_at(&mut self, index: usize, compare: &impl Fn(T, T) -> Ordering) -> Option<T> {
        let last = self.values.pop().unwrap();
        if index == self.values.len() {
            return None;
        }
        if self.down(index, last, compare) == index && self.up(index, last, compare) != index {
            Some(last)
        } else {
            None
        }
    }
    pub(super) fn remove_if(
        &mut self,
        mut predicate: impl FnMut(T) -> bool,
        compare: impl Fn(T, T) -> Ordering,
    ) {
        let mut cursor = 0;
        let mut moved = VecDeque::new();
        while cursor < self.values.len() {
            let value = self.values[cursor];
            cursor += 1;
            if predicate(value) {
                if let Some(value) = self.remove_at(cursor - 1, &compare) {
                    moved.push_back(value);
                } else {
                    cursor -= 1;
                }
            }
        }
        while let Some(value) = moved.pop_front() {
            if predicate(value) {
                let index = self
                    .values
                    .iter()
                    .position(|&candidate| candidate == value)
                    .unwrap();
                self.remove_at(index, &compare);
            }
        }
    }
}
