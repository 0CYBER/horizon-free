//! A mod built to `mod.wasm` (`cargo horizon-mod build --wasm`): the same source builds as a
//! native mod too.
// ANCHOR: mod
use mod_api::{
    BlockKey, DataLoadContext, EffectKey, Flow, FluidDefinition, FluidIngredient, FluidKey,
    FluidStack, FluidStateError, Hooks, HostFluidStates, HostSide, ItemPart, LootTableLoadEvent,
    Registrar, RegistrationContext, ServerChatEvent, ServerHost, ServerTickEvent, StateConstraint,
    Text, UseItemEvent,
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Mod {
    stone: std::sync::OnceLock<BlockKey>,
    speed: std::sync::OnceLock<EffectKey>,
    water: std::sync::OnceLock<FluidKey>,
    chocolate: std::sync::OnceLock<FluidKey>,
}

/// Sticks announce each use.
struct Stick;

#[mod_api::horizon_part]
impl ItemPart for Stick {
    fn use_item(
        &self,
        host: HostSide<'_>,
        event: &mut UseItemEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        if let HostSide::Server(host) = host {
            let item = event.stack.item().id(host);
            host.messages()
                .broadcast(Text::literal(format!("a {item} was used")));
        }
        None
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        let item = registrar.item_key("minecraft:stick");
        registrar.item_part(item, Stick);
        let _ = self.stone.set(registrar.block_key("minecraft:stone"));
        let _ = self.speed.set(registrar.effect_key("minecraft:speed"));
        let _ = self.water.set(registrar.fluid_key("minecraft:water"));
        let _ = self.chocolate.set(registrar.declare_fluid("chocolate"));
    }

    /// Counts server ticks through the ordinary hook, including on plain builds.
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, mod_api::ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        if ticks % 200 == 0 {
            host.messages()
                .broadcast(Text::literal(format!("hello_wasm: {ticks} server ticks")));
        }
    }

    fn loot_table_load(&self, context: &dyn DataLoadContext, event: &mut LootTableLoadEvent<'_>) {
        if event.id != "test:loot" {
            return;
        }
        let water = context.fluid_key("minecraft:water").expect("water");
        let ingredient =
            FluidIngredient::from_json(&mod_api::serde_json::json!("minecraft:water"), context)
                .expect("ingredient");
        assert!(ingredient.test(&FluidStack::new(water, 1)));
        *event.table = mod_api::serde_json::json!({"fluid_guest_ok": true});
    }

    fn server_chat(&self, host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let stone = *self.stone.get().expect("registered block key");
        let speed = *self.speed.get().expect("registered effect key");
        assert_eq!(stone.id(host), "minecraft:stone");
        assert_eq!(speed.id(host), "minecraft:speed");
        let state = host.block_state_default(stone).expect("stone state");
        let fluid = host.state_fluid_state(state).expect("fluid state");
        assert_eq!(
            (
                fluid.amount,
                fluid.falling,
                fluid.is_source(),
                fluid.is_empty()
            ),
            (0, false, false, true)
        );
        let service = host.fluid_states();
        let water = *self.water.get().expect("registered water");
        let chocolate = *self.chocolate.get().expect("declared chocolate");
        let Some(FluidDefinition::World {
            default,
            properties,
        }) = service.definition(water)
        else {
            panic!("water definition")
        };
        assert!(default.matches(&[StateConstraint::new("falling", "true")]));
        assert!(
            properties
                .iter()
                .any(|p| p.name == "falling" && p.values.contains(&"false".into()))
        );
        assert_eq!(service.validate(water, &[]), Ok(()));
        assert_eq!(
            service.validate(water, &[StateConstraint::new("level", "8")]),
            Err(FluidStateError::UnknownProperty(0))
        );
        assert_eq!(
            service.validate(
                water,
                &[
                    StateConstraint::new("falling", "true"),
                    StateConstraint::new("falling", "true")
                ]
            ),
            Err(FluidStateError::Duplicate(1))
        );
        let properties = service.type_properties(water).expect("water properties");
        assert_eq!(properties.description_id, "block.minecraft.water");
        assert_eq!(properties.light_level, 3);
        assert_eq!(properties.fall_distance_modifier, 0.25);
        assert!(properties.can_push_entity && properties.can_drown && properties.can_hydrate);
        assert!(!properties.can_swim && !properties.can_extinguish && !properties.supports_boating);
        assert_eq!(properties.explosion_resistance, 7.5);
        assert!(properties.randomly_ticks);
        assert_eq!(
            properties.drip_particle.as_deref(),
            Some("sample:dripping_water")
        );
        // The sample host answers 1000 plus the amount (8) plus one for a falling state.
        assert_eq!(
            service.legacy_block(default).map(|state| state.0),
            Some(1009)
        );
        assert_eq!(service.type_properties(chocolate), None);
        assert_eq!(
            service.definition(chocolate),
            Some(FluidDefinition::TankOnly)
        );
        assert_eq!(
            service.validate(chocolate, &[]),
            Err(FluidStateError::NoWorldState)
        );
        *event.message = Text::literal(format!("{} (via mod.wasm)", event.raw_text));
        Flow::Continue
    }
}

static TICKS: AtomicU64 = AtomicU64::new(0);

// ANCHOR_END: mod
