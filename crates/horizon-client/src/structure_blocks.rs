//! Block entities placed by world generation (structure chests, barrels and
//! template containers): turned into client containers, with loot tables
//! unpacked as `RandomizableContainer.unpackLootTable` would on first open.
use crate::{containers::BlockEntity, items::ItemStack};
use std::{
    cell::RefCell,
    path::Path,
    sync::{Arc, Mutex},
};
use {
    horizon_core::nbt::{Compound, JavaString, Tag},
    horizon_world::{item_defaults::ItemDefaults, loot::LootData, worldgen::generator::ProtoChunk},
};

/// The `custom_data` key holding an exploration map's search until the game
/// runs it (vanilla runs `ExplorationMapFunction` when the chest is unpacked,
/// on first access; the search is deferred until the game can resolve it).
const PENDING_MAP: &str = "horizon:exploration_map";
const CUSTOM_DATA: &str = "minecraft:custom_data";

/// A generated chest's item, with its exploration map search kept in
/// `minecraft:custom_data` (which vanilla loads as is).
fn loot_item(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(s.item.as_str()),
        count: s.count.max(0) as u32,
        damage: s.damage.max(0) as u32,
        extra: None,
    };
    if let Some(map) = &s.exploration {
        let search = Tag::compound([
            ("destination", Tag::String(map.destination.as_str().into())),
            ("decoration", Tag::String(map.decoration.as_str().into())),
            ("zoom", Tag::Byte(map.zoom as i8)),
            ("search_radius", Tag::Int(map.search_radius)),
            (
                "skip_existing_chunks",
                Tag::Byte(i8::from(map.skip_existing_chunks)),
            ),
        ]);
        item.data_mut()
            .other
            .push((CUSTOM_DATA.into(), Tag::compound([(PENDING_MAP, search)])));
    }
    item
}

/// A rolled stack as an item with its enchantments (stored ones on a
/// book), potion and ominous bottle level.
pub fn loot_item_with_components(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = loot_item(s);
    for (id, level) in &s.enchantments {
        let level = (*level).max(0) as u32;
        if &*item.item == "minecraft:enchanted_book" {
            crate::item_data::set_level(&mut item.data_mut().stored_enchantments, id, level);
        } else {
            item.enchant(id, level);
        }
    }
    if let Some(potion) = &s.potion {
        item = item.with_potion(potion);
    }
    if let Some(level) = s.ominous_amplifier.filter(|l| *l > 0) {
        item.data_mut().ominous_amplifier = Some(level.clamp(0, 4) as u8);
    }
    // set_stew_effect's `suspicious_stew_effects`.
    for (effect, duration) in &s.stew_effects {
        item.data_mut()
            .stew_effects
            .push((Arc::from(effect.as_str()), *duration));
    }
    for (id, text) in &s.names {
        item.data_mut().set_loot_name(id, text);
    }
    for (id, value) in &s.components {
        item.data_mut().set_loot_component(id, value);
    }
    item
}

/// The exploration map search an item still waits for, removed from it.
pub fn take_pending_map(item: &mut ItemStack) -> Option<horizon_world::loot::ExplorationMap> {
    let data = item.extra.as_deref_mut()?;
    let i = data
        .other
        .iter()
        .position(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())?;
    let (_, mut custom) = data.other.remove(i);
    let search = match &mut custom {
        Tag::Compound(c) => c.remove(&JavaString::from(PENDING_MAP))?,
        _ => return None,
    };
    if matches!(&custom, Tag::Compound(c) if !c.is_empty()) {
        data.other.insert(i, (CUSTOM_DATA.into(), custom));
    }
    let text = |k: &str| match search.get(k) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    let int = |k: &str| match search.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    Some(horizon_world::loot::ExplorationMap {
        destination: text("destination")?,
        decoration: text("decoration")?,
        zoom: int("zoom").unwrap_or(2).clamp(0, 4) as u8,
        search_radius: int("search_radius").unwrap_or(50),
        skip_existing_chunks: int("skip_existing_chunks").unwrap_or(1) != 0,
    })
}

/// Whether an item still waits for its exploration map search.
pub fn has_pending_map(item: &ItemStack) -> bool {
    item.data().is_some_and(|d| {
        d.other
            .iter()
            .any(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())
    })
}

