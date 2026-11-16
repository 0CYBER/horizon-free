//! When a mod's special model renderer is asked, in a real game: on the client's frame, outside
//! any tick's lifted pass and any action transaction (matrix rows 6 and 7 of
//! `docs/design/mod-api/resource-renderers.md`). Each test runs alone in its own process: it
//! registers the process's renderers and changes the game's stick into a special model.
use crate::game::Game;
use horizon_assets::item_model::{ClientItem, DisplayContext, register_special_model_types};
use horizon_client::item_render::ItemRenderState;
use mod_api::{ClientHost, ItemStack, SpecialModelPart, SpecialModelRenderer};
use std::sync::{Arc, Mutex, Once};

/// The stack counts the renderer was asked about, in order.
static ASKED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Draws a stick for every stack.
struct Stick;
impl SpecialModelRenderer for Stick {
    fn extract(
        &self,
        _: &dyn ClientHost,
        stack: &ItemStack,
        _: &horizon_core::nbt::Tag,
    ) -> Result<Vec<SpecialModelPart>, String> {
        ASKED.lock().unwrap().push(stack.count());
        Ok(vec![SpecialModelPart::new("minecraft:item/stick")])
    }
}

/// How many times the renderer was asked about a stack of `count`.
pub(in crate::game) fn asked(count: u32) -> usize {
    ASKED
        .lock()
        .unwrap()
        .iter()
        .filter(|&&c| c == count)
        .count()
}

/// Makes `game`'s stick a `minecraft:special` model of the type `test:stick`, drawn by [`Stick`]:
/// in the game's own item definitions, which every route that resolves a stick reads.
pub(in crate::game) fn install_special_stick(game: &mut Game<'_>) {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        register_special_model_types(vec!["test:stick".into()]);
        assert!(super::register(vec![(
            "test:stick".into(),
            Arc::new(Stick)
        )]));
    });
    let definition = ClientItem::from_json(
        r#"{"model":{"type":"minecraft:special","base":"minecraft:item/stick","model":{"type":"test:stick"}}}"#,
    )
    .unwrap();
    Arc::get_mut(&mut game.pack.item_renderer)
        .expect("the game's item renderer is shared")
        .models
        .definitions
        .insert("minecraft:stick".into(), definition);
}

/// A stick of `count` resolved as a frame resolves a held item (`ItemModelResolver.updateForTopItem`).
pub(in crate::game) fn draw_stick(game: &Game<'_>, count: u32) -> ItemRenderState {
    game.resolve_item(
        &horizon_client::items::ItemStack::new("minecraft:stick", count),
        DisplayContext::ThirdPersonRightHand,
        None,
        0,
    )
}

/// What the renderer had been asked about a stack of [`COUNT`] when the tick's mob pass was
/// lifted: set from inside the pass.
static ASKED_IN_PASS: Mutex<Option<usize>> = Mutex::new(None);
const COUNT: u32 = 6;

/// Row 6: the renderer and the game's special stick are in place, and a frame's resolve asks
/// the renderer (the positive control). A tick then lifts its mob pass and ticks a zombie
/// holding that stick, and the renderer is not asked, inside the pass or after it. Item models
/// resolve only on the client's frame, never inside a tick's lifted pass or an action
/// transaction.
#[test]
#[ignore = "daily"]
fn a_special_renderer_is_never_asked_inside_a_lifted_pass() {
    crate::game::in_own_process(
        "game::native_special_renderers::frame_tests::a_special_renderer_is_never_asked_inside_a_lifted_pass",
        a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated,
    );
}

fn a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated() {
    fn in_pass(_: &mut Game<'_>) {
        assert!(
            !horizon_client::join_seq::visibility::passes_settled(),
            "not inside a lifted pass"
        );
        *ASKED_IN_PASS.lock().unwrap() = Some(asked(COUNT));
    }
    crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
        const ZOMBIE: u32 = 1_900_000_100;
        install_special_stick(game);
        let before = asked(COUNT);
        draw_stick(game, COUNT);
        let asked_by_the_frame = asked(COUNT);
        assert_eq!(
            asked_by_the_frame,
            before + 1,
            "the frame's resolve didn't ask"
        );
        balance.drain_balanced();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = horizon_client::mobs::Mob::new(
            ZOMBIE,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            0.0,
        );
        mob.no_ai = true;
        mob.extra.equipment.set(
            horizon_items::equipment::MAINHAND,
            Some(horizon_client::items::ItemStack::new(
                "minecraft:stick",
                COUNT,
            )),
        );
        game.mobs.push(mob);
        if !game.world.chunks.contains_key(&(0, 0)) {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        }
        crate::game::entity::taken_pass_tests::with_spawn(in_pass, || {
            game.tick_creature_entities().unwrap();
        });
        assert!(balance.drain_balanced() > 0, "the tick lifted no mob pass");
        let zombie = game.mobs.iter().find(|m| m.id == ZOMBIE).unwrap();
        assert_eq!(zombie.age, 1, "the zombie didn't tick");
        assert_eq!(
            ASKED_IN_PASS.lock().unwrap().take(),
            Some(asked_by_the_frame),
            "the renderer was asked before the pass, inside the tick"
        );
        assert_eq!(
            asked(COUNT),
            asked_by_the_frame,
            "the renderer was asked in the tick"
        );
        game.mobs.clear();
    });
}
