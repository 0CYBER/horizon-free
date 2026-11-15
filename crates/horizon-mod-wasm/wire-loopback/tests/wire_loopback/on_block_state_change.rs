//! The native listener and real generated guest dispatcher read both predicted values.
use super::*;
#[path = "../../../tests/fixture/src/block_state_notice.rs"]
mod fixture;
#[path = "../../../tests/support/block_state_notice.rs"]
mod support;

struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.block_key("minecraft:stone");
        r.block_key("minecraft:air");
    }
}
#[test]
fn wire_loopback_on_block_state_change_reads_predicted_like_native() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("notice", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let world = support::World::new();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        world.check(&fixture::NoticePart);
        let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockPart>(Arc::new(
            fixture::NoticePart,
        ))]);
        let proxy = WasmHandler {
            index,
            module: WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            },
        };
        world.check(&proxy);
    });
}
