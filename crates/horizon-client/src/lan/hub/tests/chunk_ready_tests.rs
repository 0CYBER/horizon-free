//! The neighbour check behind `next_batch`'s readiness: the nine columns around
//! a lit chunk must each be a full chunk or a proto chunk.
use super::*;

type Key = (i32, i32);

/// The nine offsets of a 3x3 neighbourhood, in row order.
fn ring() -> Vec<Key> {
    (-1..=1)
        .flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)))
        .collect()
}

/// The scan's answer for centre `c` when `full` are resident chunks and `proto`
/// are decorated proto columns.
fn scan(full: &[Key], proto: &[Key], c: Key) -> bool {
    let mut chunks = crate::chunk_map::ChunkMap::default();
    for &key in full {
        chunks.insert(key, std::sync::Arc::new(Chunk::empty(key.0, key.1)));
    }
    let centre = Chunk::empty(c.0, c.1);
    let proto = |x, z| proto.contains(&(x, z));
    let scanned = neighbours_scanned(&chunks, proto, &centre);
    // The fast path never changes the answer.
    assert_eq!(neighbours_ready(&chunks, proto, &centre), scanned);
    scanned
}

#[test]
fn neighbour_scan_literal_cases() {
    let all = ring();
    let without_corner: Vec<Key> = all.iter().copied().filter(|&k| k != (1, 1)).collect();
    // Nothing resident and nothing proto.
    assert!(!scan(&[], &[], (0, 0)));
    // All nine resident.
    assert!(scan(&all, &[], (0, 0)));
    // The corner (1,1) missing.
    assert!(!scan(&without_corner, &[], (0, 0)));
    // The same corner as a proto column.
    assert!(scan(&without_corner, &[(1, 1)], (0, 0)));
    // A single proto column on the edge is not enough.
    assert!(!scan(&[], &[(0, -1)], (0, 0)));
    // Centre (1,0) reaches column x=2: one of its three cells is not enough.
    assert!(!scan(&all, &[(2, 0)], (1, 0)));
    assert!(scan(&all, &[(2, -1), (2, 0), (2, 1)], (1, 0)));
}

#[test]
fn neighbour_scan_is_ready_exactly_when_every_neighbour_is_covered() {
    // Each of the nine cells is absent (0), full (1), proto (2) or both (3).
    // A cell is covered unless it is absent, so 3^9 = 19683 of the 4^9 patterns are ready.
    let cells = ring();
    let mut ready = 0;
    for code in 0..4usize.pow(9) {
        let mut full = Vec::new();
        let mut proto = Vec::new();
        for (i, &key) in cells.iter().enumerate() {
            let state = (code >> (2 * i)) & 3;
            if state & 1 != 0 {
                full.push(key);
            }
            if state & 2 != 0 {
                proto.push(key);
            }
        }
        let covered = (0..9).all(|i| (code >> (2 * i)) & 3 != 0);
        let got = scan(&full, &proto, (0, 0));
        assert_eq!(got, covered, "pattern {code}");
        ready += usize::from(got);
    }
    assert_eq!(ready, 19683);
}

#[test]
fn surrounded_is_true_exactly_when_all_nine_columns_are_resident() {
    // The fast path's premise: the index agrees with the nine lookups for full chunks.
    let all = ring();
    let mut chunks = crate::chunk_map::ChunkMap::default();
    for &(x, z) in all.iter().filter(|&&k| k != (1, 1)) {
        chunks.insert((x, z), std::sync::Arc::new(Chunk::empty(x, z)));
    }
    assert!(!chunks.surrounded((0, 0)));
    chunks.insert((1, 1), std::sync::Arc::new(Chunk::empty(1, 1)));
    assert!(chunks.surrounded((0, 0)));
    // A neighbour of the ring is not surrounded: its own ring is incomplete.
    assert!(!chunks.surrounded((1, 0)));
}
