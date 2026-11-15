//! Two `[[native.loop]]` slots in `block_entities.tick` (`src/loops/`), compiled into the launcher's
//! copy of the loop, and a HEAD mixin on the loop's entry (`src/mixins/`). Its block
//! `fnr_loop:counter` has a ticking block entity of its own type, which the `count` slot picks out.
#[derive(Default)]
struct Mod;

/// The counter's block and its block entity type.
const COUNTER: &str = "fnr_loop:counter";

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("counter").blocks(&[COUNTER]).build();
        r.block_entity_part(&kind, Counter).unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new(COUNTER, "minecraft:stone"));
    }
}

/// Makes the counter's block entity tick.
struct Counter;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Counter {}
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Counter {
    fn block_entity_ticks(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::BlockEntityTicksEvent<'_>,
    ) {
        if event.block == COUNTER {
            event.ticks = true;
        }
    }
}

/// Called by the helpers: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: loop {what}");
}

static ENTRY_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The API listener at the loop's entry, `[[inject]] listen`.
pub fn entry_listener(_host: mod_api::HostSide<'_>, _args: &mut mod_points::TickBlockEntitiesArgs) {
    let calls = ENTRY_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if calls % 200 == 1 {
        note(&format!(
            "api head listener at tick_block_entities, call {calls}"
        ));
    }
}
