//! Bulk item storage: [`STORAGE`], a block's items by kind, in 64-bit amounts and without slots,
//! for the stores, pipes and networks of a large base. Every block entity with slots answers it
//! through the game's own adapter over [`ITEMS`], so a pipe reaches a chest and a mod's drive
//! through one capability; a mod's bulk store (drawers, a drive, a vault) answers it itself, one
//! entry per kind however many items it holds.
//!
//! **Diverges from upstream: Horizon addition.** NeoForge 26.3's transfer API has no by-kind
//! capability; `ITEMS` (`Capabilities.Item.BLOCK`) is its counterpart and stays as it is.
//!
//! What a call costs is the handler's. A bulk store keyed by kind answers a question about one
//! kind, and a move of one kind, in constant time whatever it holds. The game's own adapter
//! answers each question in one pass over the slots the side reaches (O(slots), with no work per
//! item moved); a mod's `ITEMS` handler wrapped by kind is asked slot by slot. [`transfer`] is
//! four handler calls whatever it moves. A listing comes a [`StoragePage`] at a time, never
//! longer than [`MAX_STORAGE_PAGE`] entries, whatever a caller asks.
//!
//! **When not to use it.** A pipe that moves a few items a tick between small containers is
//! cheaper on `ITEMS` and [`move_resources`](crate::move_resources), slot by slot: in the
//! measurements behind this API, 256 pipes moving 8 items a tick each cost 2.8 ms a tick that
//! way against 3.7 ms through `STORAGE`. `STORAGE` pays off for bulk stores, large moves, and
//! networks that sleep while idle.
use crate::content::resource::{Ends, move_once};
use crate::{
    BlockSide, Capability, CapabilityKey, ITEMS, ItemResource, ResourceHandler, ServerHost,
    Transaction,
};
use std::marker::PhantomData;

/// The most entries one [`StoragePage`] holds: the host's cap, whatever limit a caller asks.
pub const MAX_STORAGE_PAGE: usize = 256;

/// Why a [`STORAGE`] question or move was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StorageError {
    /// A page of no entries was asked for.
    ZeroPage,
    /// A total, an amount or a cursor does not fit in 64 bits.
    Overflow,
    /// A handler answered more than it was asked: a mod bug, warned once naming the mod. The
    /// move it was part of is undone.
    Overanswered {
        /// The mod whose handler answered, or "the game".
        owner: String,
    },
    /// The handler could not answer: a wasm mod's that is stopped, busy or out of scope.
    Unavailable {
        /// The mod whose handler was asked.
        owner: String,
    },
}

impl From<(crate::Identifier, String)> for StorageError {
    fn from((owner, _): (crate::Identifier, String)) -> Self {
        Self::Unavailable {
            owner: owner.to_string(),
        }
    }
}

impl From<crate::Overanswered> for StorageError {
    fn from(error: crate::Overanswered) -> Self {
        Self::Overanswered { owner: error.owner }
    }
}

/// One page of a storage's listing: at most its limit of (kind, amount) entries. The limit is the
/// host's to cap ([`MAX_STORAGE_PAGE`]), and [`push`](Self::push) refuses an entry past it, so a
/// handler cannot hand back more than was asked. Reuse one page across calls: [`page`] clears it.
///
/// [`page`]: ItemStorage::page
// no wire: a manual codec (`wire/wire_types.rs`) re-checks the cap on decode, so a guest handler
// cannot hand back more than the host's limit
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct StoragePage {
    entries: Vec<(ItemResource, u64)>,
    limit: usize,
}