/// The loot data and the [`loot_key`] it was loaded for; None until
/// `loot_data` runs. `Some((_, None))` when the tables failed to load.
struct LoadedLoot {
    key: u64,
    defaults_generation: u64,
    data: Option<Arc<LootData>>,
}
static LOOT: Mutex<Option<LoadedLoot>> = Mutex::new(None);

/// A world's atomically replaceable loot snapshot, shared by gameplay loot readers.
#[derive(Clone, Default)]
pub struct LootSnapshot(Arc<std::sync::RwLock<Option<Arc<LootData>>>>);

impl LootSnapshot {
    /// Capture the snapshot once before doing work.
    pub fn get(&self) -> Option<Arc<LootData>> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Publish a server load or reload to all readers at once.
    pub fn set(&self, loot: Option<Arc<LootData>>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = loot;
    }
}

thread_local! {
    /// Loot data this thread keeps to itself, read ahead of [`LOOT`]. A game
    /// never sets it: a test that installs a world pack's tables does, so the
    /// tables reach its own rolls only and never a test running beside it
    /// (each test runs on its own thread; #14266).
    static THREAD_LOOT: RefCell<Option<Arc<LootData>>> = const { RefCell::new(None) };
}

fn thread_loot() -> Option<Arc<LootData>> {
    THREAD_LOOT.with(|own| own.borrow().clone())
}

/// Load the loot data of `root` with `overlay` for this thread alone:
/// [`loot_data`] and [`loaded_loot_data`] return it on this thread, while
/// other threads keep the process's copy. For tests whose world has packs.
pub fn load_thread_loot_data(
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &ItemDefaults,
) -> Option<Arc<LootData>> {
    let data = load_loot(root, overlay, defaults).data;
    THREAD_LOOT.with(|own| *own.borrow_mut() = data.clone());
    data
}

/// The resource root and the packs' files the loot data was loaded from.
fn loot_key(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    overlay.files.hash(&mut hasher);
    overlay.conditions.hash(&mut hasher);
    hasher.finish()
}

fn load_loot(
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &ItemDefaults,
) -> LoadedLoot {
    let data = match LootData::load(root, overlay, defaults) {
        Ok(data) => Some(Arc::new(data)),
        Err(e) => {
            log::warn!("Horizon: loot tables unavailable, structure containers stay empty: {e}");
            None
        }
    };
    LoadedLoot {
        key: loot_key(root, overlay),
        defaults_generation: defaults.generation(),
        data,
    }
}

/// Loot data of the resource root with the world's packs over it, loaded
/// once per process (a world open with other packs, or `/reload`, loads it
/// again).
pub fn loot_data(
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &ItemDefaults,
) -> Option<Arc<LootData>> {
    if let Some(own) = thread_loot() {
        return Some(own);
    }
    let mut loot = LOOT.lock().unwrap_or_else(|e| e.into_inner());
    if loot
        .as_ref()
        .is_none_or(|loaded| loaded.defaults_generation != defaults.generation())
    {
        *loot = Some(load_loot(root, overlay, defaults));
    }
    loot.as_ref().and_then(|loaded| loaded.data.clone())
}

/// [`loot_data`] for a world being opened: the copy already loaded when it
/// came from the same root and packs (world generation loads it first, with
/// no packs; a second copy held 8 MB for the whole run), else a fresh one.
pub fn world_loot_data(
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &ItemDefaults,
) -> Option<Arc<LootData>> {
    reuse_or_load(
        &mut LOOT.lock().unwrap_or_else(|e| e.into_inner()),
        root,
        overlay,
        defaults,
    )
}

fn reuse_or_load(
    loot: &mut Option<LoadedLoot>,
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &ItemDefaults,
) -> Option<Arc<LootData>> {
    let key = loot_key(root, overlay);
    if loot.as_ref().is_none_or(|loaded| {
        loaded.key != key || loaded.defaults_generation != defaults.generation()
    }) {
        *loot = Some(load_loot(root, overlay, defaults));
    }
    loot.as_ref().and_then(|loaded| loaded.data.clone())
}

