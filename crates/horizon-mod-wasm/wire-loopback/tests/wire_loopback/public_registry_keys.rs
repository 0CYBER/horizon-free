use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::{Backend, BackendRef, BlockKey, ClientHost, EffectKey, Host, ServerHost};

struct Registry;
impl Host for Registry {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Registry, except(keys));
impl mod_api::prelude::KeysHost for Registry {
    fn block_key(&self, id: &str) -> Option<BlockKey> {
        mod_api::__private::block_key(id)
    }
    fn effect_key(&self, id: &str) -> Option<EffectKey> {
        mod_api::__private::effect_key(id)
    }
}
impl Backend for Registry {}
mod_api::impl_backend_areas!(Registry, except(keys));
impl mod_api::prelude::KeysBackend for Registry {
    fn block_key_id(&self, key: BlockKey) -> String {
        mod_api::__private::block_key_id(key).unwrap().to_owned()
    }
    fn effect_key_id(&self, key: EffectKey) -> String {
        mod_api::__private::effect_key_id(key).unwrap().to_owned()
    }
}
impl ServerHost for Registry {}
mod_api::impl_server_host_areas!(Registry);
impl ClientHost for Registry {}
mod_api::impl_client_host_areas!(Registry);

fn assert_keys(host: &dyn Host, complete: bool) {
    if complete {
        assert_eq!(
            host.block_key("minecraft:stone").unwrap().id(host),
            "minecraft:stone"
        );
        assert_eq!(
            host.effect_key("minecraft:speed").unwrap().id(host),
            "minecraft:speed"
        );
    } else {
        assert_eq!(host.block_key("minecraft:stone"), None);
        assert_eq!(host.effect_key("minecraft:speed"), None);
    }
    assert_eq!(host.block_key("test:missing_block"), None);
    assert_eq!(host.effect_key("test:missing_effect"), None);
}
fn assert_sides(server: &dyn ServerHost, client: &dyn ClientHost, complete: bool) {
    assert_eq!(server.block_key("minecraft:stone").is_some(), complete);
    assert_eq!(server.effect_key("minecraft:speed").is_some(), complete);
    assert_eq!(client.block_key("minecraft:stone").is_some(), complete);
    assert_eq!(client.effect_key("minecraft:speed").is_some(), complete);
}
#[test]
fn public_registry_keys_native_and_wasm_loopback() {
    mod_api::__private::initialize().unwrap();
    let mods: mod_api::Mods = vec![].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let mut frame = boundary::Frame::default();
        frame.set_host(&Registry);
        let mut cx = boundary::Cx::default();
        cx.frames.push(frame);
        let cx = std::cell::RefCell::new(cx);
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        for complete in [false, true] {
            if complete {
                let properties = std::collections::BTreeMap::new();
                mod_api::__private::resolve_block_keys(["minecraft:stone"], |_| {
                    horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                        .map_err(|e| format!("{e:?}"))
                })
                .unwrap();
                mod_api::__private::resolve_effects(["minecraft:speed"], |_| Some(0)).unwrap();
            }
            assert_keys(&Registry, complete);
            assert_sides(&Registry, &Registry, complete);
            assert_keys(&guest::Guest, complete);
            assert_sides(&guest::Guest, &guest::Guest, complete);
        }
        guest::set_transport(None);
    });
}
