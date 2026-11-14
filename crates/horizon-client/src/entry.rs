//! Startup shared by the gameplay library and its executable.
#[path = "horizon_look.rs"]
mod horizon_look;
use crate::{crash, game, logging};
use horizon_mod_package::selection::{Argument, LaunchArguments};
pub(crate) mod dedicated_arguments;
pub(crate) mod runtime_environment;

/// Validated startup state. No native hooks or workers may run before prepare returns.
pub struct Startup {
    launch: LaunchArguments,
    options: game::Options,
    _capture: Option<horizon_mod_package::output::Capture>,
}

impl Startup {
    /// Validated launch context used by the platform's code loader.
    pub fn launch(&self) -> &LaunchArguments {
        &self.launch
    }
}

/// Holder mode runs first. The game's sole Options validation reads arguments/environment
/// and resource marker metadata only: no logging, account loading, mods or worker threads.
/// Account IO, cwd changes and command registration remain in Options::read, after leases.
pub fn prepare() -> Startup {
    horizon_mod_package::holder::enter();
    let result: crate::Result<_> = (|| {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.args(std::env::args_os().skip(1));
        let launch = LaunchArguments::for_command(&command)?;
        let options = game::Options::validate(launch.clone())?;
        Ok((launch, options))
    })();
    let (launch, options) = result.unwrap_or_else(|error| {
        eprintln!("Horizon: {error}");
        std::process::exit(game::exit_code::USAGE);
    });
    let Some(options) = options else {
        std::process::exit(0);
    };
    let capture = horizon_mod_package::holder::acquire(&launch);
    Startup {
        launch,
        options,
        _capture: capture,
    }
}

/// Refuse startup if any merged copy could not be installed, before loading any mod.
/// Cells cannot roll back: each copy merges all requesting rows, so stopping the whole
/// start is how we ensure none of a refused mod's rows run.
pub fn installed(patched: &[(&str, &[&str], bool)]) {
    let refusals = installation_refusals(patched);
    for line in &refusals {
        eprintln!("{line}");
    }
    if !refusals.is_empty() {
        std::process::exit(game::exit_code::ERROR);
    }
}

fn installation_refusals(patched: &[(&str, &[&str], bool)]) -> Vec<String> {
    patched.iter().filter(|(_, _, installed)| !installed).map(|(target, mods, _)| {
        format!("Horizon: {}: {target}: a patched copy was already installed; the game was not started", mods.join(", "))
    }).collect()
}

/// Code-runtime services supplied by the platform at startup.
pub struct CodeRuntime {
    pub initialize: fn(&[mod_api::PackageInfo]),
    pub refusal: fn(&str) -> Option<String>,
    pub startup_refused: fn(&str) -> bool,
}
static CODE_RUNTIME: std::sync::OnceLock<CodeRuntime> = std::sync::OnceLock::new();
pub(crate) fn initialize_code(inventory: &[mod_api::PackageInfo]) {
    let runtime = CODE_RUNTIME
        .get()
        .expect("code initialization requires entry::start");
    (runtime.initialize)(inventory);
}
pub(crate) fn code_startup_refused(id: &str) -> bool {
    CODE_RUNTIME
        .get()
        .is_some_and(|runtime| (runtime.startup_refused)(id))
}
pub(crate) fn code_refusal(id: &str) -> Option<String> {
    CODE_RUNTIME.get().and_then(|runtime| (runtime.refusal)(id))
}

