//! The wasm fixture's lending mod, registered natively: the same answers as `real_member`.
#[path = "fixture/src/lending.rs"]
mod lending;
#[path = "support/no_hooks.rs"]
mod no_hooks;

#[test]
fn collect_positions_copy_in_and_out_natively_as_in_the_real_guest() {
    struct Quiet;
    impl mod_api::Host for Quiet {}
    mod_api::impl_host_areas!(Quiet, except(items));
    impl mod_api::prelude::ItemsHost for Quiet {
        fn item_key(&self, id: &mod_api::Identifier) -> Option<mod_api::ItemKey> {
            mod_api::__private::item_key_by_identifier(id)
        }
    }
    impl mod_api::ServerHost for Quiet {}
    mod_api::impl_server_host_areas!(Quiet);
    mod_api::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "native_lender",
            Box::new(lending::Lender),
        )]
        .into(),
        || {
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            lending::check(&mut Quiet);
            mod_api::__private::resolve_items(["minecraft:bow"]).unwrap();
            lending::check_death(
                &mut Quiet,
                mod_api::__private::item_key("minecraft:bow").unwrap(),
            );
        },
    );
}
