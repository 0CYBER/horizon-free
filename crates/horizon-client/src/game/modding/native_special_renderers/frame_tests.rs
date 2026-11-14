//! When a mod's special model renderer is asked, in a real game: on the client's frame, outside
//! any tick's lifted pass and any action transaction (matrix rows 6 and 7 of
//! `docs/design/mod-api/resource-renderers.md`). Each test runs alone in its own process: it
//! registers the process's renderers.
use crate::game::Game;
use horizon_assets::item_model::{
    ClientItem, Context, DisplayContext, ItemModels, Layer, LayerContent, StackView,
    register_special_model_types,
};
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

/// The layers a stick of `count` draws when its model is a `minecraft:special` of the type
/// `test:stick`, resolved as a frame resolves a held item (`ItemModelResolver.updateForTopItem`
/// through the game's `ItemEnv`).
pub(in crate::game) fn special_stick_layers(game: &Game<'_>, count: i32) -> Vec<Layer> {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        register_special_model_types(vec!["test:stick".into()]);
        assert!(super::register(vec![(
            "test:stick".into(),
            Arc::new(Stick)
        )]));
    });
    let mut models = ItemModels::default();
    models.definitions.insert(
        "minecraft:stick".into(),
        ClientItem::from_json(
            r#"{"model":{"type":"minecraft:special","base":"minecraft:item/stick","model":{"type":"test:stick"}}}"#,
        )
        .unwrap(),
    );
    Arc::make_mut(&mut models.prototypes).insert(
        "minecraft:stick".into(),
        std::collections::BTreeMap::from([(
            "minecraft:item_model".into(),
            serde_json::Value::String("minecraft:stick".into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::ThirdPersonRightHand,
        owner: None,
        level: None,
        seed: 0,
    };
    models
        .resolve(
            &StackView::new("minecraft:stick", count),
            &context,
            &game.item_env(),
        )
        .layers
}

/// Whether `layers` are the renderer's answer.
pub(in crate::game) fn drawn_by_the_renderer(layers: &[Layer]) -> bool {
    matches!(
        layers,
        [Layer {
            content: LayerContent::Parts(_),
            ..
        }]
    )
}

/// Row 6: a tick lifts its mob pass while a mob holds a special item, and the renderer is not
/// asked; the frame's draw of the same stack asks it. Item models resolve only on the client's
/// frame, never inside a tick's lifted pass or an action transaction.
#[test]
#[ignore = "daily"]
fn a_special_renderer_is_never_asked_inside_a_lifted_pass() {
    crate::game::in_own_process(
        "game::native_special_renderers::frame_tests::a_special_renderer_is_never_asked_inside_a_lifted_pass",
        a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated,
    );
}

fn a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated() {
    crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
        const COUNT: u32 = 6;
        balance.drain_balanced();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = horizon_client::mobs::Mob::new(
            1_900_000_100,
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
        let before = asked(COUNT);
        game.tick_creature_entities().unwrap();
        assert!(balance.drain_balanced() > 0, "the tick lifted no mob pass");
        assert_eq!(asked(COUNT), before, "the renderer was asked in a tick");
        let layers = special_stick_layers(game, COUNT as i32);
        assert!(drawn_by_the_renderer(&layers), "{layers:?}");
        assert_eq!(asked(COUNT), before + 1);
        game.mobs.clear();
    });
}
