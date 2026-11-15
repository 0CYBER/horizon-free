//! Asks a capability named like counter_lib's, with a handler trait of its own: alpha, in
//! another group, answers counter_lib's, which this mod isn't linked with.
use mod_api::{
    CapabilityKey, Flow, Hooks, Registrar, RegistrationContext, ServerChatEvent, ServerHost, Text,
};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// This mod's own counter.
pub trait Counter: Send + Sync {
    /// Its count.
    fn count(&self) -> u32;
}

const COUNTER: CapabilityKey<dyn Counter> = CapabilityKey::new("test:counter");

/// Where the test world's counter box is.
const BOX: [i32; 3] = [0, 64, 0];

/// How many times the game ran this member's `register` and `server_chat`.
static REGISTERS: AtomicU32 = AtomicU32::new(0);
static CHATS: AtomicU32 = AtomicU32::new(0);

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, _registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        REGISTERS.fetch_add(1, Relaxed);
    }

    fn server_chat(&self, host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let answer = host
            .level("minecraft:overworld")
            .and_then(|level| level.capability(host, &COUNTER, BOX, None))
            .map_or("none".to_string(), |counter| counter.count().to_string());
        *event.message = Text::literal(format!(
            "{}: {answer} [registers={} chats={}]",
            env!("CARGO_PKG_NAME"),
            REGISTERS.load(Relaxed),
            CHATS.fetch_add(1, Relaxed) + 1
        ));
        Flow::Continue
    }
}
