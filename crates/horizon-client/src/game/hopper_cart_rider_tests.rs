//! `MinecartHopper.tick` pulls one item a tick, ridden or not: `ServerLevel`
//! ticks the vehicle (`tickNonPassenger`) before its passengers, and
//! `tryConsumeItems` has no rider condition. Horizon ticks a ridden cart in
//! its rider's turn instead (`tick_cart_riding`, from `guest_vehicle_tick`),
//! which used to move the cart without the suck: a player riding a hopper
//! minecart under a chest got nothing.
use super::super::{
    Game,
    minecarts::{CartKind, Minecart},
    native_machines::tests::server_game,
};
use horizon_client::{containers::BlockEntity, items::ItemStack};
use horizon_core::geometry::Vec3;

fn chest(game: &mut Game<'_>, x: i32) {
    let registry = game.pack.assets.registry;
    let rail = registry.default_state("minecraft:rail").unwrap();
    let chest = registry.default_state("minecraft:chest").unwrap();
    // The rail holds the cart still; the chest sits in the block the cart
    // pulls from (`getLevelY` is y + 0.5, so the block above is y + 1.5).
    game.world.set(x, 64, 8, rail).unwrap();
    game.world.set(x, 65, 8, chest).unwrap();
    game.world.block_entities.insert(
        (x, 65, 8),
        BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 27]),
    );
}

fn counts(game: &Game<'_>, x: i32, id: u32) -> (u32, u32) {
    let in_chest: u32 = match game.world.block_entities.get(&(x, 65, 8)) {
        Some(BlockEntity::Chest(s)) => s.iter().flatten().map(|s| s.count).sum(),
        _ => 0,
    };
    let in_cart: u32 = game
        .carts
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .items
        .iter()
        .flatten()
        .map(|s| s.count)
        .sum();
    (in_chest, in_cart)
}

/// A player riding a hopper minecart loads exactly like one without a rider,
/// and keeps its rider.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_ridden_hopper_cart_takes_one_item_a_tick() {
    let mut game = server_game();
    chest(&mut game, 8);
    chest(&mut game, 12);
    let ridden = Minecart::new(1, CartKind::Hopper, Vec3::new(8.5, 64.0625, 8.5));
    let empty = Minecart::new(2, CartKind::Hopper, Vec3::new(12.5, 64.0625, 8.5));
    game.carts.push(ridden);
    game.carts.push(empty);
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.5, 8.5)).unwrap(),
    );
    // A hopper cart's interact opens its menu (`AbstractMinecartContainer.interact`),
    // so a player only gets in with `/ride`, whose `RideCommand.mount` forces
    // `startRiding` past `isRideable`. `tick_carts` then leaves that cart to
    // `tick_cart_riding`, as the server's guests' turns do.
    let mount = crate::game::commands::Target::Cart(1);
    assert!(
        game.ride_mount(crate::game::commands::Target::Player, mount)
            .is_ok(),
        "`/ride` mounts a player on a hopper minecart"
    );
    assert_eq!(game.riding, Some(1), "the rider is in the first cart");
    for _ in 0..20 {
        assert!(game.tick_cart_riding(super::super::Keys::default()));
        game.tick_carts();
    }
    assert_eq!(game.riding, Some(1), "the rider keeps its cart");
    let (ridden_chest, ridden_cart) = counts(&game, 8, 1);
    let (empty_chest, empty_cart) = counts(&game, 12, 2);
    assert_eq!((empty_cart, empty_chest), (20, 27 * 64 - 20));
    assert_eq!(ridden_cart, 20, "one item a tick while ridden");
    assert_eq!(ridden_chest, 27 * 64 - ridden_cart);
}
