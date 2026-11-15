//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
// no wire: encoded by the existing manual Wire implementation on TicketOwner.
/// What a ticket belongs to: a block (a machine) or an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: a closed set the game matches without a wildcard
pub enum TicketOwner {
    /// The block at `[x, y, z]`.
    Block([i32; 3]),
    /// The entity with this UUID.
    Entity(u128),
}
/// One chunk a mod holds loaded for an owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModTicket {
    /// The controller that holds it: its slot in the installed controller table
    /// (`mod_api::TicketController::slot`).
    pub controller: u32,
    pub owner: TicketOwner,
    pub chunk: (i32, i32),
    /// Whether the chunk joins the spawning chunks (inhabited time, thunder,
    /// a spawn attempt) with no player within 128 blocks
    /// (`forceChunk`'s `forceNaturalSpawning`). One owner holds the two kinds
    /// apart.
    pub natural_spawning: bool,
}
/// The chunks one owner holds, the two kinds apart (NeoForge's `TicketSet`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct TicketSet {
    /// Chunks held without natural spawning.
    pub non_natural_spawning: Vec<[i32; 2]>,
    /// Chunks held with natural spawning forced.
    pub natural_spawning: Vec<[i32; 2]>,
}
impl TicketSet {
    fn push(&mut self, chunk: [i32; 2], natural_spawning: bool) {
        if natural_spawning {
            self.natural_spawning.push(chunk);
        } else {
            self.non_natural_spawning.push(chunk);
        }
    }
}
/// The tickets one controller loaded, as `validate_tickets` hands them out
/// (NeoForge's `TicketHelper`, one for each controller). Every call acts as
/// the mod marked as running, and only the controller's owning mod sees or
/// removes anything, block owners and entity owners apart. The game releases
/// what was removed once the hook returns.
pub trait TicketValidation {
    /// The controller's block tickets: each block position with the chunks it
    /// holds without and with natural spawning (`TicketHelper.getBlockTickets`'
    /// `TicketSet`: `nonNaturalSpawning`, `naturalSpawning`), in position order.
    fn block_tickets(&self) -> Vec<([i32; 3], TicketSet)>;
    /// The controller's entity tickets: each entity UUID with the chunks it
    /// holds without and with natural spawning (`TicketHelper.getEntityTickets`),
    /// in UUID order.
    fn entity_tickets(&self) -> Vec<(u128, TicketSet)>;
    /// Remove every ticket the block holds, of both kinds
    /// (`removeAllTickets(BlockPos)`): how many went.
    fn remove_block_tickets(&mut self, pos: [i32; 3]) -> usize;
    /// Remove every ticket the entity holds, of both kinds
    /// (`removeAllTickets(UUID)`).
    fn remove_entity_tickets(&mut self, uuid: u128) -> usize;
    /// Remove the block's ticket of one kind on one chunk
    /// (`removeTicket(BlockPos, ChunkPos, forceNaturalSpawning)`): whether it
    /// held one. The other kind on the chunk stays.
    fn remove_block_ticket(
        &mut self,
        pos: [i32; 3],
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool;
    /// Remove the entity's ticket of one kind on one chunk
    /// (`removeTicket(UUID, ChunkPos, forceNaturalSpawning)`).
    fn remove_entity_ticket(&mut self, uuid: u128, chunk: [i32; 2], natural_spawning: bool)
    -> bool;
}

// no wire: controller-owned mutable ticket bookkeeping retains static mod ownership; individual ModTicket values cross instead.
/// The game's [`TicketValidation`]: one controller's loaded tickets, and those removed.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ControllerTickets {
    /// The `[mod] id` that owns the controller.
    owner: &'static str,
    loaded: Vec<ModTicket>,
    removed: Vec<ModTicket>,
}
/// Shows only how many tickets the calling mod's controller holds, never another mod's.
impl std::fmt::Debug for ControllerTickets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerTickets")
            .field("own", &self.own().count())
            .finish()
    }
}
impl ControllerTickets {
    /// A validation of the `loaded` tickets of one controller, owned by mod
    /// `owner`. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(owner: &'static str, loaded: Vec<ModTicket>) -> Self {
        Self {
            owner,
            loaded,
            removed: Vec::new(),
        }
    }
    /// The tickets removed. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn into_removed(self) -> Vec<ModTicket> {
        self.removed
    }
}
impl TicketValidation for ControllerTickets {
    fn block_tickets(&self) -> Vec<([i32; 3], TicketSet)> {
        let mut grouped = std::collections::BTreeMap::<[i32; 3], TicketSet>::new();
        for (owner, chunk, spawning) in self.own() {
            if let TicketOwner::Block(pos) = owner {
                grouped.entry(pos).or_default().push(chunk, spawning);
            }
        }
        grouped.into_iter().collect()
    }
    fn entity_tickets(&self) -> Vec<(u128, TicketSet)> {
        let mut grouped = std::collections::BTreeMap::<u128, TicketSet>::new();
        for (owner, chunk, spawning) in self.own() {
            if let TicketOwner::Entity(uuid) = owner {
                grouped.entry(uuid).or_default().push(chunk, spawning);
            }
        }
        grouped.into_iter().collect()
    }
    fn remove_block_tickets(&mut self, pos: [i32; 3]) -> usize {
        self.remove(TicketOwner::Block(pos), None)
    }
    fn remove_entity_tickets(&mut self, uuid: u128) -> usize {
        self.remove(TicketOwner::Entity(uuid), None)
    }
    fn remove_block_ticket(
        &mut self,
        pos: [i32; 3],
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool {
        self.remove(TicketOwner::Block(pos), Some((chunk, natural_spawning))) > 0
    }
    fn remove_entity_ticket(
        &mut self,
        uuid: u128,
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool {
        self.remove(TicketOwner::Entity(uuid), Some((chunk, natural_spawning))) > 0
    }
}
impl ControllerTickets {
    fn own(&self) -> impl Iterator<Item = (TicketOwner, [i32; 2], bool)> + '_ {
        let owned = crate::content::crash::current_native_mod() == Some(self.owner);
        self.loaded
            .iter()
            .filter(move |_| owned)
            .map(|t| (t.owner, [t.chunk.0, t.chunk.1], t.natural_spawning))
    }
    fn remove(&mut self, owner: TicketOwner, kind: Option<([i32; 2], bool)>) -> usize {
        if crate::content::crash::current_native_mod() != Some(self.owner) {
            return 0;
        }
        let (gone, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.loaded).into_iter().partition(|t| {
                t.owner == owner
                    && kind.is_none_or(|([x, z], spawning)| {
                        t.chunk == (x, z) && t.natural_spawning == spawning
                    })
            });
        self.loaded = kept;
        let count = gone.len();
        self.removed.extend(gone);
        count
    }
}
