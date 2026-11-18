//! Vanilla's logical side: the gameplay the server jar holds, common code included.
#![forbid(unsafe_code)]
// Listed in portable.pending (tools/layers.json) until the types it needs move to portable crates.
extern crate horizon_client_lib as horizon_client;
pub mod cat_spawner;
pub mod dedicated;
pub mod interaction_shapes;
pub mod item_areas;
pub mod lan;
pub mod mob_index;
pub mod tick_profile;
pub mod wandering_trader;