/// The loot data `loot_data` loaded, if it ran.
pub fn loaded_loot_data(defaults: &ItemDefaults) -> Option<Arc<LootData>> {
    if let Some(own) = thread_loot() {
        return Some(own);
    }
    LOOT.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|loaded| loaded.defaults_generation == defaults.generation())
        .and_then(|loaded| loaded.data.clone())
}

/// Let go of the loot data: for a preload no world used
/// (game/world/world_loading.rs). The next `loot_data` loads it again.
pub fn forget_loot_data() {
    *LOOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn text(nbt: &Compound, key: &str) -> Option<String> {
    match nbt.get(&JavaString::from(key)) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The client block entity for one of a chunk's generated block entities,
/// before a container's loot is unpacked: `StructureTemplate.placeInWorld`
/// hands the template's block entity NBT to `loadWithComponents`, which is
/// what a chunk load does with a saved one.
fn generated_block_entity(block: &str, nbt: &Compound) -> Option<BlockEntity> {
    let declared = crate::block_entity_types::type_for(block);
    // A mod sign has no vanilla block name; its saved entity id is still vanilla's.
    let fallback = BlockEntity::for_block(block).or_else(|| {
        let id = text(nbt, "id")?;
        if !block.starts_with("minecraft:")
            && matches!(id.as_str(), "minecraft:sign" | "minecraft:hanging_sign")
        {
            BlockEntity::for_block(&id)
        } else {
            None
        }
    });
    let id = declared.or_else(|| fallback.as_ref().map(BlockEntity::id))?;
    let mut tag = nbt.clone();
    // RandomizableContainer.tryLoadLootTable: with a table the items are not read.
    if declared.is_none() && text(&tag, "LootTable").is_some() {
        tag.remove(&JavaString::from("Items"));
    }
    // The declaration owns identity; structures supply its payload, not a factory call.
    if declared.is_some() || !tag.contains_key(&JavaString::from("id")) {
        tag.insert("id".into(), Tag::String(id.into()));
    }
    for axis in ["x", "y", "z"] {
        tag.insert(axis.into(), Tag::Int(0));
    }
    // A saved kind the client cannot decode keeps the block's own entity.
    crate::save::decode_block_entity(&Tag::Compound(tag))
        .map(|(_, entity)| entity)
        .or(fallback)
}

/// Client block entities for the generated block entities of a chunk, and the
/// pending loot metadata of its containers.
pub fn block_entities(
    chunk: &ProtoChunk,
) -> (
    Vec<((i32, i32, i32), BlockEntity)>,
    std::collections::HashMap<(i32, i32, i32), crate::save::PendingContainerLoot>,
) {
    let mut out = Vec::new();
    let mut pending = std::collections::HashMap::new();
    for (pos, nbt) in &chunk.block_entities {
        let [x, y, z] = *pos;
        let Some(entity) = generated_block_entity(chunk.block_name(x, y, z), nbt) else {
            continue;
        };
        if matches!(entity, BlockEntity::Chest(_))
            && let Some(loot) =
                crate::save::PendingContainerLoot::from_tag(&Tag::Compound(nbt.clone()))
        {
            pending.insert((x, y, z), loot);
        }
        out.push(((x, y, z), entity));
    }
    (out, pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::loot::{ExplorationMap, LootStack};

    #[test]
    #[ignore = "daily"]
    fn declared_structure_entities_keep_template_payload_not_template_type() {
        const CHILD: &str = "HORIZON_TEST_DECLARED_STRUCTURE_ENTITY";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--include-ignored",
                        "--exact",
                        "structure_blocks::tests::declared_structure_entities_keep_template_payload_not_template_type",
                    ])
                    .env(CHILD, "1")
                    .output()
                    .unwrap();
            assert!(
                output.status.success(),
                "test=structure_blocks::tests::declared_structure_entities_keep_template_payload_not_template_type exit_code={:?} category=fixture-child-failed",
                output.status.code()
            );
            return;
        }
        use crate::block_entity_types::{self, BlockEntityTypes};
        block_entity_types::install(BlockEntityTypes::new(
            [
                ("test:ornate_banner".into(), "test:machine".into()),
                ("test:press".into(), "test:machine".into()),
            ]
            .into(),
            vec!["test:machine".into()],
        ))
        .unwrap();
        let Tag::Compound(nbt) = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:banner",energy:900L,LootTable:"test:loot",Items:[{Slot:0b,id:"minecraft:stone",count:3}]}"#,
        ).unwrap().0 else {
            panic!("not a compound");
        };
        for block in ["test:ornate_banner", "test:press"] {
            let Some(BlockEntity::Mod(entity)) = generated_block_entity(block, &nbt) else {
                panic!("declared type did not override {block}");
            };
            assert_eq!(entity.id(), "test:machine");
            assert_eq!(entity.nbt.get(&"energy".into()), Some(&Tag::Long(900)));
            assert_eq!(
                entity.nbt.get(&"LootTable".into()),
                Some(&Tag::String("test:loot".into()))
            );
            assert_eq!(entity.items[0].as_ref().unwrap().count, 3);
        }
    }

    #[test]
    fn a_saved_mod_sign_uses_its_vanilla_entity_type() {
        for hanging in [false, true] {
            let mut sign = crate::signs::SignData {
                hanging,
                ..Default::default()
            };
            sign.front.lines[0] = "Front".into();
            sign.back.lines[0] = "Back".into();
            sign.front.color = 9;
            sign.back.glowing = true;
            sign.waxed = true;
            let mut nbt: Compound = sign
                .encode()
                .into_iter()
                .map(|(k, v)| (k.into(), v))
                .collect();
            nbt.insert("id".into(), Tag::String(sign.id().into()));
            let Some(BlockEntity::Sign(loaded)) = generated_block_entity("test:notice", &nbt)
            else {
                panic!("saved sign missing");
            };
            assert_eq!(loaded, sign);
            assert!(generated_block_entity("minecraft:stone", &nbt).is_none());
        }
        assert!(generated_block_entity("test:road_sign", &Compound::new()).is_none());
    }

    #[test]
    fn generated_end_gateway_keeps_its_exit() {
        let Tag::Compound(nbt) = Tag::compound([
            ("exit_portal", Tag::IntArray(vec![100, 50, 0])),
            ("ExactTeleport", Tag::Byte(1)),
        ]) else {
            unreachable!()
        };
        let Some(BlockEntity::EndGateway(g)) =
            generated_block_entity("minecraft:end_gateway", &nbt)
        else {
            panic!("gateway");
        };
        assert_eq!(g.exit_portal, Some([100, 50, 0]));
        assert!(g.exact_teleport);
    }

    #[test]
    fn placed_chest_loot_keeps_its_enchantments_and_potions() {
        let stack =
            |item: &str, enchantments: Vec<(String, i32)>, potion: Option<&str>| LootStack {
                item: item.into(),
                count: 1,
                damage: 0,
                enchantments,
                potion: potion.map(str::to_owned),
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: Vec::new(),
                names: Vec::new(),
                components: Vec::new(),
            };
        // An End city chest's enchant_with_levels roll.
        let boots = loot_item_with_components(&stack(
            "minecraft:diamond_boots",
            vec![
                ("minecraft:protection".into(), 3),
                ("minecraft:unbreaking".into(), 2),
            ],
            None,
        ));
        assert_eq!(boots.enchantments().len(), 2);
        // A book's go into stored_enchantments.
        let book = loot_item_with_components(&stack(
            "minecraft:enchanted_book",
            vec![("minecraft:mending".into(), 1)],
            None,
        ));
        assert!(book.enchantments().is_empty());
        assert_eq!(book.stored_enchantments().len(), 1);
        let arrow = loot_item_with_components(&stack(
            "minecraft:tipped_arrow",
            Vec::new(),
            Some("minecraft:slowness"),
        ));
        assert_eq!(arrow.potion(), Some("minecraft:slowness"));
    }

    /// Opening a world keeps the loot data world generation loaded when the
    /// packs are the same, and loads it again when they differ.
    #[test]
    fn a_world_open_reuses_loot_data_of_the_same_packs() {
        use horizon_world::loot::PackOverlay;
        let root = std::env::temp_dir().join(format!("horizon-loot-reuse-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let mut slot = None;
        let first =
            reuse_or_load(&mut slot, &root, &PackOverlay::NONE, &defaults).expect("loot data");
        let again = reuse_or_load(&mut slot, &root, &PackOverlay::NONE, &defaults).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let files = [(
            "loot_table/chests/x".to_owned(),
            vec![br#"{"pools": []}"#.to_vec()],
        )];
        let packs = PackOverlay::new(&files);
        let other = reuse_or_load(&mut slot, &root, &packs, &defaults).unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        let next_defaults = horizon_world::item_defaults::read(&root).unwrap();
        let next = reuse_or_load(&mut slot, &root, &packs, &next_defaults).unwrap();
        assert!(!Arc::ptr_eq(&other, &next));

        let snapshot = LootSnapshot::default();
        let worker = snapshot.clone();
        snapshot.set(Some(first.clone()));
        let in_flight = worker.get().unwrap();
        snapshot.set(Some(other.clone()));
        assert!(Arc::ptr_eq(&in_flight, &first));
        assert!(Arc::ptr_eq(&worker.get().unwrap(), &other));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exploration_maps_wait_in_custom_data_until_searched() {
        let mut stack = LootStack {
            item: "minecraft:buried_treasure_map".into(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: true,
            exploration: Some(Box::new(ExplorationMap::from_json(&serde_json::json!({
                "decoration": "minecraft:red_x",
                "destination": "#minecraft:on_treasure_maps",
                "skip_existing_chunks": false,
                "zoom": 1
            })))),
            ominous_amplifier: None,
            stew_effects: Vec::new(),
            names: Vec::new(),
            components: Vec::new(),
        };
        let mut item = loot_item(&stack);
        assert!(has_pending_map(&item));
        // The rest of custom_data stays.
        if let Some(Tag::Compound(c)) = item
            .data_mut()
            .other
            .iter_mut()
            .find(|(k, _)| k == CUSTOM_DATA)
            .map(|(_, t)| t)
        {
            c.insert("note".into(), Tag::Int(7));
        }
        let map = take_pending_map(&mut item).unwrap();
        assert_eq!(
            map,
            ExplorationMap {
                destination: "minecraft:on_treasure_maps".into(),
                decoration: "minecraft:red_x".into(),
                zoom: 1,
                search_radius: 50,
                skip_existing_chunks: false,
            }
        );
        assert!(!has_pending_map(&item));
        assert!(take_pending_map(&mut item).is_none());
        let custom = &item.data().unwrap().other;
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].1.get("note"), Some(&Tag::Int(7)));
        // Only the search: the component goes away with it.
        stack.item = "minecraft:abandoned_camp_map".into();
        let mut item = loot_item(&stack);
        take_pending_map(&mut item).unwrap();
        assert!(item.data().unwrap().other.is_empty());
        // A plain item has nothing pending.
        stack.exploration = None;
        assert!(!has_pending_map(&loot_item(&stack)));
    }

    /// The End cities' tower tops save their banners' patterns in the
    /// template; the placed banner must keep them (`EndCityPieces`).
    #[test]
    fn a_generated_banner_keeps_its_template_patterns() {
        let patterns = Tag::List(vec![
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_top".into())),
            ]),
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_bottom".into())),
            ]),
        ]);
        let nbt = Tag::compound([
            ("id", Tag::String("minecraft:banner".into())),
            ("patterns", patterns),
        ]);
        let Tag::Compound(nbt) = nbt else {
            unreachable!()
        };
        let Some(BlockEntity::Banner(banner)) =
            generated_block_entity("minecraft:magenta_banner", &nbt)
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert_eq!(banner.layers.len(), 2);
        assert_eq!(
            banner.layers[0].pattern.id(),
            Some("minecraft:triangle_top")
        );
        assert_eq!(
            banner.layers[1].pattern.id(),
            Some("minecraft:triangle_bottom")
        );
        assert_eq!(&*banner.layers[0].color, "black");
        // A banner the template saved without patterns stays bare.
        let Some(BlockEntity::Banner(bare)) =
            generated_block_entity("minecraft:white_banner", &Compound::new())
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert!(bare.layers.is_empty());
    }
}
