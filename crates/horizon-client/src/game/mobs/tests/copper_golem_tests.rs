//! `CarvedPumpkinBlock.trySpawnGolem` finds the copper golem's block of
//! `#minecraft:copper` in any of the six directions from the pumpkin
//! (`BlockPattern.find` tries every forwards and up pair), and
//! `PumpkinBlock.useItemOn` wears the shears in the hand that carved and
//! reports that hand's break (`hurtAndBreak(1, player, hand.asEquipmentSlot())`).
use super::super::*;
use crate::game::native_machines::tests::server_game;
use horizon_client::items::ItemStack;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

fn block_name(game: &Game<'_>, p: [i32; 3]) -> String {
    game.res
        .facts()
        .get(game.world.block(p[0], p[1], p[2]))
        .name
        .to_string()
}

#[test]
#[ignore = "daily"]
fn copper_above_below_or_beside_the_pumpkin_builds_the_golem() {
    let mut game = server_game();
    let air = game.world.air;
    let head = [8, 64, 8];
    let (carved, copper) = {
        let registry = game.pack.assets.registry;
        (
            registry.default_state("minecraft:carved_pumpkin").unwrap(),
            registry.default_state("minecraft:copper_block").unwrap(),
        )
    };
    // The six neighbours: vanilla's pattern search reaches every one of them
    // through a forwards/up pair.
    for step in [
        [0, 1, 0],
        [0, -1, 0],
        [1, 0, 0],
        [-1, 0, 0],
        [0, 0, 1],
        [0, 0, -1],
    ] {
        let block = [head[0] + step[0], head[1] + step[1], head[2] + step[2]];
        for p in [head, block] {
            game.world.set(p[0], p[1], p[2], air).unwrap();
        }
        game.world
            .set(block[0], block[1], block[2], copper)
            .unwrap();
        game.mobs.clear();
        // `CarvedPumpkinBlock.onPlace` of the pumpkin: the pattern completes
        // inside the setBlock, so the pumpkin is already gone by the end.
        game.set_golem_head(head, carved).unwrap();
        let golems: Vec<_> = game
            .mobs
            .iter()
            .filter(|m| m.kind.id == "minecraft:copper_golem")
            .collect();
        assert_eq!(golems.len(), 1, "one copper golem, copper at {step:?}");
        let at = golems[0].position;
        assert_eq!(
            [at.x, at.y, at.z],
            [8.5, 64.0 + 0.05, 8.5],
            "the golem stands where the pumpkin was, copper at {step:?}"
        );
        assert_eq!(
            block_name(&game, block),
            "minecraft:copper_chest",
            "the copper became a chest, copper at {step:?}"
        );
        assert_eq!(
            block_name(&game, head),
            "minecraft:air",
            "the pumpkin is gone, copper at {step:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn carving_with_off_hand_shears_breaks_the_off_hand() {
    let mut game = server_game();
    game.options.dedicated = None;
    game.set_mode_id(0);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
    // A guest's break is queued with its entity event; the local player's is
    // only a sound where it stands.
    game.lan_acting = Some(1);
    let pos = [8, 64, 8];
    let pumpkin = game
        .pack
        .assets
        .registry
        .default_state("minecraft:pumpkin")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], pumpkin).unwrap();
    let max = game.items().get("minecraft:shears").unwrap().max_damage;
    let mut shears = ItemStack::new("minecraft:shears", 1);
    shears.damage = max - 1;
    game.offhand = Some(shears.clone());
    let stick = ItemStack::new("minecraft:stick", 3);
    game.inventory[game.selected] = Some(stick.clone());
    game.hand = Hand::Off;
    assert!(game.use_pumpkin(pos, [0, 1, 0], Some(&shears)).unwrap());
    // Entity event 48 is OFFHAND's break (47 is the main hand's): the shears
    // that carved wear out, and the main hand keeps its stack.
    assert_eq!(
        game.guest_equipment_breaks.last().copied(),
        Some(48)
    );
    assert!(game.offhand.is_none(), "the shears broke");
    assert_eq!(game.inventory[game.selected], Some(stick));
}
