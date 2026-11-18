//! Vanilla's logical side: the gameplay the server jar holds, common code included.
#![forbid(unsafe_code)]
// Listed in portable.pending (tools/layers.json) until the types it needs move to portable crates.
extern crate horizon_client_lib as horizon_client;
pub mod allays;
pub mod api_values;
pub mod block_entity_ticks;
pub mod boats;
pub mod cat_spawner;
pub mod commands;
pub mod custom_spawner;
pub mod dedicated;
pub mod dim;
pub mod execution_role;
pub mod fireworks;
pub mod foxes;
pub mod interaction_shapes;
pub mod item_areas;
pub mod lan;
pub mod minecarts;
pub mod mob_index;
pub mod mob_level;
pub mod mob_looting;
pub mod panda_level;
pub mod pandas;
pub mod piglin_level;
pub mod piglins;
pub mod stats;
pub mod tick_profile;
pub mod tnt;
pub mod trident;
pub mod village_siege;
pub mod villager_level;
pub mod villagers;
pub mod wandering_trader;
