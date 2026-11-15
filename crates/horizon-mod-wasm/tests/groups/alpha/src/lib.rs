//! Answers counter_lib's capability for `test:counter_box` block entities, and asks it.
use counter_lib::{COUNTER, Counter};
use mod_api::{
    BoolGameRuleKey, Flow, Hooks, Registrar, RegistrationContext, ServerChatEvent, ServerHost, Text,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
/// The test harness registers every mod once to collect its controllers and again to install it,
/// so each registration replaces the key.
static RULE: Mutex<Option<BoolGameRuleKey>> = Mutex::new(None);

/// Where the test world's counter box is.
const BOX: [i32; 3] = [0, 64, 0];

struct Seven;
impl Counter for Seven {
    fn count(&self) -> u32 {
        7
    }
}

/// How many times the game ran this member's `register` and `server_chat`.
static REGISTERS: AtomicU32 = AtomicU32::new(0);
static CHATS: AtomicU32 = AtomicU32::new(0);

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        REGISTERS.fetch_add(1, Relaxed);
        registrar.game_rule_category("machines").expect("category");
        *RULE.lock().unwrap() = Some(
            registrar
                .game_rule_bool("enabled", "alpha:machines", true)
                .expect("rule"),
        );
        let kind = registrar.block_entity_type_key("test:counter_box");
        registrar.block_entity_capability(&COUNTER, &kind, |_, _| {
            Some(Box::new(Seven) as Box<dyn Counter>)
        });
    }

    fn server_chat(&self, host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let answer = host
            .level("minecraft:overworld")
            .and_then(|level| level.capability(host, &COUNTER, BOX, None))
            .map_or("none".to_string(), |counter| counter.count().to_string());
        let rule = host
            .level("minecraft:overworld")
            .and_then(|level| level.game_rule_bool(host, RULE.lock().unwrap().unwrap()))
            .unwrap();
        *event.message = Text::literal(format!(
            "{}: {answer} {rule} [registers={} chats={}]",
            env!("CARGO_PKG_NAME"),
            REGISTERS.load(Relaxed),
            CHATS.fetch_add(1, Relaxed) + 1
        ));
        Flow::Continue
    }
}
