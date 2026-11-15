//! Exported selector resources keep the same source, candidate and predicate result.
use super::*;
use mod_api::{CommandSource, EntityRef, EntitySelectorType, SelectorBase, SelectorSort};
struct Allies;
impl EntitySelectorType for Allies {
    fn base(&self) -> SelectorBase {
        SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.allies")
    }
    fn matches(
        &self,
        _: &mut dyn mod_api::ServerHost,
        source: &CommandSource,
        entity: EntityRef,
    ) -> bool {
        matches!(source, CommandSource::Console { .. }) && entity.id() == 12
    }
}
#[test]
fn wire_loopback_entity_selector_resource_preserves_base_source_and_candidate() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    let mut out = Writer::new();
    {
        let _resources = guest::resource_scope();
        let handler: Arc<dyn EntitySelectorType> = Arc::new(Allies);
        out.put(&handler);
    }
    assert_eq!(out.error(), None);
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut cx = boundary::Cx::default();
    cx.module = &module;
    let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
    let handler = Reader::decode::<Arc<dyn EntitySelectorType>>(out.bytes()).unwrap();
    assert_eq!(
        handler.base(),
        SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.allies")
    );
    let source = CommandSource::Console {
        permission: mod_api::PermissionLevel::from_level(2),
    };
    assert!(handler.matches(
        &mut World,
        &source,
        EntityRef::new(mod_api::WorldEpoch::new(1), 12)
    ));
    assert!(!handler.matches(
        &mut World,
        &source,
        EntityRef::new(mod_api::WorldEpoch::new(1), 11)
    ));
}
