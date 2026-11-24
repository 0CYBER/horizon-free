//! The official game: the platform's game (`horizon-game`) with Horizon's own in-game parts
//! plugged in through the platform's hooks (docs/layers.md, "How a Horizon crate plugs in").
//! The release builds ship its executable under the game's name, `horizon-client`
//! (`game-build.yml`); horizon-client keeps its own plain executable.
//!
//! Each Horizon crate that runs in the game gives a `&'static` value implementing
//! `horizon_platform::hooks::Hooks`, and its settings store and pages if it has any, and
//! [`register`] adds them. Nothing here depends on an engine crate.
#![forbid(unsafe_code)]

use horizon_platform::hooks::{self, Hooks};
use horizon_platform::{mod_links, pages, settings};

/// Every Horizon part in the game, in the order its hooks run.
const PARTS: [&dyn Hooks; 6] = [
    // The anonymous "I'm playing" ping.
    &horizon_heartbeat::HOOKS,
    // Play with Friends: `friend:` addresses join through its tunnel.
    &horizon_friends::game::HOOKS,
    // The official server in the player's server list.
    &horizon_official_servers::HOOKS,
    // "Playing Horizon" on the player's Discord profile.
    &horizon_discord::HOOKS,
    // The player's own name over their head in third person (docs/extras.md).
    &horizon_name_tag::HOOKS,
    // Open and Share for saved performance captures.
    &horizon_perf_share::HOOKS,
];
/// Their settings, in the player's settings files.
const STORES: [&dyn settings::Store; 2] = [&horizon_discord::SETTING, &horizon_name_tag::SETTING];
/// Their rows and pages in the options screens.
const PAGES: [&dyn pages::Page; 2] = [&horizon_discord::SECTION, &horizon_name_tag::SECTION];

/// The product chooses the built-in Mods screen entries' outside links.
static MOD_LINKS: [mod_links::Links; 2] = [
    mod_links::Links {
        id: "minecraft",
        license: None,
        homepage: Some(horizon_addresses::MINECRAFT_WEBSITE),
        issues: Some(horizon_addresses::MINECRAFT_ISSUES),
    },
    mod_links::Links {
        id: "horizon",
        license: Some(horizon_addresses::LICENSE),
        homepage: Some(horizon_addresses::WEBSITE),
        issues: Some(horizon_addresses::ISSUES),
    },
];

/// Registers Horizon's in-game parts with the game. The executable calls it before it runs the
/// game, and so does the launcher's generated executable for native mods.
pub fn register() {
    // Horizon's mark in the windows and the taskbar (branding: the plain game keeps the default
    // icon), and the menus' look, here too for the native-mod executable, which starts the game
    // without `horizon_game::main`.
    horizon_game::set_icon_png(horizon_theme::ICON_PNG);
    horizon_game::give_menu_look();
    let late = PARTS.into_iter().map(hooks::register);
    let late = late.chain(STORES.into_iter().map(settings::register));
    let late = late.chain(MOD_LINKS.iter().map(mod_links::register));
    for result in late.chain(PAGES.into_iter().map(pages::register)) {
        if let Err(e) = result {
            // The executable registered after starting the game: a bug, not a player's problem.
            log::warn!("Horizon: an in-game part did not register: {e}");
        }
    }
}
