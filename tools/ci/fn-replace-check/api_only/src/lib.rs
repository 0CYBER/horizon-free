//! An API-only mod: hooks and one `[[inject]]` listener. Alone it takes the API route (compiled
//! against the API metadata into its own library); beside a mod with a `[native]` table row the
//! whole set takes the engine route. It runs the same on both.
use mod_api::{Hooks, ServerHost};

struct Mod;

impl Default for Mod {
    fn default() -> Self {
        eprintln!("fnr: api_only constructed");
        Mod
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
        // A libm call (`sinf`): the mods library's link must bring the C maths library.
        let angle = std::hint::black_box(0.5f32);
        eprintln!("fnr: api_only server_started, sin {:.3}", angle.sin());
    }
}

pub fn entry_listener(_host: mod_api::HostSide<'_>, _args: &mut mod_points::TickBlockEntitiesArgs) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("fnr: api_only listener at tick_block_entities"));
}
