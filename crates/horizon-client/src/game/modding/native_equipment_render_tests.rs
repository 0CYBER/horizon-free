//! What a living entity wears and holds, as a mod's `render_worn_equipment` lets it be drawn
//! (horizon-rust/horizon#11072): the hook is asked for each entity drawn, and what it hides or
//! tints changes only those draws, on another player's armour and this window's held item.
use super::Game;
use horizon_client::{items::ItemStack, mesher::Vertex, mobs::Mob};
use mod_api::{
    EquipmentRenderPart as Drawn, EquipmentRenderSlot as Worn, Hooks, RenderWornEquipmentEvent,
};
use std::sync::{Arc, Mutex};

/// What the mod does to every entity it is asked about, and the ids it was asked with.
#[derive(Default)]
struct Wish {
    hide: Vec<(Worn, Drawn)>,
    tint: Vec<(Worn, u32)>,
    asked: Vec<u32>,
}

/// A mod listening to `render_worn_equipment` that does what its [`Wish`] says.
struct Equip(Arc<Mutex<Wish>>);

#[mod_api::__subscriptions]
impl Hooks for Equip {
    fn render_worn_equipment(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderWornEquipmentEvent<'_>,
    ) {
        let mut wish = self.0.lock().unwrap();
        wish.asked.push(event.entity.id);
        for &(slot, part) in &wish.hide {
            event.equipment.hide(slot, part);
        }
        for &(slot, rgb) in &wish.tint {
            event.equipment.tint(slot, rgb);
        }
    }
}

/// The camera, south of the entities and looking at them.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// Runs `check` on a client game with prepared resources and the [`Equip`] mod.
fn with_equip(wish: &Arc<Mutex<Wish>>, check: impl FnOnce(&mut Game<'_>)) {
    let mods = vec![mod_api::ModEntry::new(
        "equip",
        Box::new(Equip(wish.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            check(game);
        });
    });
}

/// One frame of the level's mobs, begun as the entity pass begins it.
fn mob_frame(game: &mut Game<'_>) -> Vec<Vertex> {
    game.prepare_entity_culling(glam::DVec3::ZERO);
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

/// This window's player in third person, drawn by the game's own path for it, with its own entity
/// id 4321.
fn own_player_frame(game: &mut Game<'_>) -> Vec<Vertex> {
    let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut player = horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap();
    player.entity_id = 4321;
    game.player = Some(player);
    if let Some(remote) = game.remote.as_mut() {
        remote.entity_id = 4321;
    }
    game.camera_type = horizon_client::camera::CameraType::ThirdPersonBack;
    game.tick_player_model();
    game.prepare_entity_culling(glam::DVec3::ZERO);
    let mut out = Vec::new();
    game.player_model_geometry(CAMERA, 1.0, None, &mut out);
    out
}

#[test]
#[ignore = "daily"]
fn a_mod_hides_and_tints_another_players_armour() {
    let wish: Arc<Mutex<Wish>> = Arc::default();
    with_equip(&wish, |game| {
        let kind = game.res.player_type().clone().unwrap();
        let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        game.mobs.push(Mob::new(91, kind, at, 0.0));
        // Another player wearing an iron helmet, iron boots, both or neither.
        let wear = |game: &mut Game<'_>, helmet: bool, boots: bool| {
            use horizon_items::equipment::{FEET, HEAD};
            let stack = |on: bool, item: &str| on.then(|| ItemStack::new(item, 1));
            let player = &mut game.mobs[0];
            player
                .extra
                .equipment
                .set(HEAD, stack(helmet, "minecraft:iron_helmet"));
            player
                .extra
                .equipment
                .set(FEET, stack(boots, "minecraft:iron_boots"));
            mob_frame(game)
        };
        let both = wear(game, true, true);
        let boots_only = wear(game, false, true);
        let helmet_only = wear(game, true, false);
        assert!(
            both.len() > boots_only.len() && both.len() > helmet_only.len(),
            "each piece draws: {} {} {}",
            both.len(),
            boots_only.len(),
            helmet_only.len()
        );
        let boots = both.len() - helmet_only.len();
        assert_eq!(wish.lock().unwrap().asked.first(), Some(&91));
        // The helmet hidden: what draws is what the boots alone draw.
        wish.lock().unwrap().hide.push((Worn::Head, Drawn::Base));
        assert_eq!(wear(game, true, true).len(), boots_only.len());
        // The boots tinted red instead: only their vertices change, losing green and blue.
        {
            let mut wish = wish.lock().unwrap();
            wish.hide.clear();
            wish.tint.push((Worn::Feet, 0xFF_0000));
        }
        let red = wear(game, true, true);
        assert_eq!(red.len(), both.len());
        let changed: Vec<_> = both
            .iter()
            .zip(&red)
            .filter(|(a, b)| a.color != b.color)
            .collect();
        assert_eq!(changed.len(), boots, "the boots' vertices, and only them");
        assert!(
            changed
                .iter()
                .all(|(a, b)| b.color == a.color & 0xFF00_00FF)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_mod_hides_this_windows_players_held_item() {
    let wish: Arc<Mutex<Wish>> = Arc::default();
    with_equip(&wish, |game| {
        let held = |game: &mut Game<'_>, stick: bool| {
            game.inventory[game.selected] = stick.then(|| ItemStack::new("minecraft:stick", 1));
            own_player_frame(game)
        };
        let with_stick = held(game, true);
        let without = held(game, false);
        assert!(with_stick.len() > without.len(), "the stick draws");
        assert!(
            wish.lock().unwrap().asked.contains(&4321),
            "asked with this window's own entity id"
        );
        // The off hand hidden leaves the main hand's stick; the main hand hidden takes it.
        wish.lock().unwrap().hide.push((Worn::OffHand, Drawn::Base));
        assert_eq!(held(game, true).len(), with_stick.len());
        wish.lock()
            .unwrap()
            .hide
            .push((Worn::MainHand, Drawn::Base));
        assert_eq!(held(game, true).len(), without.len());
    });
}
