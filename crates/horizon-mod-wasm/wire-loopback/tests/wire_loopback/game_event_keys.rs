use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::KeysHost;
use mod_api::{Backend, BackendRef, ClientHost, GameEventKey, GameEventKeyError, Host, ServerHost};

struct Registry;
impl Host for Registry {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Registry, except(keys));
impl mod_api::prelude::KeysHost for Registry {
    fn game_event_key(&self, id: &str) -> Option<GameEventKey> {
        mod_api::__private::game_event_key(id)
    }
}
impl Backend for Registry {}
mod_api::impl_backend_areas!(Registry, except(keys, data_maps));
impl mod_api::prelude::KeysBackend for Registry {
    fn game_event_key_id(&self, key: GameEventKey) -> Result<String, GameEventKeyError> {
        mod_api::__private::game_event_key_id(key).ok_or(GameEventKeyError::InvalidKey)
    }
}
impl mod_api::prelude::DataMapsBackend for Registry {
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<mod_api::Tag>, mod_api::DataMapError> {
        assert_eq!(registry, "minecraft:game_event");
        assert_eq!(map, "neoforge:vibration_frequencies");
        Ok((entry == "minecraft:step").then(|| {
            mod_api::Tag::Compound(std::collections::BTreeMap::from([(
                "frequency".into(),
                mod_api::Tag::Int(7),
            )]))
        }))
    }
}
struct Frequencies;
impl mod_api::DataMap for Frequencies {
    type Entry = GameEventKey;
    type Value = mod_api::Tag;
    fn id(&self) -> &str {
        "neoforge:vibration_frequencies"
    }
    fn decode(&self, value: &mod_api::Tag) -> Option<mod_api::Tag> {
        Some(value.clone())
    }
}
impl ServerHost for Registry {}
mod_api::impl_server_host_areas!(Registry);
impl ClientHost for Registry {}
mod_api::impl_client_host_areas!(Registry);

#[test]
fn game_event_key_lookup_and_id_cross_the_wire() {
    mod_api::__private::initialize().unwrap();
    let mods: mod_api::Mods = vec![].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        assert_eq!(Registry.game_event_key("minecraft:step"), None);
        mod_api::__private::resolve_game_events(["minecraft:step", "minecraft:swim", "test:chime"])
            .unwrap();
        let mut frame = boundary::Frame::default();
        frame.set_host(&Registry);
        let mut cx = boundary::Cx::default();
        cx.frames.push(frame);
        let cx = std::cell::RefCell::new(cx);
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            // SAFETY: the host frame lives for this synchronous transport.
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        for host in [&Registry as &dyn Host, &guest::Guest as &dyn Host] {
            assert_eq!(
                host.game_event_key("test:chime").unwrap().id(host),
                Ok("test:chime".into())
            );
            let step = host.game_event_key("minecraft:step").unwrap();
            assert_eq!(step.id(host), Ok("minecraft:step".into()));
            assert_eq!(host.game_event_key("test:nope"), None);
            assert_eq!(
                step.data(host, &Frequencies),
                Ok(Some(mod_api::Tag::Compound(
                    std::collections::BTreeMap::from([("frequency".into(), mod_api::Tag::Int(7))])
                )))
            );
            assert_eq!(
                host.game_event_key("minecraft:swim")
                    .unwrap()
                    .data(host, &Frequencies),
                Ok(None)
            );
            let mut bytes = Writer::new();
            bytes.put(&step);
            assert_eq!(Reader::decode::<GameEventKey>(bytes.bytes()).unwrap(), step);
        }
        assert!(Reader::decode::<GameEventKey>(&u32::MAX.to_le_bytes()).is_err());
        guest::set_transport(None);
    });
}