/// Start the engine with dependency-ordered code supplied by the host.
pub fn start<G>(
    native: horizon_mod_package::runtime::Runtime,
    load: impl FnOnce(
        &horizon_mod_package::runtime::Runtime,
        &LaunchArguments,
    ) -> (Vec<&'static str>, G),
    code: CodeRuntime,
    startup: Startup,
) {
    let _ = CODE_RUNTIME.set(code);
    let Startup {
        launch,
        options,
        _capture,
    } = startup;
    runtime_environment::capture(&launch);
    horizon_threads::configure_allocator();
    horizon_chat_filter::install();
    horizon_look::give();
    // OptiFine's and Polytone's pack formats (platform, docs/layers.md), before any pack loads.
    horizon_client::pack_formats::install();
    // horizon-extras (docs/forking.md): without the feature, no Horizon Settings or Mods page.
    if !cfg!(feature = "horizon-extras") {
        horizon_client::options_ui::HORIZON_EXTRAS
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    horizon_client::telemetry::mark_start();
    // Opt-in telemetry keeps each crash report (docs/telemetry.md), whatever the mode.
    crash::on_report(horizon_client::telemetry::keep_crash);
    // Crates above the engine registered their hooks before this (horizon-official's `main`),
    // then the Horizon parts still in this crate; none registers after this point.
    game::player_hooks::register();
    horizon_hooks::engine::seal();
    // The text crates ship (`horizon_hooks::lang`), loaded with the game's own languages.
    horizon_client::lang::install_crate_files(horizon_hooks::lang::engine::all());
    horizon_client::spans::startup::init(std::time::Instant::now());
    logging::set_game_home(launch.context.home.clone());
    // The dedicated server logs in its own folder, as vanilla's does; the game in its home.
    let (home, side, thread) = if launch
        .rest
        .iter()
        .any(|arg| matches!(arg, Argument::Dedicated(_)))
    {
        (launch.context.game_dir.clone(), "server", "Server thread")
    } else {
        // A player's own log stays separate; crash reports follow the shared home.
        if let Some(dir) = launch.rest.iter().find_map(|arg| match arg {
            Argument::Value(name, value) if name == "--player-dir" => Some(value),
            _ => None,
        }) {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (launch.context.home.clone(), "client", "Render thread")
    };
    logging::init(
        logging::player_dir_override().unwrap_or(home.as_path()),
        thread,
    );
    // The log's first line names the build, so a bug report that carries the log says which one it was.
    log::info!("Horizon {}", crash::version());
    // Mods' startup bars and lines (`RegistrationContext::progress`), before they register.
    game::startup_progress::install(side == "server");
    // Mod loading emits refusals; both client and dedicated logs must exist first.
    let (_ids, _loaded_code) = load(&native, &launch);
    crash::install(home.clone(), side);
    if horizon_world::native_load_hooks::install(
        horizon_world::native_load_hooks::NativeLoadHooks {
            item_ability_default: horizon_items::item_abilities::vanilla_can_perform,
            loot_ability: crate::game::api_values::loot_values::dispatch_loot_ability,
            loot_table_load: |context, id, table| {
                if !mod_api::__private::listened(mod_api::HookId::LootTableLoad) {
                    return;
                }
                let reader = horizon_client_render::api_context::LoadReader(context);
                let context = mod_api::OwningLoad::new(
                    horizon_client_render::api_context::load_origin(context.origin()),
                    &reader,
                );
                mod_api::dispatch_loot_table_load(&context, id, table);
            },
            modify_default_components: |context, item, components| {
                if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                    return;
                }
                let reader = horizon_client_render::api_context::LoadReader(context);
                let context = reader.context();
                mod_api::dispatch_modify_default_components(&context, item, components);
            },
            loot_function: |kind, data, ctx, stack, tags| {
                crate::game::api_values::dispatch_loot_function(kind, data, ctx, stack, tags);
            },
            loot_condition: |kind, data, ctx, holds, tags| {
                crate::game::api_values::dispatch_loot_condition(kind, data, ctx, holds, tags);
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if horizon_world::native_worldgen_hooks::install(
        horizon_world::native_worldgen_hooks::NativeWorldgenHooks {
            alter_ground: crate::game::api_values::alter_ground,
            on_tree_grow: crate::game::native_tree_grow::on_tree_grow,
            rotate: crate::game::native_rotate::rotate,
            feature: crate::game::native_worldgen_parts::feature,
        },
    )
    .is_err()
    {
        log::warn!("Native worldgen hooks already installed; keeping the first callbacks");
    }
    if horizon_items::native_enchantment_hooks::install(
        crate::enchantment_levels::get_enchantment_level,
    )
    .is_err()
    {
        log::warn!("The enchantment level hook is already installed; keeping the first");
    }
    install_client_hooks();
    raise_open_file_limit();
    if let Err(error) = game::run_with(options, native) {
        // A bad command line is the caller's mistake, not a crash: no report.
        let missing_mods = game::dedicated::is_missing_mods_refusal(error.as_ref());
        let usage = error.is::<game::UsageError>() || missing_mods;
        if !missing_mods {
            log::error!("Horizon: {error}");
        }
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !usage {
            crash::report_error(&home, side, &*error);
        }
        // Started from Explorer, the console window closes with the game:
        // keep the message on screen until it is read.
        #[cfg(windows)]
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
    horizon_hooks::engine::shutdown();
    logging::flush();
}

pub(crate) fn install_client_hooks() {
    game::api_values::install_stack_hooks();
    if !crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
        menu_still_valid: game::api_values::menu_still_valid,
        menu_still_valid_listens: |menu| {
            mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu)
        },
        item_stacked_on_other: |context, event, canceled| {
            game::api_values::stacked_on_other(context, event, canceled);
        },
        living_heal: |context, entity, amount, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingHeal) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_heal(&context, entity, amount, cancel);
        },
        living_breathe: |context, entity, kind, can_breathe, consume, refill| {
            if !mod_api::__private::listened(mod_api::HookId::LivingBreathe) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_breathe(&context, entity, kind, can_breathe, consume, refill);
        },
        farmland_trample: |context, entity, pos, state, distance, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::FarmlandTrample) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_farmland_trample(&context, entity, pos, state, distance, cancel);
        },
        has_mods: || !mod_api::__private::mods().is_empty(),
        player_name_format: |context, username, displayname| {
            use game::api_values::{TryIntoApi, TryIntoEngine};
            if !mod_api::__private::listened(mod_api::HookId::PlayerNameFormat) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let username = username.try_into_api().unwrap();
            let mut converted = displayname.try_into_api().unwrap();
            mod_api::dispatch_player_name_format(&context, &username, &mut converted);
            *displayname = converted.try_into_engine(&()).unwrap();
        },
        player_tab_list_name_format: |context, player, name, display_name| {
            use game::api_values::{TryIntoApi, TryIntoEngine};
            if !mod_api::__private::listened(mod_api::HookId::PlayerTabListNameFormat) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let mut converted = display_name.try_into_api().unwrap();
            mod_api::dispatch_player_tab_list_name_format(&context, player, name, &mut converted);
            *display_name = converted.try_into_engine(&()).unwrap();
        },
        living_drown: |context, entity, kind, is_drowning, damage_amount, bubble_count, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingDrown) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_drown(
                &context,
                entity,
                kind,
                is_drowning,
                damage_amount,
                bubble_count,
                cancel,
            );
        },
        living_fall: |context, entity, kind, distance, multiplier, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFall) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_fall(&context, entity, kind, distance, multiplier, cancel);
        },
        living_jump: |context, entity, kind| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJump) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump(&context, entity, kind);
        },
        living_jump_power: |context, entity, kind, power| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJumpPower) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump_power(&context, entity, kind, power);
        },
        living_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingSpeed) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_speed(&context, entity, kind, speed);
        },
        living_flying_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFlyingSpeed) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_flying_speed(&context, entity, kind, speed);
        },
        block_jump_factor: |context, state, factor| {
            crate::game::block_jump_factor(context, state, factor);
        },
        block_speed_factor: |context, state, factor| {
            crate::game::block_speed_factor(context, state, factor);
        },
        is_ladder: |context, entity, pos, state, ladder| {
            crate::game::is_ladder(context, entity, pos, state, ladder);
        },
        makes_open_trapdoor_above_climbable:
            crate::game::native_trapdoor_ladder::makes_open_trapdoor_above_climbable,
        friction: |context, entity, kind, pos, state, friction| {
            crate::game::native_surface::friction(context, entity, kind, pos, state, friction);
        },
        player_stays_on_ground_surface: |context, entity, staying| {
            if !mod_api::__private::listened(mod_api::HookId::PlayerStaysOnGroundSurface) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_player_stays_on_ground_surface(&context, entity, staying);
        },
        ender_entity_teleport: |context, entity, kind, from, target, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::EnderEntityTeleport) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_ender_entity_teleport(&context, entity, kind, from, target, cancel);
        },
        fluid_flow: |context, fluid, pos, flow| {
            if !mod_api::__private::listened(mod_api::HookId::FluidFlow) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_fluid_flow(&context, fluid, pos, flow);
        },
        sound_type: |context, pos, state, entity, cause, sound| {
            crate::game::native_block_sounds::sound_type(context, pos, state, entity, cause, sound);
        },
    }) {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
}

