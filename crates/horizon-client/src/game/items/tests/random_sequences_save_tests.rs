//! `random_sequences.dat` across a save and a reopening: vanilla writes `RandomSequences`
//! only while a draw (`DirtyMarkingRandomSource`) or its creation (`SavedDataStorage.set`)
//! has marked it dirty (`SavedDataStorage.collectDirtyTagsToSave`), and writes what it holds
//! then: the map `/random reset *` emptied and the salt it set.
use super::{read_random_sequences, write_random_sequences};
use horizon_client::random_sequences::RandomSequences;
use horizon_core::nbt::Tag;
use horizon_core::random::RandomSource;

const SEED: i64 = 1234;
const TEST: &str = "minecraft:test";

/// An empty world folder of its own.
fn world(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-random-sequences-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// One `/random value 1..1000 minecraft:test`.
fn draw(data: &mut RandomSequences) -> i32 {
    data.get(TEST, SEED).next_int_bound(1000).expect("a bound")
}

/// The salt `random_sequences.dat` holds.
fn salt_on_disk(root: &std::path::Path) -> Option<Tag> {
    let tag = horizon_client::save::read_dat(&root.join("data/minecraft/random_sequences.dat"))?;
    tag.get("data")?.get("salt").cloned()
}

#[test]
fn a_reset_after_a_draw_survives_the_reopening() {
    let root = world("reset");
    let mut data = RandomSequences::default();
    let first = draw(&mut data);
    write_random_sequences(&root, &mut data);
    draw(&mut data);
    assert_eq!(data.clear(), 1);
    write_random_sequences(&root, &mut data);
    let mut reopened = read_random_sequences(&root);
    assert!(reopened.ids().is_empty(), "the reset emptied the file");
    assert_eq!(draw(&mut reopened), first, "the sequence starts over");
    // `/random reset * 42` after a draw: the salt is written too.
    write_random_sequences(&root, &mut reopened);
    draw(&mut reopened);
    reopened.set_defaults(42, true, true);
    reopened.clear();
    write_random_sequences(&root, &mut reopened);
    assert_eq!(salt_on_disk(&root), Some(Tag::Int(42)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_reset_with_no_draw_since_the_save_is_not_written() {
    let root = world("undrawn");
    let mut data = RandomSequences::default();
    let first = draw(&mut data);
    write_random_sequences(&root, &mut data);
    // Handing a sequence out (a trade screen borrowing it, a `get` nothing draws from)
    // is no draw.
    let lent = data.lend(TEST, SEED);
    data.give_back(TEST.to_owned(), lent);
    data.get(TEST, SEED);
    data.set_defaults(42, true, true);
    data.clear();
    write_random_sequences(&root, &mut data);
    let mut reopened = read_random_sequences(&root);
    assert_eq!(salt_on_disk(&root), Some(Tag::Int(0)), "the old file stays");
    assert_eq!(reopened.ids(), [TEST]);
    assert_ne!(draw(&mut reopened), first, "the old stream goes on");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_lent_sequence_stays_in_the_file() {
    let root = world("lent");
    let mut data = RandomSequences::default();
    draw(&mut data);
    let lent = data.lend(TEST, SEED);
    write_random_sequences(&root, &mut data);
    assert_eq!(read_random_sequences(&root).ids(), [TEST]);
    data.give_back(TEST.to_owned(), lent);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_world_writes_its_first_save() {
    let root = world("new");
    let mut data = read_random_sequences(&root);
    data.set_defaults(42, true, true);
    assert_eq!(data.clear(), 0);
    write_random_sequences(&root, &mut data);
    assert_eq!(salt_on_disk(&root), Some(Tag::Int(42)));
    assert!(read_random_sequences(&root).ids().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
