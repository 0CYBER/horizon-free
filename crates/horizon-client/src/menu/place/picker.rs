//! Shared augmenting-path recipe assignment.
/// `StackedContents.RecipePicker`: augmenting paths over item-ingredient pairs.
pub(in crate::menu) struct Picker<'a> {
    amounts: &'a mut [i32],
    ingredient_count: usize,
    /// Indices into the stacked keys of the items any ingredient accepts.
    items: Vec<usize>,
    limits: Vec<i32>,
    assigned: Vec<bool>,
    satisfied: Vec<bool>,
    visited_ingredient: Vec<bool>,
    visited_item: Vec<bool>,
    path: Vec<usize>,
}

impl<'a> Picker<'a> {
    /// Compatibility and per-cell stack caps are computed once, before matching.
    /// Candidate identities are caller-owned: item holders for vanilla, exact
    /// component-equal stack classes for predicate-sensitive mod placement.
    pub(in crate::menu) fn new(
        amounts: &'a mut [i32],
        ingredient_count: usize,
        mut limit: impl FnMut(usize, usize) -> i32,
    ) -> Self {
        let mut items = Vec::new();
        let mut limits = Vec::new();
        for (candidate, count) in amounts.iter().enumerate() {
            if *count <= 0 {
                continue;
            }
            let row = (0..ingredient_count)
                .map(|g| limit(candidate, g))
                .collect::<Vec<_>>();
            if row.iter().any(|cap| *cap > 0) {
                items.push(candidate);
                limits.extend(row);
            }
        }
        let (ni, nt) = (ingredient_count, items.len());
        Self {
            amounts,
            ingredient_count,
            items,
            limits,
            assigned: vec![false; ni * nt],
            satisfied: vec![false; ni],
            visited_ingredient: vec![false; ni],
            visited_item: vec![false; nt],
            path: Vec::new(),
        }
    }
    fn upper_bound(&self) -> i32 {
        (0..self.ingredient_count)
            .map(|g| {
                self.items
                    .iter()
                    .enumerate()
                    .map(|(t, key)| self.amounts[*key].min(self.limits[self.at(t, g)]))
                    .max()
                    .unwrap_or(0)
            })
            .min()
            .unwrap_or(i32::MAX)
    }
    fn at(&self, item: usize, ingredient: usize) -> usize {
        item * self.ingredient_count + ingredient
    }
    fn clear_visited(&mut self) {
        self.visited_ingredient.fill(false);
        self.visited_item.fill(false);
    }
    pub(in crate::menu) fn try_pick(
        &mut self,
        capacity: i32,
        mut output: Option<&mut Vec<usize>>,
    ) -> bool {
        if capacity <= 0 {
            return true;
        }
        let mut satisfied_count = 0;
        loop {
            let Some(path) = self.try_assigning_new_item(capacity) else {
                let valid = satisfied_count == self.ingredient_count;
                let has_output = valid && output.is_some();
                self.clear_visited();
                self.satisfied.fill(false);
                for g in 0..self.ingredient_count {
                    for t in 0..self.items.len() {
                        let at = self.at(t, g);
                        if self.assigned[at] {
                            self.assigned[at] = false;
                            let key = self.items[t];
                            self.amounts[key] += capacity;
                            if has_output && let Some(out) = output.as_deref_mut() {
                                out.push(key);
                            }
                            break;
                        }
                    }
                }
                return valid;
            };
            let key = self.items[path[0]];
            self.amounts[key] -= capacity;
            self.satisfied[path[path.len() - 1]] = true;
            satisfied_count += 1;
            for i in 0..path.len() - 1 {
                if i & 1 == 0 {
                    let at = self.at(path[i], path[i + 1]);
                    self.assigned[at] = true;
                } else {
                    let at = self.at(path[i + 1], path[i]);
                    self.assigned[at] = false;
                }
            }
        }
    }
    fn try_assigning_new_item(&mut self, capacity: i32) -> Option<Vec<usize>> {
        self.clear_visited();
        for t in 0..self.items.len() {
            if self.amounts[self.items[t]] >= capacity
                && let Some(path) = self.find_path(t, capacity)
            {
                return Some(path);
            }
        }
        None
    }
    fn find_path(&mut self, start: usize, capacity: i32) -> Option<Vec<usize>> {
        self.path.clear();
        self.visited_item[start] = true;
        self.path.push(start);
        while !self.path.is_empty() {
            let len = self.path.len();
            if (len - 1) & 1 == 0 {
                let item = self.path[len - 1];
                for g in 0..self.ingredient_count {
                    let at = self.at(item, g);
                    if !self.visited_ingredient[g]
                        && self.limits[at] >= capacity
                        && !self.assigned[at]
                    {
                        self.visited_ingredient[g] = true;
                        self.path.push(g);
                        break;
                    }
                }
            } else {
                let ingredient = self.path[len - 1];
                if !self.satisfied[ingredient] {
                    return Some(self.path.clone());
                }
                for t in 0..self.items.len() {
                    if !self.visited_item[t] && self.assigned[self.at(t, ingredient)] {
                        self.visited_item[t] = true;
                        self.path.push(t);
                        break;
                    }
                }
            }
            if self.path.len() == len {
                self.path.pop();
            }
        }
        None
    }
    pub(in crate::menu) fn try_pick_all(
        &mut self,
        max_size: i32,
        output: Option<&mut Vec<usize>>,
    ) -> i32 {
        let mut min = 0i32;
        // Java int arithmetic: an empty ingredient list overflows as vanilla does.
        let mut max = max_size.min(self.upper_bound()).wrapping_add(1);
        loop {
            let mid = min.wrapping_add(max) / 2;
            if self.try_pick(mid, None) {
                if max.wrapping_sub(min) <= 1 {
                    if mid > 0 {
                        self.try_pick(mid, output);
                    }
                    return mid;
                }
                min = mid;
            } else {
                max = mid;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flexible_role_reassigns_its_only_restricted_candidate() {
        let mut amounts = [1, 1];
        let mut picker = Picker::new(&mut amounts, 2, |candidate, role| {
            if candidate == 0 || role == 0 {
                i32::MAX
            } else {
                0
            }
        });
        let mut selected = Vec::new();
        assert!(picker.try_pick(1, Some(&mut selected)));
        assert_eq!(selected, vec![1, 0]);
        assert_eq!(amounts, [1, 1]);
    }
    #[test]
    fn same_item_component_classes_are_not_pooled_into_one_cell() {
        let mut amounts = [1, 1];
        let mut picker = Picker::new(&mut amounts, 1, |_, _| 64);
        assert!(!picker.try_pick(2, None));
        assert!(picker.try_pick(1, None));
    }
    #[test]
    fn maximum_respects_candidate_specific_slot_caps() {
        let mut amounts = [64, 64];
        let mut picker = Picker::new(&mut amounts, 2, |candidate, role| {
            if candidate == role {
                if role == 0 { 5 } else { 64 }
            } else {
                0
            }
        });
        let mut selected = Vec::new();
        assert_eq!(picker.try_pick_all(64, Some(&mut selected)), 5);
        assert_eq!(selected, vec![0, 1]);
        assert_eq!(amounts, [64, 64]);
    }
    #[test]
    fn capped_assignment_matches_exhaustive_two_role_oracle() {
        for encoded in 0..256usize {
            let limits = [
                encoded & 3,
                (encoded >> 2) & 3,
                (encoded >> 4) & 3,
                (encoded >> 6) & 3,
            ];
            let expected = (1..=3)
                .filter(|amount| {
                    (0..2).any(|first| {
                        (0..2).any(|second| {
                            limits[first * 2] >= *amount
                                && limits[second * 2 + 1] >= *amount
                                && (0..2).all(|candidate| {
                                    usize::from(first == candidate) * amount
                                        + usize::from(second == candidate) * amount
                                        <= [2, 3][candidate]
                                })
                        })
                    })
                })
                .max()
                .unwrap_or(0);
            let mut amounts = [2, 3];
            let mut picker = Picker::new(&mut amounts, 2, |candidate, role| {
                limits[candidate * 2 + role] as i32
            });
            assert_eq!(
                picker.try_pick_all(3, None),
                expected as i32,
                "limits {limits:?}"
            );
            assert_eq!(amounts, [2, 3]);
        }
    }
}
