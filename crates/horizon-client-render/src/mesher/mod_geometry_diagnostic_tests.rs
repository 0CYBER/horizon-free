use super::*;

#[test]
fn missing_sprite_diagnostic_is_bounded_without_limiting_valid_names() {
    let name = "界".repeat(350_000);
    let quad = SectionQuad {
        positions: [[0.0; 3]; 4],
        uvs: [[0.0; 2]; 4],
        color: 0,
        light: [0; 2],
        translucent: false,
    };
    let mut sprites = BTreeMap::new();
    let mut budget = Budget {
        quads: 1,
        bytes: QUAD_BYTES,
    };
    let shared = AtomicUsize::new(0);
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert!(!batch.add(&name, quad));
    let diagnostic = batch.refused.as_ref().unwrap();
    assert!(diagnostic.len() < 320);
    assert!(diagnostic.contains('…'));
    assert_eq!(batch.in_flight, 0);
    drop(batch);
    sprites.insert(name.clone(), 1);
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert!(batch.add(&name, quad));
}
