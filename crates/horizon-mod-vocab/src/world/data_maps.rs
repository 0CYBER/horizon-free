//! Typed reads of data maps (NeoForge's `DataMapType` and `IWithData.getData`): the value the
//! server's data packs and mods' files gave a registry entry, after every pack was applied.
use crate::{
    Backend, BlockEntityTypeKey, BlockKey, DataMapType, EffectKey, EnchantmentKey, EntityTypeKey,
    FluidKey, Host, ItemKey, NoBackend, ProfessionKey,
};
use horizon_core::nbt::Tag;

/// A data map a mod registers and reads, typed (NeoForge's `DataMapType<R, T>`): its id, the
/// registry its entries are in (through [`DataMap::Entry`]), and how a value decodes.
///
/// Builtin maps are already registered. They return canonical codec NBT compounds,
/// including Float distances (with infinity preserved) and Int frequencies. This store
/// is server-only, shared by native and wasm hosts, not network-synced to clients.
/// Mod-defined maps retain the JSON-to-NBT mapping below.
///
/// Register a mod-defined map in `data_map_types` with [`DataMap::ty`], then read an entry's value with its
/// key's `data` ([`ItemKey::data`], [`EntityTypeKey::data`]...), or by name with
/// [`data_map_value`] and [`data_map_entries`] for a registry without a typed key. A value arrives as the NBT form of the JSON a file
/// held: a whole number is an `Int`, or a `Long` past `i32`, a fraction a `Double`, a boolean a
/// `Byte`, an object a `Compound` and an array a `List`. Integers outside signed
/// 64-bit range and `null` at any nesting depth return [`DataMapError::Undecodable`]
/// rather than changing the value. Fractional/exponent JSON tokens use the parser's
/// `f64` value; empty and heterogeneous arrays retain their elements.
///
/// ```
/// use mod_api::{DataMap, ItemKey, Tag};
/// /// `projecte:emc`: an item's EMC value.
/// struct Emc;
/// impl DataMap for Emc {
///     type Entry = ItemKey;
///     type Value = i64;
///     fn id(&self) -> &str {
///         "projecte:emc"
///     }
///     fn decode(&self, value: &Tag) -> Option<i64> {
///         match value {
///             Tag::Int(value) => Some(i64::from(*value)),
///             Tag::Long(value) => Some(*value),
///             _ => None,
///         }
///     }
/// }
/// assert_eq!(Emc.ty().registry, "minecraft:item");
/// ```
pub trait DataMap {
    /// The registry entries the map attaches values to: one of the [`DataMapEntry`] keys.
    type Entry: DataMapEntry;
    /// The value an entry holds.
    type Value;
    /// The map's id, `mod_id:name`, not in the `minecraft` namespace, as registered.
    fn id(&self) -> &str;
    /// Decode an entry's value, refusing one of another shape.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// The type to push in `data_map_types`: this map's id, for its entries' registry.
    fn ty(&self) -> DataMapType {
        DataMapType {
            id: self.id().to_owned(),
            registry: <Self::Entry as entry::Entry>::REGISTRY.to_owned(),
        }
    }
}

/// A registry entry a data map can attach a value to through a typed key. Sealed: a registry
/// without one is read by name with [`data_map_value`].
pub trait DataMapEntry: entry::Entry {}

mod entry {
    use super::*;

    /// The registry an entry is in, and its id there through the game's backend.
    pub trait Entry {
        /// The registry's id.
        const REGISTRY: &'static str;
        /// The entry's full registry id.
        fn id(&self, backend: &dyn Backend) -> String;
    }
}

