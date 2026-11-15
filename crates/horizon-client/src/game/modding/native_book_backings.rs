//! Canonical physical storage behind a detached custom placement.
use super::{Game, native_book_scope::MenuSnapshot};
use horizon_client::{
    containers::BlockEntity, items::ItemStack, lan::menu::SlotBacking,
    menu::place::custom_plan::MenuPlacement, recipe_book::placement_sync::Refusal,
};
use horizon_core::nbt::Compound;
use std::collections::{BTreeMap, BTreeSet};
type Pos = (i32, i32, i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub dimension: &'static str,
    pub pos: Pos,
    pub slot: usize,
}
struct Block {
    state: u32,
    generation: Option<u64>,
    kind: std::mem::Discriminant<BlockEntity>,
    id: Option<String>,
    items: Vec<Option<ItemStack>>,
    limit: Option<u32>,
    data: Option<(u64, Compound, Vec<i32>)>,
}
pub(super) struct Backings {
    dimension: &'static str,
    blocks: BTreeMap<Pos, Block>,
    pub main: Vec<Option<ItemStack>>,
    main_keys: Vec<Option<Key>>,
    clear: BTreeSet<Key>,
    pub source_keys: Vec<Vec<Option<Key>>>,
}
impl Backings {
    pub fn capture(
        game: &Game<'_>,
        before: &MenuSnapshot,
        plan: &MenuPlacement,
    ) -> Result<Self, Refusal> {
        let guest = game
            .lan
            .as_ref()
            .and_then(|h| h.hub.guest(before.conn))
            .ok_or(Refusal::Changed)?;
        let main = before
            .slots
            .read(&game.world, guest)
            .filter(|items| items.len() == before.size)
            .ok_or(Refusal::Unavailable)?;
        let mut value = Self {
            dimension: game.dim.id(),
            blocks: BTreeMap::new(),
            main,
            main_keys: Vec::with_capacity(before.size),
            clear: BTreeSet::new(),
            source_keys: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        for slot in 0..before.size {
            let key = match before.slots.logical_backing(&game.world, guest, slot) {
                Some(SlotBacking::Block(pos, index)) => {
                    let key = Key {
                        dimension: value.dimension,
                        pos,
                        slot: index,
                    };
                    if !seen.insert(key) {
                        return Err(Refusal::Unavailable);
                    }
                    value.keep(game, pos)?;
                    Some(key)
                }
                None => None,
                _ => return Err(Refusal::Unavailable),
            };
            value.main_keys.push(key);
        }
        for &slot in plan.cleared_slots() {
            if before.hidden.contains(&slot) {
                return Err(Refusal::Unavailable);
            }
            let key = value
                .main_keys
                .get(slot)
                .copied()
                .flatten()
                .ok_or(Refusal::Unavailable)?;
            value.clear.insert(key);
        }
        Ok(value)
    }
    fn keep(&mut self, game: &Game<'_>, pos: Pos) -> Result<(), Refusal> {
        if self.blocks.contains_key(&pos) {
            return Ok(());
        }
        let entity = game
            .world
            .block_entities
            .get(&pos)
            .ok_or(Refusal::Changed)?;
        let (id, data) = match entity {
            BlockEntity::Mod(m) => (
                Some(m.id().to_owned()),
                Some((m.revision().identity(), m.nbt.clone(), m.data.clone())),
            ),
            _ => (None, None),
        };
        self.blocks.insert(
            pos,
            Block {
                state: game.world.block(pos.0, pos.1, pos.2).0,
                generation: game.world.block_entities.generation(&pos),
                kind: std::mem::discriminant(entity),
                id,
                items: entity.slots().to_vec(),
                limit: game.world.container_stack_limit(pos),
                data,
            },
        );
        Ok(())
    }
    /// The source hook grants containers, not duplicate copies of a physical
    /// slot. Cleared slots already return their contents through the player.
    pub fn add_sources(
        &mut self,
        game: &Game<'_>,
        sources: &[(Pos, Vec<Option<ItemStack>>)],
    ) -> Result<Vec<Vec<Option<ItemStack>>>, Refusal> {
        let mut seen = self.clear.clone();
        let mut stock = Vec::with_capacity(sources.len());
        for (pos, items) in sources {
            self.keep(game, *pos)?;
            let mut keys = Vec::with_capacity(items.len());
            let mut values = Vec::with_capacity(items.len());
            for (slot, item) in items.iter().enumerate() {
                let key = Key {
                    dimension: self.dimension,
                    pos: *pos,
                    slot,
                };
                let owned = seen.insert(key);
                keys.push(owned.then_some(key));
                values.push(if owned { item.clone() } else { None });
            }
            self.source_keys.push(keys);
            stock.push(values);
        }
        Ok(stock)
    }
    pub fn unchanged(&self, game: &Game<'_>) -> bool {
        game.dim.id() == self.dimension
            && self.blocks.iter().all(|(pos, before)| {
                let Some(entity) = game.world.block_entities.get(pos) else {
                    return false;
                };
                if game.world.block(pos.0, pos.1, pos.2).0 != before.state
                    || game.world.block_entities.generation(pos) != before.generation
                    || std::mem::discriminant(entity) != before.kind
                    || entity.slots() != before.items
                    || game.world.container_stack_limit(*pos) != before.limit
                {
                    return false;
                }
                match entity {
                    BlockEntity::Mod(m) => {
                        before.id.as_deref() == Some(m.id())
                            && before.data.as_ref().is_some_and(|(identity, nbt, data)| {
                                m.revision().identity() == *identity
                                    && &m.nbt == nbt
                                    && &m.data == data
                            })
                    }
                    _ => before.id.is_none() && before.data.is_none(),
                }
            })
    }
    pub fn limits(&self) -> Vec<Option<u32>> {
        self.main_keys
            .iter()
            .map(|key| key.and_then(|key| self.blocks.get(&key.pos).and_then(|block| block.limit)))
            .collect()
    }
    /// Only operation-owned cells are published. Rewriting untouched displayed
    /// slots would restore consumed items when a linked view aliases a source.
    pub fn changes(
        &self,
        menu: &mut horizon_client::menu::Menu,
    ) -> BTreeMap<Key, Option<ItemStack>> {
        let mut after = BTreeMap::new();
        for (slot, key) in self.main_keys.iter().enumerate() {
            if let Some(key) = key
                && self.clear.contains(key)
            {
                after.insert(*key, menu.slot(slot).cloned());
            }
        }
        for (keys, items) in self.source_keys.iter().zip(&menu.sources) {
            for (key, item) in keys.iter().zip(items) {
                if let Some(key) = key {
                    after.insert(*key, item.clone());
                }
            }
        }
        for (slot, key) in self.main_keys.iter().enumerate() {
            if let Some(item) = key.and_then(|key| after.get(&key)) {
                menu.load_slot(slot, item.clone());
            }
        }
        after.retain(|key, item| {
            self.blocks
                .get(&key.pos)
                .and_then(|block| block.items.get(key.slot))
                != Some(item)
        });
        after
    }
}
