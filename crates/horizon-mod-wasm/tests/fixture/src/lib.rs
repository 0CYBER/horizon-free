use mod_api::Hooks;
use std::sync::atomic::{AtomicU32, Ordering};

static ENTITY_PRE_CALLS: AtomicU32 = AtomicU32::new(0);
static ENTITY_POST_CALLS: AtomicU32 = AtomicU32::new(0);

// Generated in filename order by build.rs; a new guest registration needs only its own file.
include!(concat!(env!("OUT_DIR"), "/wasm_fixture_modules.rs"));
pub use matrix::{mirrors, sum_return};
pub use replace::own_average;
pub use shutdown::on_shutdown;
pub use turn::turn_angle;

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn entity_interact(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        predicted_use::entity_interact(host, event)
    }

    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> mod_api::Flow {
        predicted_use::packet(event);
        mod_api::Flow::Continue
    }

    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        lending::Lender.living_death(host, event)
    }

    fn movement_input(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::MovementInputEvent) {
        movement::movement_input(event);
    }

    fn tick_target_millis(
        &self,
        host: &dyn mod_api::ClientHost,
        event: &mut mod_api::TickTargetMillisEvent,
    ) {
        if event.millis == -2.0 {
            event.millis = strict_math_checks::check_vectors() as f32;
            return;
        }
        let profile = host.math_profile();
        assert_eq!(profile, host.math_profile());
        event.millis = match profile {
            mod_api::MathProfile::LibmRounding => 1.0,
            mod_api::MathProfile::CorrectlyRounded => 2.0,
            mod_api::MathProfile::Aarch64Fma => 3.0,
            mod_api::MathProfile::StrictMath => 4.0,
            _ => panic!("fixture does not support the new math profile"),
        };
    }

    fn player_stat_award(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::PlayerStatAwardEvent<'_>,
    ) -> mod_api::Flow {
        // A named acceptance actor keeps this fixture inert for unrelated Game tests.
        if event.name != "stat_award_fixture" {
            return mod_api::Flow::Continue;
        }
        if event.stat_type.as_str() == "minecraft:mined" && event.stat.as_str() == "minecraft:stone"
        {
            event.value *= 2;
        } else if event.stat_type.as_str() == "minecraft:custom"
            && event.stat.as_str() == "minecraft:jump"
        {
            *event.stat = "wasm_fixture:steps".into();
            event.value = 3;
        }
        mod_api::Flow::Continue
    }

    fn server_tick(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerTickEvent<'_>,
    ) {
        host.messages()
            .broadcast(mod_api::Text::literal("wasm tick phase regression"))
            .unwrap();
    }

    fn right_click_block(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::RightClickBlockEvent<'_>,
    ) -> mod_api::Flow {
        interaction::right_click_block(host, event)
    }

    fn use_item_on_block(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::UseItemOnBlockEvent<'_>,
    ) -> mod_api::Flow {
        interaction::use_item_on_block(host, event)
    }

    fn interaction_key_mapping_triggered(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::InteractionKeyMappingTriggeredEvent,
    ) -> mod_api::Flow {
        if predicted_use::STATE.lock().unwrap().0[0] != 0 {
            return mod_api::Flow::Continue;
        }
        event.swing_hand = event.attempt == mod_api::InteractionAttempt::Held;
        if event.hand == mod_api::Hand::Off {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }

    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        event.commands.extend([
            mod_api::Command::new("wasmheal <targets:players>"),
            mod_api::Command::new("wasmentity <target:entity>"),
            mod_api::Command::new("wasmentities <targets:entities>"),
            mod_api::Command::new("wasmplayer <target:player>"),
            mod_api::Command::new("wasmcauldron"),
            mod_api::Command::new("wasmbulkfiles"),
            mod_api::Command::new("wasmbulkfilesnoworld"),
        ]);
    }

    fn run_mod_command(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RunModCommandEvent<'_>,
    ) -> mod_api::Flow {
        match event.command.literals.first().map(String::as_str) {
            Some("wasmbulkfiles") => {
                bulk_files::check(host);
                return mod_api::Flow::Cancel;
            }
            Some("wasmbulkfilesnoworld") => {
                bulk_files::no_world(host);
                return mod_api::Flow::Cancel;
            }
            _ => {}
        }
        if event.command.literals.first().map(String::as_str) == Some("wasmcauldron") {
            cauldron::read(host);
            return mod_api::Flow::Cancel;
        }
        selector_command::run(event)
    }

    fn recipe_sources(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeSourcesEvent<'_>,
    ) {
        lending::Lender.recipe_sources(host, event);
    }

    fn menu_links(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::MenuLinksEvent<'_>,
    ) {
        lending::Lender.menu_links(host, event);
    }

    fn register_json_animation_types(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
    ) {
        // The guest pushes a plain batch; the host names this mod on each entry.
        assert!(
            event.types.is_empty(),
            "the guest starts from its own batch"
        );
        event.types.push(mod_api::JsonAnimationType::Target(
            mod_api::Identifier::parse("wasm_fixture:pulse").unwrap(),
        ));
    }

    fn standalone_models(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::StandaloneModelsEvent<'_>,
    ) {
        assert!(
            event.forms.is_empty(),
            "dispatch owns the guest's form batch"
        );
        assert_eq!(event.models.as_slice(), ["native:kept"]);
        event.models.push("wasm_fixture:block/plain".into());
        event.forms.extend([
            mod_api::StandaloneModelForm::Rotated {
                id: "another_namespace:east".into(),
                model: "dependency:block/arrow".into(),
                turns: [2, 1, 3],
                uvlock: true,
            },
            mod_api::StandaloneModelForm::Combined {
                id: "wasm_fixture:assembly".into(),
                parts: vec![
                    "dependency:block/base".into(),
                    "dependency:block/overlay".into(),
                ],
            },
        ]);
    }

    fn entity_tick_listened(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityTickListenedEvent,
    ) {
        event.listened = true;
    }

    fn entity_tick_pre(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        ENTITY_PRE_CALLS.fetch_add(1, Ordering::Relaxed);
        mod_api::Flow::Continue
    }

    fn entity_tick_post(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        ENTITY_POST_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    #[listener(priority = 37)]
    fn server_stopped(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ServerStoppedEvent) {
    }

    fn server_started(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        runtime_environment::started(host);
        capability_registration::started(host);
        if host.is_loaded("datapack_fixture") {
            datapack::read(host);
        }
    }

    #[listener(priority = -12, receive_cancelled)]
    fn render_gui_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        event.cancelled = false;
        mod_api::Flow::Continue
    }

    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        part_state::blocks(event);
    }

    fn register(&self, registrar: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {
        block_state_notice::register(registrar, cx);
        runtime_environment::register(cx);
        capability_registration::register(registrar, cx);
        game_payload::register(registrar);
        interaction::register(registrar);
        predicted_use::register(registrar);
        cauldron::register(registrar);
        tick_reply::register(registrar);
        part_state::register(registrar);
        registrar.register_custom_stat("steps").unwrap();
        registrar
            .declare_datapack_registry(
                "spell",
                mod_api::DatapackRegistrySync::ServerOnly,
                vec![mod_api::DatapackField::required(
                    "power",
                    mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                )],
            )
            .unwrap();
        if cx.mods().get("refuse_registration").is_some() {
            registrar.declare_fluid("before_refusal");
            let kind = registrar
                .block_entity_type("discarded_callback")
                .blocks(&[])
                .build();
            registrar.block_entity_part(&kind, Counter).unwrap();
            cx.refuse("guest bootstrap failed: café\nmissing recipe bridge");
            cx.refuse("must not replace the first diagnostic");
            registrar.declare_fluid("foreign:after_refusal");
            return;
        }
        registrar.item_key("minecraft:snowball");
        let launch = registrar
            .block_entity_type("owner_launch")
            .blocks(&[])
            .build();
        registrar
            .block_entity_part(&launch, projectile_owner::OwnerLaunch)
            .unwrap();
        let kind = registrar.block_entity_type("counter").blocks(&[]).build();
        registrar.block_entity_part(&kind, Counter).unwrap();
        let reads = registrar
            .block_entity_type("counter_reads")
            .blocks(&[])
            .build();
        registrar.block_entity_part(&reads, CounterReads).unwrap();
    }
}