/// `DataMapEntry` and `data` for a key whose id the keys backend answers.
macro_rules! typed_entries {
    ($($key:ty => $registry:literal, $id:ident;)+) => {$(
        impl DataMapEntry for $key {}
        impl entry::Entry for $key {
            const REGISTRY: &'static str = $registry;
            fn id(&self, backend: &dyn Backend) -> String {
                backend.$id(*self)
            }
        }
        impl $key {
            /// This entry's value in data map `map` (`Holder.getData`), as the server's data
            /// packs and mods' files left it: the newest file wins, a `#tag` entry gives every
            /// entry in the tag its value, `replace` and `remove` apply, and conditional files
            /// and values count only when their conditions hold. `Ok(None)` where the map
            /// gives this entry no value.
            ///
            /// Server side: the server merges the maps when its data loads, on a world's first
            /// open and after every `/reload`, and a read sees the last load whole, never a mix
            /// of two. Readable from `data_maps_updated` on, in the world and while data
            /// reloads. A reload that drops a value drops it here too.
            pub fn data<M: DataMap<Entry = $key>>(
                self,
                host: &dyn Host,
                map: &M,
            ) -> Result<Option<M::Value>, DataMapError> {
                data(&self, host, map)
            }
        }
    )+};
}
typed_entries! {
    ItemKey => "minecraft:item", item_key_id;
    BlockKey => "minecraft:block", block_key_id;
    FluidKey => "minecraft:fluid", fluid_key_id;
    EffectKey => "minecraft:mob_effect", effect_key_id;
    EntityTypeKey => "minecraft:entity_type", entity_type_key_id;
    EnchantmentKey => "minecraft:enchantment", enchantment_key_id;
    BlockEntityTypeKey => "minecraft:block_entity_type", block_entity_type_key_id;
}
impl DataMapEntry for ProfessionKey {}
impl entry::Entry for ProfessionKey {
    const REGISTRY: &'static str = "minecraft:villager_profession";
    fn id(&self, _: &dyn Backend) -> String {
        self.0.to_string()
    }
}
impl ProfessionKey {
    /// This profession's value in data map `map`, as for [`ItemKey::data`].
    pub fn data<M: DataMap<Entry = ProfessionKey>>(
        &self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}

/// Why a data map read has no answer. An entry the map gives no value is `Ok(None)`, not one of
/// these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DataMapError {
    /// No data maps are loaded here: a client (data maps are not synced to clients), a server
    /// before its world's data first loads, or a host with no game behind it.
    Unavailable,
    /// No mod registered the map for the entry's registry in the current data load
    /// (`data_map_types`).
    UnknownType,
    /// The entry's value is not one the map's [`DataMap::decode`] takes.
    Undecodable,
}
impl NoBackend for DataMapError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// `entry`'s value in `map`, decoded.
fn data<M: DataMap>(
    entry: &M::Entry,
    host: &dyn Host,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    let id = entry::Entry::id(entry, backend);
    let registry = <M::Entry as entry::Entry>::REGISTRY;
    match backend.data_map_value(registry, &id, map.id())? {
        Some(value) => map
            .decode(&value)
            .map(Some)
            .ok_or(DataMapError::Undecodable),
        None => Ok(None),
    }
}

/// Entry `entry` (its full id, `minecraft:pig`) of registry `registry`: its raw value in data
/// map `map` (`IRegistryExtension.getData`), for a registry without a typed key, read as a
/// key's `data` reads it. The value is the map's NBT, not yet decoded.
pub fn data_map_value(
    host: &dyn Host,
    registry: &str,
    entry: &str,
    map: &str,
) -> Result<Option<Tag>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_value(registry, entry, map)
}

