use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::KeysHost;
use mod_api::{Backend, BackendRef, BiomeKey, BiomeKeyError, DataMap, DataMapError, Host, Tag};
struct Biomes {
    keys: mod_api::__private::BiomeKeys,
    value: &'static str,
}
impl Host for Biomes {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Biomes, except(keys));
impl mod_api::prelude::KeysHost for Biomes {
    fn biome_key(&self, id: &str) -> Option<BiomeKey> {
        self.keys.key(id)
    }
}
impl Backend for Biomes {}
mod_api::impl_backend_areas!(Biomes, except(keys, data_maps));
impl mod_api::prelude::KeysBackend for Biomes {
    fn biome_key_id(&self, key: BiomeKey) -> Result<String, BiomeKeyError> {
        self.keys.id(key).map(str::to_owned)
    }
}
impl mod_api::prelude::DataMapsBackend for Biomes {
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<Tag>, DataMapError> {
        assert_eq!(registry, "minecraft:worldgen/biome");
        assert_eq!(map, "neoforge:villager_types");
        Ok((entry == "minecraft:plains").then(|| {
            Tag::Compound(std::collections::BTreeMap::from([(
                "villager_type".into(),
                Tag::String(self.value.into()),
            )]))
        }))
    }
}
// A client or a server before world data installation: no registry or maps.
struct Unavailable;
impl Host for Unavailable {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Unavailable, except(keys));
impl mod_api::prelude::KeysHost for Unavailable {
    fn biome_key(&self, _: &str) -> Option<BiomeKey> {
        None
    }
}
impl Backend for Unavailable {}
mod_api::impl_backend_areas!(Unavailable, except(keys, data_maps));
impl mod_api::prelude::KeysBackend for Unavailable {
    fn biome_key_id(&self, _: BiomeKey) -> Result<String, BiomeKeyError> {
        Err(BiomeKeyError::Unavailable)
    }
}
impl mod_api::prelude::DataMapsBackend for Unavailable {
    fn data_map_value(&self, _: &str, _: &str, _: &str) -> Result<Option<Tag>, DataMapError> {
        Err(DataMapError::Unavailable)
    }
}

struct VillagerTypes;
impl DataMap for VillagerTypes {
    type Entry = BiomeKey;
    type Value = Tag;
    fn id(&self) -> &str {
        "neoforge:villager_types"
    }
    fn decode(&self, value: &Tag) -> Option<Tag> {
        Some(value.clone())
    }
}
fn assert_biomes(host: &dyn Host, value: &str) {
    let plains = host.biome_key("minecraft:plains").unwrap();
    assert_eq!(plains.id(host), Ok("minecraft:plains".into()));
    assert_eq!(host.biome_key("test:nope"), None);
    assert_eq!(
        plains.data(host, &VillagerTypes),
        Ok(Some(Tag::Compound(std::collections::BTreeMap::from([(
            "villager_type".into(),
            Tag::String(value.into())
        )]))))
    );
    assert_eq!(
        host.biome_key("minecraft:desert")
            .unwrap()
            .data(host, &VillagerTypes),
        Ok(None)
    );
}
#[test]
fn biome_data_maps_native_and_wire_loopback() {
    mod_api::__private::initialize().unwrap();
    mod_api::testing::with_mods(vec![].into(), || {
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        for value in ["minecraft:plains", "minecraft:savanna"] {
            let host = Biomes {
                keys: mod_api::__private::BiomeKeys::new(
                    1,
                    ["minecraft:plains", "minecraft:desert"],
                ),
                value,
            };
            assert_biomes(&host, value);
            let mut frame = boundary::Frame::default();
            frame.set_host(&host);
            let mut cx = boundary::Cx::default();
            cx.frames.push(frame);
            let cx = std::cell::RefCell::new(cx);
            guest::set_transport(Some(Rc::new(move |id, bytes| {
                let mut out = Writer::new();
                unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
                out.into_bytes()
            })));
            assert_biomes(&guest::Guest, value);
            // World A remains alive while its token is sent to receiving world B.
            let other = Biomes {
                keys: mod_api::__private::BiomeKeys::new(2, ["minecraft:plains"]),
                value,
            };
            let foreign = other.biome_key("minecraft:plains").unwrap();
            assert_eq!(foreign.id(&host), Err(BiomeKeyError::WrongGeneration));
            assert_eq!(
                foreign.id(&guest::Guest),
                Err(BiomeKeyError::WrongGeneration)
            );
            assert_eq!(
                foreign.data(&host, &VillagerTypes),
                Err(DataMapError::InvalidKey)
            );
            assert_eq!(
                foreign.data(&guest::Guest, &VillagerTypes),
                Err(DataMapError::InvalidKey)
            );
            let expired = {
                let old = mod_api::__private::BiomeKeys::new(3, ["minecraft:plains"]);
                old.key("minecraft:plains").unwrap()
            };
            assert_eq!(
                expired.id(&guest::Guest),
                Err(BiomeKeyError::WrongGeneration)
            );
            assert_eq!(
                expired.data(&guest::Guest, &VillagerTypes),
                Err(DataMapError::InvalidKey)
            );
            let key = host.biome_key("minecraft:plains").unwrap();
            assert_eq!(
                key.data(&Unavailable, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );
            let mut frame = boundary::Frame::default();
            frame.set_host(&Unavailable);
            let mut cx = boundary::Cx::default();
            cx.frames.push(frame);
            let cx = std::cell::RefCell::new(cx);
            guest::set_transport(Some(Rc::new(move |id, bytes| {
                let mut out = Writer::new();
                unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
                out.into_bytes()
            })));
            assert_eq!(guest::Guest.biome_key("minecraft:plains"), None);
            assert_eq!(key.id(&guest::Guest), Err(BiomeKeyError::Unavailable));
            assert_eq!(
                key.data(&guest::Guest, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );
            guest::set_transport(None);
        }
    });
}
