//! Engine-owned proposals; these never mutate world storage before tick acceptance.
use super::Game;
use horizon_client::{
    items::ItemStack,
    join_seq::residency::table::{EntityLookup, Location},
};

#[derive(Clone, Copy)]
pub(super) struct Key {
    pub pos: (i32, i32, i32),
    pub placeholder_identity: u64,
    pub failure_generation: u64,
    pub transaction_depth: usize,
    pub unbalanced: bool,
}

#[derive(Clone)]
pub(super) struct Proposal {
    pub id: u32,
    pub location: Location,
    pub original: ItemStack,
    pub replacement: Option<ItemStack>,
}

pub(super) struct Scope {
    pub generation: Option<u64>,
    pos: (i32, i32, i32),
    placeholder_identity: u64,
    pub(super) failure_generation: u64,
    pub poisoned: bool,
    pub items: Option<Vec<Proposal>>,
    pub unchanged: bool,
    pub snapshot: Option<
        Box<(
            mod_api::ModBlockEntity,
            mod_api::item::containers::ModRevision,
        )>,
    >,
    savepoints: Vec<Option<Vec<Proposal>>>,
}

impl Scope {
    pub fn new(pos: (i32, i32, i32), placeholder_identity: u64, failure_generation: u64) -> Self {
        Self {
            generation: None,
            pos,
            placeholder_identity,
            failure_generation,
            poisoned: false,
            items: None,
            unchanged: false,
            snapshot: None,
            savepoints: Vec::new(),
        }
    }

    pub fn propose(
        &mut self,
        id: u32,
        location: Location,
        original: &ItemStack,
        replacement: Option<ItemStack>,
    ) {
        let proposals = self.items.get_or_insert_with(Vec::new);
        if let Some(proposal) = proposals.iter_mut().find(|p| p.id == id) {
            if proposal.location != location {
                self.poisoned = true;
            } else {
                proposal.replacement = replacement;
            }
        } else {
            proposals.push(Proposal {
                id,
                location,
                original: original.clone(),
                replacement,
            });
        }
    }

    pub fn proposal(&self, id: u32) -> Option<&Proposal> {
        self.items.as_ref()?.iter().find(|p| p.id == id)
    }

    pub fn open_transaction(&mut self) {
        self.savepoints.push(self.items.clone());
    }

    pub fn close_transaction(&mut self, commit: bool) {
        if let Some(saved) = self.savepoints.pop() {
            if !commit {
                self.items = saved;
            }
        } else {
            self.poisoned = true;
        }
    }

    pub fn balanced(&self) -> bool {
        self.savepoints.is_empty()
    }
}

