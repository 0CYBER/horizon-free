use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{
    BlockEntityDrawContext, EntityLayers, EntityRenderLayer, EntityRenderState, LayerTarget,
    PlayerModel, PosedParts,
};
/// Lights what it draws `base` plus the entity's id; scales by 2 when it was given posed parts.
struct Lit(u32);
impl EntityRenderLayer for Lit {
    fn submit(
        &self,
        entity: &EntityRenderState,
        parts: Option<&PosedParts>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        if parts.is_some() {
            draw.scale([2.0; 3]);
        }
        draw.set_light(self.0 + entity.id);
        Ok(())
    }
}
/// Adds a layer to every living type the event lists that has no mod renderer, one to the slim
/// player model, and tries the first type that isn't living, keeping the refusal's text in a
/// layer of the wide model (its light is the text's length).
pub(super) fn register(layers: &mut dyn EntityLayers) {
    for kind in layers.entity_types() {
        if layers.is_living(&kind) && !layers.has_mod_renderer(&kind) {
            layers
                .add(LayerTarget::Entity(kind), Arc::new(Lit(100)))
                .unwrap();
        }
    }
    let models = layers.player_models();
    assert_eq!(models, [PlayerModel::Wide, PlayerModel::Slim]);
    layers
        .add(LayerTarget::Player(PlayerModel::Slim), Arc::new(Lit(200)))
        .unwrap();
    let refused = layers
        .add(
            LayerTarget::Entity("minecraft:oak_boat".into()),
            Arc::new(Lit(0)),
        )
        .unwrap_err();
    layers
        .add(
            LayerTarget::Player(PlayerModel::Wide),
            Arc::new(Lit(refused.len() as u32)),
        )
        .unwrap();
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_entity_layers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityLayersEvent<'_>,
    ) {
        register(event.layers);
    }
}
/// The game's side of the event: a zombie, a mod's golem with a renderer of its own, a boat; it
/// keeps what was added.
#[derive(Default)]
struct Event {
    added: Vec<(LayerTarget, Arc<dyn EntityRenderLayer>)>,
}
impl EntityLayers for Event {
    fn entity_types(&self) -> Vec<String> {
        ["minecraft:oak_boat", "minecraft:zombie", "ruby:golem"]
            .map(String::from)
            .into()
    }
    fn is_living(&self, entity_type: &str) -> bool {
        entity_type != "minecraft:oak_boat"
    }
    fn player_models(&self) -> Vec<PlayerModel> {
        vec![PlayerModel::Wide, PlayerModel::Slim]
    }
    fn has_mod_renderer(&self, entity_type: &str) -> bool {
        entity_type == "ruby:golem"
    }
    fn add(
        &mut self,
        target: LayerTarget,
        layer: Arc<dyn EntityRenderLayer>,
    ) -> Result<(), String> {
        if target == LayerTarget::Entity("minecraft:oak_boat".into()) {
            return Err("the entity layer for minecraft:oak_boat is refused".into());
        }
        self.added.push((target, layer));
        Ok(())
    }
}
/// What a layer did to the draw it was handed.
#[derive(Default, Debug, PartialEq)]
struct Draw {
    scaled: Vec<[f32; 3]>,
    light: u32,
}
impl BlockEntityDrawContext for Draw {
    fn scale(&mut self, by: [f32; 3]) {
        self.scaled.push(by);
    }
    fn set_light(&mut self, light: u32) {
        self.light = light;
    }
}
#[test]
fn wire_loopback_entity_layers_register_and_draw_alike_for_native_and_guest_mods() {
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterEntityLayers)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut event = Event::default();
                mod_api::dispatch_register_entity_layers(
                    &mod_api::RegistrationContext::new(),
                    &mut event,
                );
                let targets: Vec<_> = event.added.iter().map(|(t, _)| t.clone()).collect();
                assert_eq!(
                    targets,
                    [
                        LayerTarget::Entity("minecraft:zombie".into()),
                        LayerTarget::Player(PlayerModel::Slim),
                        LayerTarget::Player(PlayerModel::Wide),
                    ],
                    "guest {guest}"
                );
                let entity: EntityRenderState = mod_api::__private::built::EntityRenderState {
                    id: 7,
                    partial: 0.5,
                    age: 3.5,
                    yaw: 30.0,
                    pitch: 0.0,
                    living: None,
                }
                .into();
                let still = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
                let parts = PosedParts::new(|_| Some((still, true))).unwrap();
                let refused = "the entity layer for minecraft:oak_boat is refused".len() as u32;
                for ((_, layer), (with_parts, expected)) in event.added.iter().zip([
                    (
                        true,
                        Draw {
                            scaled: vec![[2.0; 3]],
                            light: 107,
                        },
                    ),
                    (
                        false,
                        Draw {
                            scaled: vec![],
                            light: 207,
                        },
                    ),
                    (
                        false,
                        Draw {
                            scaled: vec![],
                            light: refused + 7,
                        },
                    ),
                ]) {
                    let mut draw = Draw::default();
                    let parts = with_parts.then_some(&parts);
                    assert_eq!(layer.submit(&entity, parts, &mut draw), Ok(()));
                    assert_eq!(draw, expected, "guest {guest}");
                }
            },
        );
    }
}

pub(super) struct EntityLayersHooks;
impl EntityLayersHooks {
    pub(super) fn register_entity_layers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityLayersEvent<'_>,
    ) {
        self::register(event.layers);
    }
}
