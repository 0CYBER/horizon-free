use super::*;
use mod_api::{BlockMetadata, BlockPart, BlockState, PistonPushReactionEvent, PushReaction};
thread_local! { pub(super) static UNKNOWN_REACTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
struct Part;
#[mod_api::horizon_part]
impl BlockPart for Part {
    fn piston_push_reaction(&self, _: &mut PistonPushReactionEvent) -> Option<PushReaction> {
        Some(PushReaction::Push)
    }
}
struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.block_key("test:block");
    }
}
#[test]
fn wire_loopback_piston_push_reaction_and_unknown_keeps_properties() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let properties = std::collections::BTreeMap::new();
        mod_api::__private::resolve_block_keys(["test:block"], |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        let state = BlockState::from(
            metadata
                .of(mod_api::__private::part_tests::block_key("test:block"), &[])
                .unwrap(),
        );
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let index = guest::add_handler(vec![guest::view::<dyn BlockPart>(Arc::new(Part))]);
        let proxy = WasmHandler {
            index,
            module: WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            },
        };
        let mut event = PistonPushReactionEvent { state };
        assert_eq!(
            proxy.piston_push_reaction(&mut event),
            Some(PushReaction::Push)
        );
        UNKNOWN_REACTION.set(true);
        assert_eq!(
            proxy.piston_push_reaction(&mut event),
            None,
            "bad enum answer retains the properties fallback"
        );
        UNKNOWN_REACTION.set(false);
    });
}
