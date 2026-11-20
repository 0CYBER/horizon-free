//! The `NativeClientHooks` the library's tests install. The callbacks are process-wide and
//! install once, so every test that needs them calls [`install`]. Each answers only under a
//! setting its own tests write: `rules_mod` for the menu, entity and block hooks, and
//! `item_parts` for the item hooks ([`answering`]), so every other test sees vanilla's answers.
use super::NativeClientHooks;
use crate::items::ItemStack;
use horizon_world::native_load_context::{
    DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The server-side jumps the installed `living_jump` heard.
pub(crate) static JUMPS: AtomicUsize = AtomicUsize::new(0);
/// The asks the installed `menu_still_valid` heard.
pub(crate) static STILL_VALID_ASKED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The item hooks this thread asked: the hook, the stack's item, and the answer it arrived
    /// with.
    static ITEM_ASKS: std::cell::RefCell<Vec<(&'static str, String, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The item hooks this thread asked since the last call, oldest first: the hook, the stack's
/// item, and the answer it arrived with.
pub(crate) fn item_asks() -> Vec<(&'static str, String, bool)> {
    ITEM_ASKS.with_borrow_mut(std::mem::take)
}

/// A server loop context whose `item_parts` setting `hook` holds `answers`: comma-separated
/// `item=true` or `item=false` pairs, which the installed item hooks answer for those items.
pub(crate) fn answering(hook: &str, answers: &str) -> LoopContext {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let settings = std::env::temp_dir().join(format!(
        "horizon-item-parts-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = settings.join("item_parts").join(hook);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, answers).unwrap();
    let inputs = std::sync::Arc::new(LoadInputs::new(Default::default(), settings));
    LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
        .loop_context(LoopSide::Server, ["item_parts"])
}

/// Record the ask, then answer what the `item_parts` setting `hook` gives `stack`'s item, if
/// anything.
fn item_part(context: &LoopContext, hook: &'static str, stack: &ItemStack, answer: &mut bool) {
    ITEM_ASKS.with_borrow_mut(|asks| asks.push((hook, stack.item.to_string(), *answer)));
    let Some(answers) = context.setting("item_parts", hook) else {
        return;
    };
    let given = answers.split(',').find_map(|pair| {
        let (item, given) = pair.split_once('=')?;
        (item == &*stack.item).then_some(given == "true")
    });
    if let Some(given) = given {
        *answer = given;
    }
}

/// Install the test callbacks, once for the process.
pub(crate) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let installed = super::install(NativeClientHooks {
            menu_still_valid: |context, player, menu, _, eye, valid| {
                STILL_VALID_ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if menu == "test:rules" && player == 2 && eye[2] == 0.0 {
                    *valid = context.setting("rules_mod", "stack").is_some();
                }
            },
            // A part answers on the vanilla chest menu too: the game must
            // still never ask it, because it is not a mod menu type.
            menu_still_valid_listens: |menu| matches!(menu, "test:rules" | "minecraft:generic_9x3"),
            item_stacked_on_other: |context, _, canceled| {
                *canceled = context.setting("rules_mod", "stack").is_some();
            },
            living_heal: |context, _, amount, cancel| {
                if context.setting("rules_mod", "stack").is_some() {
                    *cancel = true;
                    *amount += 1.0;
                }
            },
            living_breathe: |context, _, kind, can_breathe, consume, _| {
                if context.setting("rules_mod", "stack").is_some() {
                    // A cod breathes on land; a salmon loses all its air at once.
                    *can_breathe = kind == "minecraft:cod";
                    *consume = match kind {
                        "minecraft:salmon" => i32::MAX,
                        _ => *consume + 4,
                    };
                }
            },
            player_login_profile: |player, name, properties| {
                if mod_api::__private::listened(mod_api::HookId::PlayerLoginProfile) {
                    mod_api::dispatch_player_login_profile(player, name, properties);
                }
            },
            farmland_trample: |context, _, _, _, _, cancel| {
                *cancel = context.setting("rules_mod", "stack").is_some();
            },
            has_mods: || false,
            player_name_format: |context, _, displayname| {
                if let Some(value) = context.setting("rules_mod", "stack") {
                    *displayname = crate::text::Text::literal(&value);
                }
            },
            player_tab_list_name_format: |context, _, _, display_name| {
                // Without the setting the name is left as the game made it.
                if let Some(value) = context.setting("rules_mod", "stack") {
                    *display_name = Some(crate::text::Text::literal(&value));
                }
            },
            living_drown: |context, _, _, drowning, damage, bubbles, _| {
                if context.setting("rules_mod", "stack").is_some() {
                    *drowning = true;
                    *damage += 1.0;
                    *bubbles = 0;
                }
            },
            living_fall: |context, _, _, distance, multiplier, _| {
                if context.setting("rules_mod", "stack").is_some() {
                    *distance += 1.0;
                    *multiplier *= 2.0;
                }
            },
            living_jump: |context, _, _| {
                JUMPS.fetch_add(
                    usize::from(context.side() == LoopSide::Server),
                    std::sync::atomic::Ordering::Relaxed,
                );
            },
            living_jump_power: |context, _, _, power| {
                *power += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
            },
            living_speed: |context, _, _, speed| {
                *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 2.0);
            },
            living_flying_speed: |context, _, _, speed| {
                *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 3.0);
            },
            block_jump_factor: |context, _, factor| {
                *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 4.0);
            },
            block_speed_factor: |context, _, factor| {
                *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 5.0);
            },
            is_ladder: |context, _, _, _, ladder| {
                *ladder |= context.setting("rules_mod", "stack").is_some();
            },
            makes_open_trapdoor_above_climbable: |context, _, _, _, climbable| {
                *climbable |= context.setting("rules_mod", "stack").is_some();
            },
            friction: |context, _, kind, _, _, friction| {
                if context.setting("rules_mod", "stack").is_some() {
                    *friction = match kind {
                        "minecraft:player" => 0.98,
                        "minecraft:cow" => f32::NAN,
                        _ => 3.0,
                    };
                }
            },
            player_stays_on_ground_surface: |context, _, staying| {
                *staying |= context.setting("rules_mod", "stack").is_some();
            },
            ender_entity_teleport: |context, _, _, _, target, cancel| {
                if context.setting("rules_mod", "stack").is_some() {
                    target[1] += 1.0;
                    *cancel = true;
                }
            },
            fluid_flow: |context, _, _, flow| {
                flow[0] += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
            },
            sound_type: |context, _, _, entity, cause, sound| {
                if context.setting("rules_mod", "stack").is_some()
                    && entity.is_some_and(|(_, kind)| kind == "minecraft:player")
                    && cause == mod_api::BlockSoundCause::Step
                {
                    sound.step = "rules_mod:block.rules.step".into();
                    sound.volume = 2.0;
                }
            },
            can_grindstone_repair: |context, stack, can_repair| {
                item_part(context, "can_grindstone_repair", stack, can_repair);
            },
            is_combine_repairable: |context, stack, repairable| {
                item_part(context, "is_combine_repairable", stack, repairable);
            },
            can_fit_inside_container_items: |context, stack, fits| {
                item_part(context, "can_fit_inside_container_items", stack, fits);
            },
        });
        assert!(
            installed,
            "the library's callbacks are installed only here in its tests"
        );
    });
}
