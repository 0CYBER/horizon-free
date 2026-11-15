// game-module-cfg: test
use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use horizon_client::mobs::Mob;
use horizon_core::{geometry::Vec3, position::ChunkPos};
static VALUE: mod_api::AttachmentType<u32> = mod_api::AttachmentType {
    namespace: "bench",
    name: "tick_state",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
#[test]
#[ignore = "callgrind fixture; needs HORIZON_TEST_RESOURCES"]
fn attachment_instruction_count() {
    let Some(passes) = std::env::var("ATTACHMENT_PASSES")
        .ok()
        .and_then(|passes| passes.parse::<usize>().ok())
    else {
        eprintln!("refused, needs ATTACHMENT_PASSES; run by the parity harness");
        return;
    };
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.bind_dimension_attachments(game.dim);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut holders = Vec::new();
        for id in 100..400 {
            game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
            holders.push(mod_api::Holder::Entity(id));
        }
        for z in (0..40).step_by(2) {
            for x in 0..20 {
                game.world
                    .block_entities
                    .insert((x, 201, z), crate::game::BlockEntity::Chest(vec![None; 5]));
                holders.push(mod_api::Holder::BlockEntity([x, 201, z]));
            }
        }
        for &holder in &holders {
            VALUE
                .set(game, holder, 0)
                .unwrap_or_else(|e| panic!("warm holder {holder:?}: {e:?}"));
        }
        assert_eq!(
            game.attachment_indexes[&game.dim]
                .lock()
                .unwrap()
                .snapshot_named("bench", "tick_state")
                .len(),
            700
        );
        attachment_measured_passes(game, &holders, passes);
        for &holder in &holders {
            assert_eq!(VALUE.get(game, holder).unwrap(), passes as u32);
        }
    });
}

#[inline(never)]
fn attachment_measured_passes(
    game: &mut crate::game::Game<'_>,
    holders: &[mod_api::Holder],
    passes: usize,
) {
    for _ in 0..passes {
        for &holder in holders {
            let value = VALUE.get(game, holder).unwrap();
            VALUE
                .set(game, holder, std::hint::black_box(value.wrapping_add(1)))
                .unwrap();
        }
    }
}