impl StoragePage {
    /// An empty page of at most `limit` entries, `limit` capped at [`MAX_STORAGE_PAGE`]; a limit
    /// of 0 is refused.
    pub fn new(limit: usize) -> Result<Self, StorageError> {
        if limit == 0 {
            return Err(StorageError::ZeroPage);
        }
        let limit = limit.min(MAX_STORAGE_PAGE);
        Ok(Self {
            entries: Vec::with_capacity(limit),
            limit,
        })
    }
    /// The most entries it holds.
    pub fn limit(&self) -> usize {
        self.limit
    }
    /// How many more entries it takes.
    pub fn room(&self) -> usize {
        self.limit - self.entries.len()
    }
    /// Adds an entry: false, and nothing added, when the page is full.
    pub fn push(&mut self, resource: ItemResource, amount: u64) -> bool {
        if self.entries.len() == self.limit {
            return false;
        }
        self.entries.push((resource, amount));
        true
    }
    /// Its entries, in the order the handler gave them.
    pub fn entries(&self) -> &[(ItemResource, u64)] {
        &self.entries
    }
    /// Empties it, keeping its limit.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// Its entries and limit, for its wire codec.
    pub(crate) fn wire_parts(&self) -> (&[(ItemResource, u64)], usize) {
        (&self.entries, self.limit)
    }
    /// A page decoded from the wire: none where its limit passes the host's cap or its entries
    /// pass its limit.
    pub(crate) fn from_wire(entries: Vec<(ItemResource, u64)>, limit: u64) -> Option<Self> {
        let limit = usize::try_from(limit)
            .ok()
            .filter(|&l| (1..=MAX_STORAGE_PAGE).contains(&l))?;
        (entries.len() <= limit).then_some(Self { entries, limit })
    }
}

/// A block's items by kind: how much of one kind it holds and has room for, and its writes, by
/// kind and in transactions, as a [`ResourceHandler`] is written. Stateless and given the host,
/// as every capability handler is. Amounts are 64-bit: a million items is one call.
pub trait ItemStorage {
    /// How much of `resource` it holds.
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError>;
    /// How much of `resource` an insert now would take: what a router plans by, not a promise
    /// (an insert may still take less).
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError>;
    /// Puts up to `amount` of `resource` in, in transaction `tx`: how much went in, never more
    /// than `amount`.
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64;
    /// Takes up to `amount` of `resource` out, in transaction `tx`: how much came out, never more
    /// than `amount`.
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64;
    /// Clears `page` and fills it with what it holds from position `cursor` on, as far as the page
    /// has room: the position to ask for next, or none when the listing reached its end. The
    /// cursor is a plain value the caller keeps, 0 for the start; a listing changed between pages
    /// may pass a kind over or give it twice.
    fn page(
        &self,
        host: &mut dyn ServerHost,
        cursor: u64,
        page: &mut StoragePage,
    ) -> Result<Option<u64>, StorageError>;
}

const STORAGE_SOURCE: &CapabilityKey<dyn ResourceHandler<ItemResource>> = &ITEMS;

/// The default storage handler adapts this capability, including its mod providers.
pub(crate) fn default_source(name: &str) -> Option<&'static str> {
    (name == STORAGE.name).then_some(STORAGE_SOURCE.name)
}

/// Items by kind: a container's or composter port's [`ItemStorage`] as one side reaches it.
/// The game answers it for every container or port [`ITEMS`] reaches, from those slots; a mod's
/// bulk store answers it with a provider of its own (`block_entity_capability(&STORAGE, ..)`).
/// A block with slots and a store of its own registers its provider `before_game`, or the slots
/// answer first.
pub const STORAGE: CapabilityKey<dyn ItemStorage> = CapabilityKey {
    name: "horizon:storage",
    default: Some(|host, at| {
        let slots = at.level.capability(host, STORAGE_SOURCE, at.pos, at.side)?;
        // The game's own slots are asked in one pass each; a mod's ITEMS, slot by slot.
        Some(match slots.owner {
            None => Box::new(GameStorage(at)) as Box<dyn ItemStorage>,
            Some(_) => Box::new(Slots(slots)),
        })
    }),
    handler: PhantomData,
};

/// The game's own [`STORAGE`] for container or composter slots: each question and write one
/// pass over the slots `.side` reaches, in the game, by the rules hoppers follow.
struct GameStorage(BlockSide);

impl ItemStorage for GameStorage {
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        crate::backend_of(&*host)
            .map_or(Some(0), |b| b.container_kind_amount(self.0, resource))
            .ok_or(StorageError::Overflow)
    }
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        crate::backend_of(&*host)
            .map_or(Some(0), |b| b.container_kind_space(self.0, resource))
            .ok_or(StorageError::Overflow)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64 {
        crate::backend_mut_of(host)
            .map_or(0, |b| b.container_insert_kind(self.0, resource, amount, tx))
            .min(amount)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64 {
        crate::backend_mut_of(host)
            .map_or(0, |b| {
                b.container_extract_kind(self.0, resource, amount, tx)
            })
            .min(amount)
    }
    fn page(
        &self,
        host: &mut dyn ServerHost,
        cursor: u64,
        page: &mut StoragePage,
    ) -> Result<Option<u64>, StorageError> {
        page.clear();
        let from = usize::try_from(cursor).map_err(|_| StorageError::Overflow)?;
        let slots = crate::backend_of(&*host)
            .and_then(|b| b.container_listing(self.0, from, page.limit()))
            .unwrap_or_default();
        // The host's cap again: a listing never holds more slots than the page could.
        let read = slots.len().min(page.limit());
        for stack in slots.iter().take(read).flatten() {
            page.push(ItemResource::of(stack), stack.count() as u64);
        }
        if read < page.limit() {
            return Ok(None);
        }
        cursor
            .checked_add(read as u64)
            .map(Some)
            .ok_or(StorageError::Overflow)
    }
}

