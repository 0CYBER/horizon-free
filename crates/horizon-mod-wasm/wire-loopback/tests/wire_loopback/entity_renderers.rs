use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{
    BlockEntityDrawContext, Direction, EntityRenderState, EntityRenderer, EntityRendererRegistry,
    EntityRenderers, Pose,
};
/// Lights what it draws `base` plus the entity's id, after moving up by its age.
struct Marked(u32);
impl EntityRenderer for Marked {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        draw.translate([0.0, entity.age, 0.0]);
        // A living entity turns with its body, and a baby sleeping in a bed facing east (its
        // pose, bed and flags read across the boundary) moves by its scale, walk and death time.
        let turn = entity.living.map_or(entity.yaw, |living| living.body_rot);
        draw.rotate_about([0.0, 1.0, 0.0], -turn, [0.0; 3]);
        if let Some(l) = entity.living
            && l.pose == Pose::Sleeping
            && l.bed_orientation == Some(Direction::East)
            && l.is_baby
            && l.has_red_overlay
        {
            draw.translate([l.scale * l.age_scale, l.walk_animation_pos, l.death_time]);
        }
        draw.set_light(self.0 + entity.id);
        Ok(())
    }
}
pub(super) fn register(renderers: &mut dyn EntityRenderers) {
    renderers.register("wire_test:bolt".into(), Arc::new(Marked(100)));
    renderers.register("wire_test:dart".into(), Arc::new(Marked(200)));
    // The later registration for a type wins.
    renderers.register("wire_test:bolt".into(), Arc::new(Marked(300)));
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        register(event.renderers);
    }
}
/// What a renderer did to the draw it was handed.
#[derive(Default, Debug, PartialEq)]
struct Draw {
    moved: Vec<[f32; 3]>,
    turned: Vec<([f32; 3], f32)>,
    light: u32,
}
impl BlockEntityDrawContext for Draw {
    fn translate(&mut self, by: [f32; 3]) {
        self.moved.push(by);
    }
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, _: [f32; 3]) {
        self.turned.push((axis, degrees));
    }
    fn set_light(&mut self, light: u32) {
        self.light = light;
    }
}
#[test]
fn wire_loopback_entity_renderers_register_and_draw_alike_for_native_and_guest_mods() {
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterEntityRenderers)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut registry = EntityRendererRegistry::default();
                mod_api::dispatch_register_entity_renderers(
                    &mod_api::RegistrationContext::new(),
                    &mut registry,
                );
                let renderers = registry.into_renderers();
                assert_eq!(
                    renderers.keys().collect::<Vec<_>>(),
                    ["wire_test:bolt", "wire_test:dart"],
                    "guest {guest}"
                );
                let entity: EntityRenderState = mod_api::__private::built::EntityRenderState {
                    id: 7,
                    partial: 0.25,
                    age: 12.25,
                    yaw: 90.0,
                    pitch: -10.0,
                    living: None,
                }
                .into();
                for (kind, light) in [("wire_test:bolt", 307), ("wire_test:dart", 207)] {
                    let mut draw = Draw::default();
                    assert_eq!(renderers[kind].submit(&entity, &mut draw), Ok(()));
                    assert_eq!(
                        draw,
                        Draw {
                            moved: vec![[0.0, 12.25, 0.0]],
                            turned: vec![([0.0, 1.0, 0.0], -90.0)],
                            light,
                        },
                        "{kind}, guest {guest}"
                    );
                }
                // A living entity's frame crosses whole.
                let living = mod_api::__private::built::LivingRenderState {
                    body_rot: 45.0,
                    y_rot: 0.0,
                    x_rot: 0.0,
                    death_time: 3.5,
                    walk_animation_pos: 2.0,
                    walk_animation_speed: 0.0,
                    scale: 3.0,
                    age_scale: 0.5,
                    is_upside_down: false,
                    is_baby: true,
                    is_in_water: false,
                    has_red_overlay: true,
                    pose: Pose::Sleeping,
                    bed_orientation: Some(Direction::East),
                };
                let mob: EntityRenderState = mod_api::__private::built::EntityRenderState {
                    id: 7,
                    partial: 0.25,
                    age: 12.25,
                    yaw: 90.0,
                    pitch: -10.0,
                    living: Some(living.into()),
                }
                .into();
                let mut draw = Draw::default();
                assert_eq!(renderers["wire_test:dart"].submit(&mob, &mut draw), Ok(()));
                assert_eq!(
                    draw,
                    Draw {
                        moved: vec![[0.0, 12.25, 0.0], [1.5, 2.0, 3.5]],
                        turned: vec![([0.0, 1.0, 0.0], -45.0)],
                        light: 207,
                    },
                    "a living entity, guest {guest}"
                );
                if guest {
                    // A guest call whose reply doesn't decode (as a refused or trapped call
                    // leaves none) is an `Err`, never a silent `Ok`: the game then draws the
                    // type's own geometry.
                    APPEND_ATTACK.set(3);
                    let refused = renderers["wire_test:bolt"].submit(&entity, &mut Draw::default());
                    APPEND_ATTACK.set(0);
                    assert!(
                        refused
                            .as_ref()
                            .is_err_and(|e| e.starts_with("wasm call refused or trapped")),
                        "{refused:?}"
                    );
                }
            },
        );
    }
}

pub(super) struct EntityRenderersHooks;
impl EntityRenderersHooks {
    pub(super) fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        self::register(event.renderers);
    }
}
