//! `upgrade_block_entity_nbt` and `upgrade_entity_nbt` through a server host, as a mod calls them: an old villager
//! takes its profession, and the refusals come back typed: a future version, data too deep to read, and a riding
//! chain the fixes make too deep.
use super::native_entity_refs::tests::with_game;
use horizon_core::nbt::{Compound, Tag, parse_snbt};
use mod_api::NbtUpgradeError;

fn compound(text: &str) -> Compound {
    match parse_snbt(text).unwrap().0 {
        Tag::Compound(c) => c,
        other => panic!("expected a compound, got {other:?}"),
    }
}

/// `{id: "Pig"}` nested `depth` times, each in the one around it: under `Riding` as a pre-1.9 rider holds its mount,
/// or under `Passengers` as a vehicle holds its rider.
fn chain(depth: usize, riding: bool) -> Compound {
    let mut inner = compound("{id: \"Pig\"}");
    for _ in 0..depth {
        let mut outer = compound("{id: \"Pig\"}");
        let value = if riding {
            Tag::Compound(inner)
        } else {
            Tag::List(vec![Tag::Compound(inner)])
        };
        outer.insert(if riding { "Riding" } else { "Passengers" }.into(), value);
        inner = outer;
    }
    inner
}

#[test]
#[ignore = "daily"]
fn a_mods_host_upgrades_old_data_and_refuses_by_name() {
    // The pre-1.13 fixes (the riding chain's) read the data fixers' tables of the prepared resources.
    horizon_world::datafix::set_resources(std::path::Path::new(
        &std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
    ));
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let host: &dyn mod_api::ServerHost = game;
            // DataVersion 1900: Profession 1 with Career 2 is a cartographer.
            let villager =
                compound("{id: \"minecraft:villager\", Profession: 1, Career: 2, CareerLevel: 1}");
            let upgraded = host.upgrade_entity_nbt(1900, villager).unwrap();
            let Some(Tag::Compound(data)) = upgraded.get(&"VillagerData".into()) else {
                panic!("VillagerData: {upgraded:?}");
            };
            assert_eq!(
                data.get(&"profession".into()),
                Some(&Tag::String("minecraft:cartographer".into()))
            );
            let sign = compound("{id: \"minecraft:sign\"}");
            let current = horizon_world::datafix::CURRENT;
            assert_eq!(
                host.upgrade_block_entity_nbt(current + 1, sign),
                Err(NbtUpgradeError::FutureVersion(current + 1))
            );
            // 512 passengers: the innermost compound at depth 1024, refused before any fix.
            assert_eq!(
                host.upgrade_entity_nbt(1900, chain(512, false)),
                Err(NbtUpgradeError::TooDeep)
            );
            // One rider at DataVersion 100 with a payload 510 deep: moving it below its mount's `Passengers`
            // adds two levels, past the cap. Hundreds of riders would refuse too, but the legacy fixes walk and
            // copy the whole chain for each one: minutes and gigabytes (horizon-world's datafix tests do the same).
            let mut rider = chain(1, true);
            let mut payload = Tag::Compound(Compound::new());
            for _ in 0..509 {
                payload = Tag::compound([("x", payload)]);
            }
            rider.insert("payload".into(), payload);
            assert!(host.upgrade_entity_nbt(current, rider.clone()).is_ok());
            assert_eq!(
                host.upgrade_entity_nbt(100, rider),
                Err(NbtUpgradeError::UpgradedTooDeep)
            );
        });
    });
}
