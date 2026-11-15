use super::super::tests::{Fixture, only_world};
use super::*;
use horizon_core::nbt::Tag;

/// A record naming one region of `gone`, a mod no test installs; `substituted` when the
/// owner already chose vanilla.
pub(in crate::title) fn record(f: &Fixture, folder: &str, substituted: bool) -> PathBuf {
    let string = |text: &str| Tag::String(text.into());
    let pair = || Tag::List(vec![Tag::Float(0), Tag::Float(0)]);
    let mut region = vec![
        ("owner", string("gone")),
        ("id", string("gone:moor")),
        ("weight", Tag::Int(4)),
        (
            "points",
            Tag::List(vec![Tag::compound([
                ("biome", string("gone:moor")),
                ("temperature", pair()),
                ("humidity", pair()),
                ("continentalness", pair()),
                ("erosion", pair()),
                ("depth", pair()),
                ("weirdness", pair()),
                ("offset", Tag::Float(0)),
            ])]),
        ),
    ];
    if substituted {
        region.push((
            "substituted",
            Tag::compound([
                ("with", string("minecraft:vanilla")),
                ("missing_mod", string("gone")),
            ]),
        ));
    }
    let file = f
        .title
        .saves
        .join(folder)
        .join("data/horizon/biome_regions.dat");
    crate::save::write_dat(
        &file,
        Tag::compound([
            ("version", Tag::Int(1)),
            ("reported", Tag::List(Vec::new())),
            (
                "dimensions",
                Tag::compound([(
                    "minecraft:overworld",
                    Tag::compound([
                        (
                            "selector",
                            Tag::compound([
                                ("version", Tag::Int(1)),
                                ("cell", Tag::Int(128)),
                                ("salt", string("horizon:biome_region")),
                                ("vanilla_weight", Tag::Int(10)),
                            ]),
                        ),
                        ("regions", Tag::List(vec![Tag::compound(region)])),
                    ]),
                )]),
            ),
        ]),
    )
    .unwrap();
    file
}

#[test]
fn a_missing_biome_mod_asks_before_the_world_opens() {
    let mut f = Fixture::new();
    f.world("Moor", "Moor");
    let file = record(&f, "Moor", false);
    let world = only_world(&f);
    assert_eq!(f.title.play(world.clone()), Action::None);
    assert!(matches!(
        &f.title.mode,
        Mode::MissingBiomeRegions { missing, .. }
            if *missing == [("gone".to_owned(), "gone:moor".to_owned())]
    ));
    // Don't Open leaves the world closed and its record as it was.
    let before = std::fs::read(&file).unwrap();
    assert_eq!(
        f.click(&lang::get("horizon.biome_regions.missing.dont_open")),
        Action::None
    );
    assert!(matches!(f.title.mode, Mode::Select { .. }));
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // Vanilla's biomes there: recorded in the world, then it opens, and is not asked again.
    assert_eq!(f.title.play(world.clone()), Action::None);
    assert_eq!(
        f.click(&lang::get("horizon.biome_regions.missing.open_vanilla")),
        Action::Play {
            folder: "Moor".into()
        }
    );
    let chosen = crate::save::read_dat(&file);
    record(&f, "Moor", true);
    assert_eq!(crate::save::read_dat(&file), chosen);
    assert_eq!(
        f.title.play(world),
        Action::Play {
            folder: "Moor".into()
        }
    );
}
