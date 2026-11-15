//! The real refresh path updates both the active world and worlds parked by dimension.
use super::{Dim, native_entity_refs::tests::with_game};
use horizon_core::LevelKey;
use horizon_world::dimension_type::DimensionFacts;
use std::sync::Arc;

struct RestoreFacts(Vec<(LevelKey, DimensionFacts)>);
impl Drop for RestoreFacts {
    fn drop(&mut self) {
        DimensionFacts::set_levels(std::mem::take(&mut self.0));
    }
}

#[test]
#[ignore = "daily"]
fn infiniburn_refresh_updates_active_and_parked_worlds() {
    with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_owned();
        let _restore = RestoreFacts(
            Dim::all()
                .into_iter()
                .map(|dim| {
                    let key = LevelKey::parse(dim.id()).unwrap();
                    (key, DimensionFacts::of_level(key).clone())
                })
                .collect(),
        );
        let mut active = DimensionFacts::overworld().clone();
        active.id = "mymod:active_type".into();
        active.infiniburn = "#mymod:eternal_flame".into();
        let mut parked = DimensionFacts::vanilla("minecraft:the_nether")
            .unwrap()
            .clone();
        parked.id = "mymod:parked_type".into();
        parked.infiniburn = "#mymod:eternal_flame".into();
        DimensionFacts::set_levels(vec![
            (LevelKey::OVERWORLD, active),
            (LevelKey::NETHER, parked),
        ]);
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(game.dimension_type().id, "mymod:active_type");
        assert_eq!(game.dimension_type_of(Dim::Nether).id, "mymod:parked_type");

        game.server_items = Some(Arc::new(game.items().clone()));
        let netherrack = game
            .res
            .behaviours()
            .table
            .block_by_name("minecraft:netherrack")
            .unwrap();
        let stone = game
            .res
            .behaviours()
            .table
            .block_by_name("minecraft:stone")
            .unwrap();
        for (member, has_netherrack, has_stone) in [
            ("minecraft:netherrack", true, false),
            ("minecraft:stone", false, true),
        ] {
            Arc::make_mut(game.server_items.as_mut().unwrap())
                .block_tags
                .insert(
                    "mymod:eternal_flame".into(),
                    [member.to_owned()].into_iter().collect(),
                );
            game.refresh_infiniburn();
            let parked_world = &game.levels.parked.get(&Dim::Nether).unwrap().world;
            for world in [&game.world, parked_world] {
                let set = world.infiniburn.as_deref().expect("resolved level tag");
                assert_eq!(
                    set.contains(netherrack),
                    has_netherrack,
                    "tag holding {member}"
                );
                assert_eq!(set.contains(stone), has_stone, "tag holding {member}");
            }
        }
    });
}
