use super::super::missing_biome_regions::tests::record;
use super::super::tests::Fixture;
use super::*;

fn adopt_offered(f: &Fixture) -> bool {
    f.title
        .buttons(480.0, 262.0)
        .iter()
        .any(|b| b.label == lang::get("horizon.biome_regions.adopt.button"))
}

#[test]
fn edit_world_offers_the_installed_regions_only_when_they_differ() {
    let mut f = Fixture::new();
    f.world("Plain", "Plain");
    f.world("Moor", "Moor");
    // A world without a record, and no region mod installed: nothing to adopt.
    let plain = list_worlds(&f.title.saves)
        .into_iter()
        .find(|w| w.folder == "Plain")
        .unwrap();
    f.title.edit_world("Plain".into(), plain);
    assert!(!adopt_offered(&f));
    // Its saved region's mod is not installed, so adopting would drop the region.
    let file = record(&f, "Moor", false);
    let moor = list_worlds(&f.title.saves)
        .into_iter()
        .find(|w| w.folder == "Moor")
        .unwrap();
    f.title.edit_world("Renamed".into(), moor.clone());
    assert!(adopt_offered(&f));
    // Cancel goes back to Edit World with the typed name; the record is as it was.
    let before = std::fs::read(&file).unwrap();
    f.click(&lang::get("horizon.biome_regions.adopt.button"));
    assert!(matches!(f.title.mode, Mode::AdoptBiomeRegions { .. }));
    f.click(&lang::get_or("gui.cancel", "Cancel"));
    assert!(matches!(&f.title.mode, Mode::Edit { name, .. } if name == "Renamed"));
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // Adopting writes the choice into the world, and the opt-in is no longer offered.
    f.click(&lang::get("horizon.biome_regions.adopt.button"));
    f.click(&lang::get("horizon.biome_regions.adopt.confirm"));
    assert!(matches!(
        &f.title.mode,
        Mode::Edit { name, adopt_regions: false, .. } if name == "Renamed"
    ));
    assert!(!adopt_offered(&f));
    assert_ne!(std::fs::read(&file).unwrap(), before);
    // The dropped region no longer asks before the world opens.
    assert_eq!(
        f.title.play(moor),
        Action::Play {
            folder: "Moor".into()
        }
    );
}

#[test]
fn escape_cancels_adoption_and_preserves_the_typed_name_and_record() {
    let mut f = Fixture::new();
    f.world("Moor", "Moor");
    let file = record(&f, "Moor", false);
    let before = std::fs::read(&file).unwrap();
    let world = super::super::tests::only_world(&f);
    f.title.edit_world("Typed name".into(), world);
    f.click(&lang::get("horizon.biome_regions.adopt.button"));
    f.title.escape();
    assert!(matches!(&f.title.mode, Mode::Edit { name, .. } if name == "Typed name"));
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn a_failed_adoption_write_keeps_the_record_and_world_closed_until_retry() {
    let mut f = Fixture::new();
    f.world("Moor", "Moor");
    let file = record(&f, "Moor", false);
    let before = std::fs::read(&file).unwrap();
    let world = super::super::tests::only_world(&f);
    assert_eq!(f.title.play(world.clone()), Action::None);
    f.title.edit_world("Typed name".into(), world.clone());
    f.click(&lang::get("horizon.biome_regions.adopt.button"));
    // A directory at the temporary output path forces a real write failure on every OS.
    let temporary = file.with_file_name("biome_regions.dat_new");
    std::fs::create_dir(&temporary).unwrap();
    assert_eq!(
        f.click(&lang::get("horizon.biome_regions.adopt.confirm")),
        Action::None
    );
    assert!(
        matches!(&f.title.mode, Mode::Error { message, return_to, .. }
        if message.starts_with(lang::get("horizon.biome_regions.save_failed").split("%s").next().unwrap_or_default())
            && matches!(&**return_to, Mode::AdoptBiomeRegions { .. }))
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::AdoptBiomeRegions { .. }));
    std::fs::remove_dir(&temporary).unwrap();
    assert_eq!(
        f.click(&lang::get("horizon.biome_regions.adopt.confirm")),
        Action::None
    );
    assert!(matches!(&f.title.mode, Mode::Edit { name, .. } if name == "Typed name"));
    assert_ne!(std::fs::read(&file).unwrap(), before);
    assert_eq!(
        f.title.play(world),
        Action::Play {
            folder: "Moor".into()
        }
    );
}

#[test]
fn confirm_refuses_a_record_changed_since_the_prompt_was_drawn() {
    let mut f = Fixture::new();
    f.world("Moor", "Moor");
    let file = record(&f, "Moor", false);
    let world = super::super::tests::only_world(&f);
    f.title.edit_world("Typed name".into(), world);
    f.click(&lang::get("horizon.biome_regions.adopt.button"));
    std::fs::write(&file, b"changed since drawing").unwrap();
    assert_eq!(
        f.click(&lang::get("horizon.biome_regions.adopt.confirm")),
        Action::None
    );
    assert!(matches!(&f.title.mode, Mode::Error { return_to, .. }
        if matches!(&**return_to, Mode::AdoptBiomeRegions { .. })));
    assert_eq!(std::fs::read(&file).unwrap(), b"changed since drawing");
}
