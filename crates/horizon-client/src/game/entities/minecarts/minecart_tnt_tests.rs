//! `MinecartTNT`'s click and fuse: vanilla has no `interact` override, so
//! flint and steel leaves the cart alone (`Entity.interact` passes), and
//! `primeFuse` plays `entity.tnt.primed` on the blocks source only when the
//! cart is not silent.
use super::super::native_machines::tests::server_game;
use super::super::*;
use super::{CartKind, Minecart};
use horizon_client::items::ItemStack;

/// A server's game on the prepared resources, or None when they are absent
/// (CI has none: the tests then skip quietly).
fn server() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    Some(server_game())
}

/// A lone TNT cart at (0.5, 64, 0.5); a silent one keeps `Silent:1b` where
/// `Entity.saveWithoutId` does.
fn tnt_cart(game: &mut Game<'_>, id: u32, silent: bool) {
    game.carts.clear();
    game.carts
        .push(Minecart::new(id, CartKind::Tnt, Vec3::new(0.5, 64.0, 0.5)));
    game.persist.raw.remove(&id);
    game.lan_entity_events.clear();
    if silent {
        let (tag, _) = horizon_world::nbt::parse_snbt("{Silent:1b}").unwrap();
        game.persist.raw.insert(id, tag);
    }
}

#[test]
fn flint_and_steel_passes_on_a_tnt_cart_without_lighting_it() {
    let Some(mut game) = server() else {
        return;
    };
    tnt_cart(&mut game, 42, false);
    game.inventory[game.selected] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.audio.capture = Some(Vec::new());
    let used = game.interact_cart(Some(InteractTarget {
        id: 42,
        location: [0.0, 0.0, 0.0],
    }));
    assert!(!used, "Entity.interact passes to the held item");
    assert_eq!(game.carts[0].fuse, -1, "flint and steel lit the cart");
    let sounds = game.audio.capture.take().unwrap();
    assert!(sounds.is_empty(), "{sounds:?}");
}

#[test]
fn a_silent_tnt_cart_primes_without_its_sound() {
    let Some(mut game) = server() else {
        return;
    };
    // Silent: the fuse is lit, the entity event goes out, no sound is played.
    tnt_cart(&mut game, 42, true);
    game.audio.capture = Some(Vec::new());
    let mut silent = game.carts[0].clone();
    game.activate_cart(&mut silent, true);
    assert_eq!(silent.fuse, 80, "a silent cart still primes");
    assert!(
        game.lan_entity_events
            .iter()
            .any(|(id, event, _)| *id == 42 && *event == 70),
        "a silent cart still broadcasts EntityEvent.TNT_PRIME: {:?}",
        game.lan_entity_events
    );
    let sounds = game.audio.capture.take().unwrap();
    assert!(sounds.is_empty(), "a silent cart hissed: {sounds:?}");

    // Not silent: exactly one `entity.tnt.primed` at the cart, on the blocks
    // source (`SoundSource.BLOCKS` ordinal 4), volume and pitch 1.
    tnt_cart(&mut game, 43, false);
    game.audio.capture = Some(Vec::new());
    let mut normal = game.carts[0].clone();
    game.activate_cart(&mut normal, true);
    assert_eq!(normal.fuse, 80);
    let sounds = game.audio.capture.take().unwrap();
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    let sound = &sounds[0];
    assert_eq!(sound.event, "minecraft:entity.tnt.primed");
    assert_eq!(sound.position, [0.5, 64.0, 0.5]);
    assert_eq!(sound.volume, 1.0);
    assert_eq!(sound.pitch, 1.0);
    assert_eq!(sound.source, 4, "SoundSource.BLOCKS");
}