/// The JVM raises its open-file limit to the hard limit as it starts
/// (`-XX:+MaxFDLimit`, on by default), so a vanilla server may hold as many
/// sockets and region files as the host allows, not the soft default (often
/// 1024 in a container). The same here; macOS caps it at `OPEN_MAX`, as the
/// JVM does there.
fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let limit = getrlimit(Resource::Nofile);
        let hard = if cfg!(target_os = "macos") {
            Some(limit.maximum.map_or(10_240, |h| h.min(10_240)))
        } else {
            limit.maximum
        };
        let raised = match (limit.current, hard) {
            (Some(soft), Some(hard)) => hard > soft,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if raised {
            let to = Rlimit {
                current: hard,
                maximum: limit.maximum,
            };
            match setrlimit(Resource::Nofile, to) {
                Ok(()) => log::info!(
                    "Open file limit raised from {:?} to {hard:?}",
                    limit.current
                ),
                Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
            }
        }
    }
}

#[cfg(test)]
mod installation_tests {
    #[test]
    fn refused_install_names_all_mods_and_only_failed_targets() {
        assert_eq!(
            super::installation_refusals(&[
                ("game::tick", &["first", "second"], false),
                ("game::other", &["third"], true)
            ]),
            vec![
                "Horizon: first, second: game::tick: a patched copy was already installed; the game was not started"
            ]
        );
        assert!(super::installation_refusals(&[]).is_empty());
    }
}