/// Every entry of `registry` that data map `map` gives a value, with that value, in id order
/// (`IRegistryExtension.getDataMap`): the whole merged map of the last data load.
pub fn data_map_entries(
    host: &dyn Host,
    registry: &str,
    map: &str,
) -> Result<Vec<(String, Tag)>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_entries(registry, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host whose data maps hold `test:emc` for items, coal at 10 and a stick at a string,
    /// and `test:hardness` for blocks, stone at 3; every other read answers no value.
    struct Maps;
    impl crate::host::data_maps::DataMapsBackend for Maps {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<Tag>, DataMapError> {
            match (registry, map, entry) {
                ("minecraft:item", "test:emc", "test:coal") => Ok(Some(Tag::Int(10))),
                ("minecraft:item", "test:emc", "test:stick") => Ok(Some(Tag::String("ten".into()))),
                ("minecraft:item", "test:emc", _) => Ok(None),
                ("minecraft:block", "test:hardness", "test:stone") => Ok(Some(Tag::Long(3))),
                ("minecraft:block", "test:hardness", _) => Ok(None),
                _ => Err(DataMapError::UnknownType),
            }
        }
        fn data_map_entries(
            &self,
            registry: &str,
            map: &str,
        ) -> Result<Vec<(String, Tag)>, DataMapError> {
            match (registry, map) {
                ("minecraft:block", "test:hardness") => {
                    Ok(vec![("test:stone".into(), Tag::Long(3))])
                }
                _ => Err(DataMapError::UnknownType),
            }
        }
    }
    impl crate::host::keys::KeysBackend for Maps {
        fn item_key_id(&self, key: ItemKey) -> String {
            ["test:coal", "test:stick", "test:air"][key.0 as usize].into()
        }
        fn block_key_id(&self, key: BlockKey) -> String {
            ["test:stone", "test:dirt"][key.0 as usize].into()
        }
    }
    impl Host for Maps {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            Some(crate::BackendRef::of(self))
        }
    }
    crate::impl_host_areas!(Maps);
    crate::impl_backend_areas!(Maps, except(data_maps, keys));
    impl Backend for Maps {}

    /// No game behind it.
    struct Bare;
    impl Host for Bare {}
    crate::impl_host_areas!(Bare);

    fn number(value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
    struct Emc;
    impl DataMap for Emc {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:emc"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    struct Hardness;
    impl DataMap for Hardness {
        type Entry = BlockKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:hardness"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    /// A map no mod registered.
    struct Unregistered;
    impl DataMap for Unregistered {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:unregistered"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }

    #[test]
    fn a_read_decodes_the_value_and_tells_no_value_from_a_refusal() {
        assert_eq!(ItemKey(0).data(&Maps, &Emc), Ok(Some(10)));
        assert_eq!(BlockKey(0).data(&Maps, &Hardness), Ok(Some(3)));
        assert_eq!(ItemKey(2).data(&Maps, &Emc), Ok(None));
        assert_eq!(BlockKey(1).data(&Maps, &Hardness), Ok(None));
        assert_eq!(ItemKey(1).data(&Maps, &Emc), Err(DataMapError::Undecodable));
        assert_eq!(
            ItemKey(0).data(&Maps, &Unregistered),
            Err(DataMapError::UnknownType)
        );
        assert_eq!(ItemKey(0).data(&Bare, &Emc), Err(DataMapError::Unavailable));
        assert_eq!(
            BlockKey(0).data(&Bare, &Hardness),
            Err(DataMapError::Unavailable)
        );
    }

    #[test]
    fn a_maps_type_names_its_entries_registry() {
        assert_eq!(
            Emc.ty(),
            DataMapType {
                id: "test:emc".into(),
                registry: "minecraft:item".into(),
            }
        );
        assert_eq!(Hardness.ty().registry, "minecraft:block");
    }

    #[test]
    fn a_read_by_name_reaches_the_whole_map() {
        assert_eq!(
            data_map_value(&Maps, "minecraft:block", "test:stone", "test:hardness"),
            Ok(Some(Tag::Long(3)))
        );
        assert_eq!(
            data_map_entries(&Maps, "minecraft:block", "test:hardness"),
            Ok(vec![("test:stone".into(), Tag::Long(3))])
        );
        assert_eq!(
            data_map_entries(&Bare, "minecraft:block", "test:hardness"),
            Err(DataMapError::Unavailable)
        );
    }
}
