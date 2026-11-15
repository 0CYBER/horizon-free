use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::{BlockEntityOnLoadEvent, BlockEntityPart, HostSide};
struct World(Arc<std::sync::atomic::AtomicUsize>);
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World);
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(block_entities));
impl mod_api::prelude::BlockEntitiesBackend for World {
    fn block_entity_data(&self, _: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(mod_api::Compound::from([(
            "count".into(),
            mod_api::Tag::Int(7),
        )]))
    }
}
struct Loaded;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Loaded {}
#[mod_api::horizon_part]
impl BlockEntityPart for Loaded {
    fn block_entity_on_load(&self, host: HostSide, event: &mut BlockEntityOnLoadEvent) {
        let HostSide::Server(host) = host else {
            panic!("wrong logical side")
        };
        assert_eq!(
            event.entity.data(host).unwrap().get(&"count".into()),
            Some(&mod_api::Tag::Int(7))
        );
    }
}
struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.block_entity_type("holder").build();
    }
}
#[test]
fn wire_loopback_block_entity_on_load_reads_saved_seven() {
    mod_api::testing::with_mods(
        vec![mod_api::ModEntry::new("test", Box::new(Register))].into(),
        || {
            mod_api::__private::resolve_block_entity_types(["test:holder"], |_| Some(())).unwrap();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let index =
                guest::add_handler(vec![guest::view::<dyn BlockEntityPart>(Arc::new(Loaded))]);
            let proxy = WasmHandler {
                index,
                module: WasmMod {
                    group: 0,
                    subscribed: &[],
                    listeners: &[],
                    corrupt: false,
                },
            };
            let entity = mod_api::__private::game::block_entity_ref(
                mod_api::LevelRef::new(
                    mod_api::WorldEpoch::new(1),
                    mod_api::DimensionKey::OVERWORLD,
                ),
                [1, 64, 1],
                1,
            );
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut world = World(reads.clone());
            proxy.block_entity_on_load(
                HostSide::Server(&mut world),
                &mut BlockEntityOnLoadEvent {
                    id: mod_api::__private::block_entity_type_key("test:holder").unwrap(),
                    entity,
                },
            );
            assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        },
    );
}
