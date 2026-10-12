//! The dedicated server's files and settings, ported from
//! net.minecraft.server.dedicated and net.minecraft.server.players:
//! server.properties, eula.txt and the four user lists. The server itself
//! runs the game's own world, simulation and LAN hub headless
//! (`horizon-client --dedicated`, see the game's `dedicated` module).
pub mod eula;
pub mod java_properties;
pub mod json;
pub mod properties;
pub mod user_lists;
