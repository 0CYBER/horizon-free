//! Iteration order of fastutil's `LongOpenHashSet`, which vanilla uses for
//! structure references. Placing a structure's pieces consumes one shared
//! random stream, so starts must be visited in the set's order.

const PHI: i64 = 0x9E37_79B9_7F4A_7C15_u64 as i64;
const LOAD_FACTOR: f64 = 0.75;

fn mix(key: i64) -> usize {
    let mut h = key.wrapping_mul(PHI);
    h ^= ((h as u64) >> 32) as i64;
    (h ^ (((h as u64) >> 16) as i64)) as i32 as u32 as usize
}

fn array_size(expected: usize) -> usize {
    ((expected as f64 / LOAD_FACTOR).ceil() as usize)
        .next_power_of_two()
        .max(2)
}

fn max_fill(n: usize) -> usize {
    ((n as f64 * LOAD_FACTOR).ceil() as usize).min(n - 1)
}

/// Indices of `keys` (distinct, in insertion order) in the order a default
/// `LongOpenHashSet` holding them iterates: the zero key first, then the table
/// from its last slot down to its first.
pub fn iteration_order(keys: &[i64]) -> Vec<usize> {
    let mut n = array_size(16);
    let mut table: Vec<Option<usize>> = vec![None; n];
    let mut contains_zero = None;
    let mut size = 0;
    for (index, &key) in keys.iter().enumerate() {
        if key == 0 {
            contains_zero = Some(index);
        } else {
            let mask = n - 1;
            let mut pos = mix(key) & mask;
            while table[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            table[pos] = Some(index);
        }
        size += 1;
        if size > max_fill(n) {
            let new_n = array_size(size + 1);
            let mut grown: Vec<Option<usize>> = vec![None; new_n];
            for slot in table.iter().rev().flatten() {
                let mask = new_n - 1;
                let mut pos = mix(keys[*slot]) & mask;
                while grown[pos].is_some() {
                    pos = (pos + 1) & mask;
                }
                grown[pos] = Some(*slot);
            }
            table = grown;
            n = new_n;
        }
    }
    contains_zero
        .into_iter()
        .chain(table.iter().rev().flatten().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_and_zero_keys() {
        assert_eq!(iteration_order(&[]), Vec::<usize>::new());
        assert_eq!(iteration_order(&[5]), vec![0]);
        assert_eq!(iteration_order(&[7, 0]), vec![1, 0]);
    }

    #[test]
    fn every_key_is_visited_once_after_growth() {
        let keys: Vec<i64> = (0..100).map(|i| i * 4_294_967_297 + 3).collect();
        let mut order = iteration_order(&keys);
        order.sort_unstable();
        assert_eq!(order, (0..100).collect::<Vec<_>>());
    }
}