// Only the guest ABI exports reach this fixture function; the bench target does not call it.
#[allow(dead_code)]
pub fn on_tick(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    args.cancel = true;
}

pub struct Counter;
pub struct CounterReads;

#[mod_api::horizon_part]
impl mod_api::BlockPart for CounterReads {}

#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for CounterReads {
    fn block_entity_tick(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        let first = host.is_loaded("wasm_fixture");
        let second = host.is_loaded("missing");
        event
            .entity
            .nbt
            .insert("counter".into(), mod_api::Tag::Int(17));
        event.entity.data = vec![if first == second { 24 } else { 25 }];
        event.entity.items.clear();
    }
}

#[mod_api::horizon_part]
impl mod_api::BlockPart for Counter {}

#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Counter {
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        event
            .entity
            .nbt
            .insert("counter".into(), mod_api::Tag::Int(17));
        event.entity.nbt.insert(
            "entity_pre_calls".into(),
            mod_api::Tag::Int(ENTITY_PRE_CALLS.load(Ordering::Relaxed) as i32),
        );
        event.entity.nbt.insert(
            "entity_post_calls".into(),
            mod_api::Tag::Int(ENTITY_POST_CALLS.load(Ordering::Relaxed) as i32),
        );
        event.entity.data = vec![23];
        event.entity.items.clear();
    }
}
