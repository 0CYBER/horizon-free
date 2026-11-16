//! Asks counter_lib's capability, which a mod of its group answers.
use counter_lib::COUNTER;
use mod_api::{Flow, Hooks, Registrar, RegistrationContext, ServerChatEvent, ServerHost, Text};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

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