/// [`STORAGE`] over a mod's own [`ITEMS`] handler, asked slot by slot. It inserts as
/// `SimpleContainer.addItem` orders it: the slots that hold the kind first, then the empty ones.
struct Slots(Capability<dyn ResourceHandler<ItemResource>>);

impl Slots {
    /// What slot `index` holds when it holds `resource`.
    fn holds(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> bool {
        self.0.resource(host, index).as_ref() == Some(resource)
    }
    /// Up to `left` into the slots `pick` chooses, as one slot takes at most `u32::MAX`.
    fn fill(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        left: &mut u64,
        pick: impl Fn(&mut dyn ServerHost, usize) -> bool,
    ) {
        for index in 0..self.0.size(host) {
            if *left == 0 {
                return;
            }
            if pick(host, index) {
                let ask = (*left).min(u32::MAX as u64) as u32;
                *left -= self.0.insert(host, tx, index, resource, ask).min(ask) as u64;
            }
        }
    }
}

impl ItemStorage for Slots {
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        let mut held = 0u64;
        for index in 0..self.0.size(host) {
            if self.holds(host, index, resource) {
                held = held
                    .checked_add(self.0.amount(host, index) as u64)
                    .ok_or(StorageError::Overflow)?;
            }
        }
        Ok(held)
    }
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        let mut room = 0u64;
        for index in 0..self.0.size(host) {
            let here = match self.0.resource(host, index) {
                Some(held) if held == *resource => self
                    .0
                    .capacity(host, index, resource)
                    .saturating_sub(self.0.amount(host, index)),
                Some(_) => 0,
                None if self.0.is_valid(host, index, resource) => {
                    self.0.capacity(host, index, resource)
                }
                None => 0,
            };
            room = room
                .checked_add(here as u64)
                .ok_or(StorageError::Overflow)?;
        }
        Ok(room)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64 {
        let mut left = amount;
        self.fill(host, tx, resource, &mut left, |host, index| {
            self.holds(host, index, resource)
        });
        self.fill(host, tx, resource, &mut left, |host, index| {
            self.0.resource(host, index).is_none()
        });
        amount - left
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u64,
    ) -> u64 {
        let mut left = amount;
        for index in 0..self.0.size(host) {
            if left == 0 {
                break;
            }
            if self.holds(host, index, resource) {
                let ask = left.min(u32::MAX as u64) as u32;
                left -= self.0.extract(host, tx, index, resource, ask).min(ask) as u64;
            }
        }
        amount - left
    }
    fn page(
        &self,
        host: &mut dyn ServerHost,
        cursor: u64,
        page: &mut StoragePage,
    ) -> Result<Option<u64>, StorageError> {
        page.clear();
        let size = self.0.size(host);
        let mut index = usize::try_from(cursor).map_err(|_| StorageError::Overflow)?;
        while index < size && page.room() > 0 {
            if let Some(resource) = self.0.resource(host, index) {
                let amount = self.0.amount(host, index) as u64;
                page.push(resource, amount);
            }
            index += 1;
        }
        Ok((index < size).then_some(index as u64))
    }
}

/// Moves up to `max` of `resource` out of `from` into `to`, in transaction `tx`: how much moved.
/// As one step of [`move_resources`](crate::move_resources): all or nothing in a transaction of
/// its own, a move `to` takes only part of tried again with that part, and a handler answering
/// more than it was asked ending it with [`StorageError::Overanswered`]. Four handler calls
/// whether one item moves or a million, and only the two questions when `from` has none or `to`
/// has no room.
pub fn transfer(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ItemStorage>,
    to: &Capability<dyn ItemStorage>,
    resource: &ItemResource,
    max: u64,
    tx: &Transaction,
) -> Result<u64, StorageError> {
    let _ = tx;
    let want = max
        .min(from.amount(host, resource)?)
        .min(to.space_for(host, resource)?);
    if want == 0 {
        return Ok(0);
    }
    Ok(move_once(
        host,
        Ends {
            from,
            to,
            at: resource,
        },
        want,
        |host, tx, want| from.extract(host, tx, resource, want),
        |host, tx, out| to.insert(host, tx, resource, out),
    )?)
}