impl Game<'_> {
    /// None means ordinary immediate semantics; Some(false) is a scoped refusal.
    pub(super) fn stage_tick_item(
        &mut self,
        id: u32,
        replacement: Option<ItemStack>,
    ) -> Option<bool> {
        let key = self.block_entity_tick_key?;
        if self.block_entity_tick_items.is_none() {
            let mut scope = Scope::new(key.pos, key.placeholder_identity, key.failure_generation);
            scope.savepoints.resize_with(
                self.transactions
                    .len()
                    .saturating_sub(key.transaction_depth),
                || None,
            );
            self.block_entity_tick_items = Some(Box::new(scope));
        }
        let EntityLookup::Found { location, .. } = self.server_resident_entity(id) else {
            return Some(false);
        };
        let Some(original) = self
            .dropped
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.stack.clone())
        else {
            return Some(false);
        };
        let generation = self
            .block_entity_tick_items
            .as_ref()
            .and_then(|scope| self.world.block_entities.generation(&scope.pos));
        if let Some(scope) = self.block_entity_tick_items.as_mut() {
            if scope.generation.is_none() {
                scope.generation = generation;
            }
            scope.propose(id, location, &original, replacement);
        }
        Some(true)
    }

    pub(super) fn finish_tick_item_writeback(
        &mut self,
        pos: (i32, i32, i32),
        mut taken: horizon_client::containers::ModBlockEntity,
        original_items: &[Option<ItemStack>],
        parent: (Option<Key>, Option<Box<Scope>>),
        changed: bool,
    ) {
        use super::api_values::{BlockEntitySnapshotError, finish_block_entity_writeback};
        use horizon_client::containers::BlockEntity;
        if let Some(key) = self.block_entity_tick_key
            && key.unbalanced
        {
            if let Some(scope) = &mut self.block_entity_tick_items {
                scope.poisoned = true;
            }
            self.abort_tick_item_transactions(key.transaction_depth);
        }
        let Some(mut scope) = self.block_entity_tick_items.take() else {
            if changed {
                self.put_back_mod_block_entity(pos, taken, original_items);
            } else {
                self.restore_unchanged_tick(pos, taken);
            }
            self.block_entity_tick_key = parent.0;
            self.block_entity_tick_items = parent.1;
            return;
        };
        if scope.snapshot.is_none() && !scope.unchanged {
            if changed {
                self.put_back_mod_block_entity(pos, taken, original_items);
            } else {
                self.restore_unchanged_tick(pos, taken);
            }
            self.block_entity_tick_key = parent.0;
            self.block_entity_tick_items = parent.1;
            return;
        }
        let owns_placeholder = self.world.block_entities.generation(&pos) == scope.generation
            && matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Mod(entity)) if entity.id().is_empty() && entity.revision().identity() == scope.placeholder_identity);
        let valid = owns_placeholder && !scope.poisoned && scope.balanced()
            && scope.failure_generation == self.wasm_dispatch_failures.get()
            && scope.items.as_ref().is_none_or(|items| items.iter().all(|proposal| {
                matches!(self.server_resident_entity(proposal.id), EntityLookup::Found { location, .. } if location == proposal.location)
                    && self.dropped.iter().any(|e| e.id == proposal.id && e.stack == proposal.original)
            }));
        let accepted = if scope.unchanged {
            valid
        } else if let Some(snapshot) = scope.snapshot.take() {
            let (snapshot, before) = *snapshot;
            finish_block_entity_writeback(
                &mut taken,
                &snapshot,
                before,
                self.items(),
                "block_entity_tick",
                if valid {
                    Ok(())
                } else {
                    Err(BlockEntitySnapshotError::InvalidRevision)
                },
            )
            .is_ok()
        } else {
            false
        };
        let mut removed = Vec::new();
        if accepted && let Some(items) = scope.items.take() {
            for proposal in items {
                if let Some(index) = self.dropped.iter().position(|e| e.id == proposal.id) {
                    if let Some(stack) = proposal.replacement {
                        self.dropped[index].stack = stack;
                    } else {
                        removed.push(self.dropped.remove(index).id);
                    }
                }
            }
        }
        if owns_placeholder {
            if changed {
                self.put_back_mod_block_entity(pos, taken, original_items);
            } else {
                self.restore_unchanged_tick(pos, taken);
            }
        } else {
            self.drop_unreturned_mod_contents(pos, taken);
        }
        self.block_entity_tick_key = parent.0;
        self.block_entity_tick_items = parent.1;
        for id in removed {
            self.native_entity_left(id);
        }
    }

    #[inline(always)]
    pub(super) fn tick_item_view(&self, id: u32) -> Option<Option<&ItemStack>> {
        let scope = self.block_entity_tick_items.as_ref()?;
        self.pending_tick_item_view(scope, id)
    }

    #[cold]
    #[inline(never)]
    fn pending_tick_item_view<'a>(
        &self,
        scope: &'a Scope,
        id: u32,
    ) -> Option<Option<&'a ItemStack>> {
        let proposal = scope.proposal(id)?;
        let EntityLookup::Found { location, .. } = self.server_resident_entity(id) else {
            return Some(None);
        };
        Some(
            (location == proposal.location)
                .then_some(proposal.replacement.as_ref())
                .flatten(),
        )
    }
}
