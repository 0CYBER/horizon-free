//! The launcher's core, shared by the GUI, the CLI and the MCP server.
pub mod account;
pub mod agents;
pub mod backend;
pub mod browser;
pub mod desktop;
#[cfg(unix)]
pub mod game_app;
pub mod heads;
pub mod modpacks;
pub mod mojang;
pub mod process;
pub mod servers;
pub mod signed;
pub mod sources;
pub mod tailscale;
pub mod telemetry;