/// Moves up to `max` of what `filter` accepts out of `from` into `to` (NeoForge's
/// `ResourceHandlerUtil.move`, by kind), in transaction `tx`: how much moved. `from` is listed a
/// full page ([`MAX_STORAGE_PAGE`]) at a time, into the one page this call allocates, and each
/// kind moved with [`transfer`]; a pass that moved something is followed by another, as a kind
/// emptied mid-listing can shift another past the cursor.
pub fn transfer_matching(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ItemStorage>,
    to: &Capability<dyn ItemStorage>,
    filter: impl Fn(&ItemResource) -> bool,
    max: u64,
    tx: &Transaction,
) -> Result<u64, StorageError> {
    let mut moved = 0;
    let mut page = StoragePage::new(MAX_STORAGE_PAGE)?;
    loop {
        let (mut cursor, mut moved_in_pass) = (Some(0), 0);
        while let Some(at) = cursor {
            cursor = from.page(host, at, &mut page)?;
            for (resource, _) in page
                .entries()
                .iter()
                .filter(|(resource, _)| filter(resource))
            {
                if moved == max {
                    return Ok(moved);
                }
                let went = transfer(host, from, to, resource, max - moved, tx)?;
                moved += went;
                moved_in_pass += went;
            }
        }
        if moved_in_pass == 0 || moved == max {
            return Ok(moved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::ItemsHost;
    use crate::testing::FakeHost;
    use crate::{HostTransaction, Identifier, ItemStack, Tag};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::hash::BuildHasher;

    fn host_with(items: &[&str]) -> (FakeHost, Vec<ItemResource>) {
        let mut host = FakeHost::new();
        let ids: Vec<_> = items
            .iter()
            .map(|id| Identifier::parse(id).unwrap())
            .collect();
        host.registries.resolve_items(ids.clone()).unwrap();
        for id in items {
            host.registries
                .item_component(id, "minecraft:max_stack_size", Tag::Int(64));
        }
        let resources = ids
            .iter()
            .map(|id| ItemResource::of(&ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
            .collect();
        (host, resources)
    }

    fn chest(host: &mut FakeHost, pos: [i32; 3], slots: &[Option<(&ItemResource, u32)>]) {
        let entity = host.level.block_entity(pos).with_items(slots.len());
        for (slot, held) in slots.iter().enumerate() {
            entity.items.as_mut().unwrap().slots[slot] = held.map(|(r, n)| r.to_stack(n).unwrap());
        }
    }

    fn storage_at(host: &mut FakeHost, pos: [i32; 3]) -> Capability<dyn ItemStorage> {
        let level = host.level.level();
        let server: &mut dyn ServerHost = host;
        level.capability(server, &STORAGE, pos, None).unwrap()
    }

    /// A bulk store in memory, counting the calls made to it: no transactions, for the cases
    /// that never abort.
    #[derive(Default)]
    struct Bulk {
        held: RefCell<HashMap<ItemResource, u64>>,
        room: u64,
        calls: Cell<u32>,
        /// What `page` tries to push, whatever the page's limit.
        greedy: usize,
    }
    struct Shared(std::rc::Rc<Bulk>);
    impl ItemStorage for Shared {
        fn amount(&self, _: &mut dyn ServerHost, r: &ItemResource) -> Result<u64, StorageError> {
            self.0.calls.set(self.0.calls.get() + 1);
            Ok(self.0.held.borrow().get(r).copied().unwrap_or(0))
        }
        fn space_for(&self, _: &mut dyn ServerHost, _: &ItemResource) -> Result<u64, StorageError> {
            self.0.calls.set(self.0.calls.get() + 1);
            Ok(self.0.room - self.0.held.borrow().values().sum::<u64>())
        }
        fn insert(&self, h: &mut dyn ServerHost, _: &Transaction, r: &ItemResource, n: u64) -> u64 {
            let went = n.min(self.space_for(h, r).unwrap());
            *self.0.held.borrow_mut().entry(r.clone()).or_default() += went;
            went
        }
        fn extract(
            &self,
            h: &mut dyn ServerHost,
            _: &Transaction,
            r: &ItemResource,
            n: u64,
        ) -> u64 {
            let came = n.min(self.amount(h, r).unwrap());
            *self.0.held.borrow_mut().entry(r.clone()).or_default() -= came;
            came
        }
        fn page(
            &self,
            _: &mut dyn ServerHost,
            _: u64,
            page: &mut StoragePage,
        ) -> Result<Option<u64>, StorageError> {
            page.clear();
            let entries: Vec<_> = self
                .0
                .held
                .borrow()
                .iter()
                .map(|(r, n)| (r.clone(), *n))
                .collect();
            for (resource, amount) in entries.into_iter().cycle().take(self.0.greedy.max(1)) {
                page.push(resource, amount);
            }
            Ok(None)
        }
    }
    fn bulk(
        room: u64,
        held: &[(&ItemResource, u64)],
    ) -> (std::rc::Rc<Bulk>, Capability<dyn ItemStorage>) {
        let store = std::rc::Rc::new(Bulk {
            held: RefCell::new(held.iter().map(|(r, n)| ((*r).clone(), *n)).collect()),
            room,
            ..Default::default()
        });
        let handler = Capability {
            handler: Box::new(Shared(store.clone())) as Box<dyn ItemStorage>,
            owner: None,
        };
        (store, handler)
    }

    #[test]
    fn the_game_answers_storage_for_slots_by_kind() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let (stone, dirt) = (&r[0], &r[1]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((stone, 40)), Some((dirt, 5)), Some((stone, 10)), None],
        );
        let storage = storage_at(&mut host, [0, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        assert_eq!(storage.amount(host, stone), Ok(50));
        assert_eq!(storage.amount(host, dirt), Ok(5));
        // 24 and 54 topping up the stone, 64 in the empty slot.
        assert_eq!(storage.space_for(host, stone), Ok(24 + 54 + 64));
        let went = host.transaction(|host, tx| Ok::<_, ()>(storage.insert(host, tx, stone, 500)));
        assert_eq!(went, Ok(142), "no more than the room");
        assert_eq!(storage.amount(host, stone), Ok(192));
        assert_eq!(
            storage.amount(host, dirt),
            Ok(5),
            "the dirt's slot is not touched"
        );
        let came = host.transaction(|host, tx| Ok::<_, ()>(storage.extract(host, tx, stone, 100)));
        assert_eq!(came, Ok(100));
        let mut page = StoragePage::new(10).unwrap();
        assert_eq!(storage.page(host, 0, &mut page), Ok(None));
        assert_eq!(page.entries().iter().map(|(_, n)| n).sum::<u64>(), 97);
    }

    #[test]
    fn a_slot_write_aborted_with_its_transaction_is_put_back() {
        let (mut host, r) = host_with(&["minecraft:stone"]);
        chest(&mut host, [0, 64, 0], &[Some((&r[0], 10)), None]);
        let storage = storage_at(&mut host, [0, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        let aborted: Result<(), ()> = host.transaction(|host, tx| {
            assert_eq!(storage.insert(host, tx, &r[0], 70), 70);
            Err(())
        });
        assert!(aborted.is_err());
        assert_eq!(storage.amount(host, &r[0]), Ok(10));
    }

    #[test]
    fn a_page_is_the_hosts_size_whatever_a_caller_asks() {
        assert_eq!(StoragePage::new(0).err(), Some(StorageError::ZeroPage));
        assert_eq!(StoragePage::new(1).unwrap().limit(), 1);
        assert_eq!(
            StoragePage::new(usize::MAX).unwrap().limit(),
            MAX_STORAGE_PAGE
        );
        // A container larger than a page lists across pages, the cursor a plain value.
        let (mut host, r) = host_with(&["minecraft:stone"]);
        let slots: Vec<_> = (0..300).map(|_| Some((&r[0], 1))).collect();
        chest(&mut host, [0, 64, 0], &slots);
        let storage = storage_at(&mut host, [0, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        let mut page = StoragePage::new(100_000).unwrap();
        assert_eq!(
            storage.page(host, 0, &mut page),
            Ok(Some(MAX_STORAGE_PAGE as u64))
        );
        assert_eq!(page.entries().len(), MAX_STORAGE_PAGE);
        let next = storage.page(host, MAX_STORAGE_PAGE as u64, &mut page);
        assert_eq!(next, Ok(None));
        assert_eq!(
            page.entries().len(),
            300 - MAX_STORAGE_PAGE,
            "the page was cleared first"
        );
        assert_eq!(
            storage.page(host, u64::MAX, &mut page),
            Ok(None),
            "past the end: empty"
        );
        assert!(page.entries().is_empty());
    }

    #[test]
    fn a_handler_cannot_hand_back_more_than_its_page() {
        let (mut host, r) = host_with(&["minecraft:stone"]);
        let (_, storage) = bulk(u64::MAX, &[(&r[0], 5)]);
        let greedy = std::rc::Rc::new(Bulk {
            held: RefCell::new([(r[0].clone(), 5)].into()),
            greedy: 10_000,
            ..Default::default()
        });
        let greedy = Capability {
            handler: Box::new(Shared(greedy)) as Box<dyn ItemStorage>,
            owner: None,
        };
        let host: &mut dyn ServerHost = &mut host;
        let mut page = StoragePage::new(8).unwrap();
        assert_eq!(greedy.page(host, 0, &mut page), Ok(None));
        assert_eq!(page.entries().len(), 8, "pushes past the limit are refused");
        assert!(!page.push(r[0].clone(), 1));
        assert_eq!(storage.page(host, 0, &mut page), Ok(None));
        assert_eq!(page.entries().len(), 1);
    }

    #[test]
    fn a_page_off_the_wire_is_held_to_the_cap() {
        let (_, r) = host_with(&["minecraft:stone"]);
        let entries = vec![(r[0].clone(), 1); 3];
        assert!(StoragePage::from_wire(entries.clone(), 3).is_some());
        assert!(
            StoragePage::from_wire(entries.clone(), 2).is_none(),
            "more entries than its limit"
        );
        assert!(
            StoragePage::from_wire(entries, MAX_STORAGE_PAGE as u64 + 1).is_none(),
            "past the cap"
        );
        assert!(
            StoragePage::from_wire(Vec::new(), 0).is_none(),
            "a zero limit"
        );
        let mut page = StoragePage::new(4).unwrap();
        page.push(r[0].clone(), 7);
        let mut out = crate::wire::Writer::default();
        crate::wire::Wire::put(&page, &mut out);
        let back: StoragePage = crate::wire::Reader::decode(&out.into_bytes()).unwrap();
        assert_eq!((back.entries(), back.limit()), (page.entries(), 4));
    }

    #[test]
    fn a_page_whose_encoded_length_passes_the_cap_is_refused_before_its_entries() {
        let (_, r) = host_with(&["minecraft:stone"]);
        let entry = {
            let mut out = crate::wire::Writer::default();
            crate::wire::Wire::put(&(r[0].clone(), 1u64), &mut out);
            out.into_bytes()
        };
        let encoded = |len: u32, entries: usize| {
            let mut bytes = len.to_le_bytes().to_vec();
            for _ in 0..entries {
                bytes.extend_from_slice(&entry);
            }
            bytes.extend_from_slice(&(MAX_STORAGE_PAGE as u64).to_le_bytes());
            bytes
        };
        let decode = |bytes: &[u8]| {
            crate::wire::Reader::decode::<StoragePage>(bytes).map(|page| page.entries().len())
        };
        assert_eq!(
            decode(&encoded(MAX_STORAGE_PAGE as u32, MAX_STORAGE_PAGE)),
            Ok(MAX_STORAGE_PAGE)
        );
        // Every entry is there, so only the cap can refuse it.
        let past = MAX_STORAGE_PAGE + 1;
        assert_eq!(
            decode(&encoded(past as u32, past)),
            Err(crate::wire::WireError::Length)
        );
        assert_eq!(
            decode(&encoded(u32::MAX, 1)),
            Err(crate::wire::WireError::Length),
            "a hostile length"
        );
    }

    #[test]
    fn a_transfer_moves_billions_in_four_calls() {
        let (mut host, r) = host_with(&["minecraft:cobblestone"]);
        let (from_store, from) = bulk(u64::MAX, &[(&r[0], 5_000_000_000)]);
        let (into_store, into) = bulk(u64::MAX, &[]);
        let host: &mut dyn ServerHost = &mut host;
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], u64::MAX, tx));
        assert_eq!(moved, Ok(5_000_000_000));
        assert_eq!(into_store.held.borrow()[&r[0]], 5_000_000_000);
        // amount and space_for to plan, then the extract (with its amount) and the insert
        // (with its space_for).
        assert_eq!(from_store.calls.get() + into_store.calls.get(), 4);
    }

    #[test]
    fn a_transfer_into_slots_takes_what_fits_and_leaves_the_rest() {
        let (mut host, r) = host_with(&["minecraft:stone"]);
        chest(&mut host, [0, 64, 0], &[None, None, None]);
        let into = storage_at(&mut host, [0, 64, 0]);
        let (from_store, from) = bulk(u64::MAX, &[(&r[0], 1000)]);
        let host: &mut dyn ServerHost = &mut host;
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], 1000, tx));
        assert_eq!(moved, Ok(192));
        assert_eq!(from_store.held.borrow()[&r[0]], 808);
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], 1000, tx));
        assert_eq!(moved, Ok(0), "a full target is not tried");
    }

    #[test]
    fn transfer_matching_moves_only_what_the_filter_takes() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((&r[0], 30)), Some((&r[1], 20)), Some((&r[1], 7))],
        );
        let from = storage_at(&mut host, [0, 64, 0]);
        let (into_store, into) = bulk(u64::MAX, &[]);
        let host: &mut dyn ServerHost = &mut host;
        let dirt = r[1].clone();
        let moved = host.transaction(|host, tx| {
            transfer_matching(host, &from, &into, |res| *res == dirt, u64::MAX, tx)
        });
        assert_eq!(moved, Ok(27));
        assert_eq!(from.amount(host, &r[0]), Ok(30));
        assert_eq!(into_store.held.borrow().get(&r[0]), None);
    }

    #[test]
    fn transfers_aborted_and_committed_conserve_every_item() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((&r[0], 64)), Some((&r[1], 40)), Some((&r[0], 9))],
        );
        chest(&mut host, [1, 64, 0], &[None, Some((&r[1], 3)), None]);
        let a = storage_at(&mut host, [0, 64, 0]);
        let b = storage_at(&mut host, [1, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        let total = |host: &mut dyn ServerHost| {
            r.iter()
                .map(|kind| a.amount(host, kind).unwrap() + b.amount(host, kind).unwrap())
                .sum::<u64>()
        };
        let before = total(host);
        for round in 0..40u64 {
            let (from, to) = if round.is_multiple_of(2) {
                (&a, &b)
            } else {
                (&b, &a)
            };
            let kind = &r[(round % 3 % 2) as usize];
            let result: Result<u64, StorageError> = host.transaction(|host, tx| {
                let moved = transfer(host, from, to, kind, round + 1, tx)?;
                // Every third round aborts after it wrote.
                if round.is_multiple_of(3) {
                    return Err(StorageError::ZeroPage);
                }
                Ok(moved)
            });
            assert_eq!(result.is_err(), round.is_multiple_of(3));
            assert_eq!(total(host), before, "round {round}");
        }
    }

    #[test]
    fn a_resource_keys_a_map_by_item_damage_and_components() {
        let (host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let stone = host
            .item_key(&Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        let state = std::hash::RandomState::new();
        let hash_with = |resource: &ItemResource| state.hash_one(resource);
        // Equal stacks of different counts are one kind, and hash alike.
        let many = ItemResource::of(&ItemStack::new(stone, 64).unwrap());
        assert_eq!(many, r[0]);
        assert_eq!(hash_with(&many), hash_with(&r[0]));
        // Damage and a component patch each make another kind.
        let mut worn = ItemStack::new(stone, 9).unwrap();
        worn.set_damage(3);
        let mut named = ItemStack::new(stone, 1).unwrap();
        named
            .set_components(
                [(
                    "minecraft:custom_data".into(),
                    Tag::Compound([("bench".into(), Tag::Int(1))].into()),
                )]
                .into(),
            )
            .unwrap();
        let named = ItemResource::of(&named);
        assert_ne!(named, r[0]);
        let mut counts: HashMap<ItemResource, u64> = HashMap::new();
        for resource in [
            &r[0],
            &r[1],
            &ItemResource::of(&worn),
            &named,
            &many,
            &named,
        ] {
            *counts.entry(resource.clone()).or_default() += 1;
        }
        assert_eq!(counts.len(), 4);
        assert_eq!(counts[&r[0]], 2, "stone of any count is one key");
        assert_eq!(counts[&named], 2, "the patch keys its own entry");
    }
}
