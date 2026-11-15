//! Scanner input only: fixture points never enter a production catalog.
use horizon_client_core::text::Text;
use horizon_world::registry::BlockStateId;
use mod_api::RegistrationData;

fn sum(a: u32, b: &u32) -> u32 {
    a + *b
}
fn mirrors(text: &Text, first: &BlockStateId, second: &BlockStateId) {}
fn no_wire(value: &RegistrationData) {}
