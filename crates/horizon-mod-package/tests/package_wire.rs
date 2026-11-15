use horizon_mod_package::content::{FluidLook, WorldFluid};

#[test]
fn package_wire_layout_identity() {
    fn same<T: mod_api::wire::Wire + mod_vocab::wire::Wire>(expected: u64) {
        let facade = <T as mod_api::wire::Wire>::layout(&mut vec![]);
        let vocabulary = <T as mod_vocab::wire::Wire>::layout(&mut vec![]);
        assert_eq!(facade, vocabulary);
        assert_eq!(facade, expected);
        assert_eq!(facade, <T as mod_vocab::wire::Wire>::layout(&mut vec![]));
    }
    same::<WorldFluid>(3141788496271527564);
    same::<FluidLook>(11310991335631905293);
}
