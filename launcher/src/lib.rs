//! The launcher's core, shared by the GUI, the CLI and the MCP server.
pub mod account;
pub mod agents;
pub mod backend;
pub mod browser;
pub mod datapacks;
pub mod desktop;
pub mod desktop_seams;
pub mod export;
#[cfg(unix)]
pub mod game_app;
pub mod game_log;
pub mod heads;
pub mod import;
pub mod instances;
pub mod modpacks;
pub mod mojang;
pub mod packs;
pub mod process;
pub mod proxy;
pub mod screenshots;
pub mod server_list;
pub mod servers;
pub mod signed;
pub mod sources;
pub mod tailscale;
pub mod telemetry;
pub mod worlds;
