//! The energy capability (arismoko/horizon#41 §7): [`ENERGY`], as NeoForge 26.3's
//! `Capabilities.Energy.BLOCK`, an [`EnergyHandler`]: an amount and a capacity, with no resource,
//! written only inside a transaction.
use crate::content::resource::{Ends, move_once};
use crate::{
    BlockSide, Capability, CapabilityKey, ItemCapabilityKey, Overanswered, ServerHost, Transaction,
};
use std::marker::PhantomData;

/// A block's energy (NeoForge's `EnergyHandler`): stateless, given the host, and written only in
/// a transaction, as a [`ResourceHandler`](crate::ResourceHandler) is.
pub trait EnergyHandler: Send + Sync {
    /// How much it holds.
    fn amount(&self, host: &mut dyn ServerHost) -> u32;
    /// How much it holds when full.
    fn capacity(&self, host: &mut dyn ServerHost) -> u32;
    /// Puts up to `amount` in, in transaction `tx`: how much went. Never more than `amount`.
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32;
    /// Takes up to `amount` out, in transaction `tx`: how much came. Never more than `amount`.
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32;
}

/// Energy, as NeoForge 26.3's `Capabilities.Energy.BLOCK`: a block entity's energy as one side
/// reaches it. The game answers it for a mod block entity's buffer (`Registrar::block_energy`, as
/// `SimpleEnergyHandler`): its capacity, the most one insert and one extract move, and the sides
/// a fill comes through and a drain leaves by.
pub const ENERGY: CapabilityKey<dyn EnergyHandler> = CapabilityKey {
    name: "horizon:energy",
    default: Some(|host, at| {
        crate::backend_of(host)
            .and_then(|backend| backend.energy_amount(at))
            .map(|_| Box::new(GameEnergy(at)) as Box<dyn EnergyHandler>)
    }),
    handler: PhantomData,
};

/// Energy held by an item, as NeoForge 26.3's `Capabilities.Energy.ITEM`: a handler for the item at
/// one place, found with [`ItemPlace::capability`](crate::ItemPlace::capability), whose changes
/// rewrite the item there with an [`exchange`](crate::ItemPlace::exchange) of one, so from a stack
/// of more than one the changed item splits off. Vanilla has no energy items, so only mods answer
/// it.
pub const ITEM_ENERGY: ItemCapabilityKey<dyn EnergyHandler> =
    ItemCapabilityKey::new("horizon:item_energy");

/// The game's own [`ENERGY`]: a mod block entity's buffer, as `.side` reaches it.
struct GameEnergy(BlockSide);
impl EnergyHandler for GameEnergy {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        crate::backend_of(host)
            .and_then(|backend| backend.energy_amount(self.0))
            .unwrap_or(0)
    }
    fn capacity(&self, host: &mut dyn ServerHost) -> u32 {
        crate::backend_of(host).map_or(0, |backend| backend.energy_capacity(self.0))
    }
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        crate::backend_mut_of(host)
            .map_or(0, |backend| backend.energy_insert(self.0, amount, tx))
            .min(amount)
    }
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        crate::backend_mut_of(host)
            .map_or(0, |backend| backend.energy_extract(self.0, amount, tx))
            .min(amount)
    }
}

/// Moves up to `max` energy out of `from` into `to` (NeoForge's `EnergyHandlerUtil.move`), in
/// transaction `tx`: how much moved. As one step of `move_resources`: all or nothing in a
/// transaction of its own, a move `to` takes only part of tried again with that part, and a
/// handler answering more than it was asked ending it with [`Overanswered`].
pub fn move_energy(
    host: &mut dyn ServerHost,
    from: &Capability<dyn EnergyHandler>,
    to: &Capability<dyn EnergyHandler>,
    max: u32,
    tx: &Transaction,
) -> Result<u32, Overanswered> {
    let _ = tx;
    let want = max.min(from.amount(host));
    if want == 0 {
        return Ok(0);
    }
    move_once(
        host,
        Ends {
            from,
            to,
            at: &"energy",
        },
        want,
        |host, tx, want| from.extract(host, tx, want),
        |host, tx, out| to.insert(host, tx, out),
    )
}

/// `EnergyHandlerUtil`'s queries, on any energy handler (a [`Capability`] reaches them through its
/// handler): `cap.is_full(host)`. To pass calls through to another handler
/// (`DelegatingEnergyHandler`), implement [`EnergyHandler`] on a struct holding it and forward.
impl dyn EnergyHandler {
    /// Whether it holds its capacity (`isFull`).
    pub fn is_full(&self, host: &mut dyn ServerHost) -> bool {
        self.amount(host) >= self.capacity(host)
    }
    /// A comparator's signal reading it (`getRedstoneSignalFromEnergyHandler`), vanilla's
    /// container formula: 0 when empty, else `1 + floor(14 * amount / capacity)`, 15 when full.
    pub fn comparator_signal(&self, host: &mut dyn ServerHost) -> u8 {
        let amount = u64::from(self.amount(host));
        let capacity = u64::from(self.capacity(host));
        if amount == 0 || capacity == 0 {
            return 0;
        }
        (1 + 14 * amount.min(capacity) / capacity) as u8
    }
}

/// No energy and no room, refusing both ways (`EmptyEnergyHandler`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: a unit marker
pub struct EmptyEnergyHandler;
impl EnergyHandler for EmptyEnergyHandler {
    fn amount(&self, _: &mut dyn ServerHost) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        0
    }
    fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
        0
    }
    fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
        0
    }
}

/// An endless source (`InfiniteEnergyHandler`): every extract is given whole, nothing goes in. A
/// creative battery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: a unit marker
pub struct InfiniteEnergyHandler;
impl EnergyHandler for InfiniteEnergyHandler {
    fn amount(&self, _: &mut dyn ServerHost) -> u32 {
        u32::MAX
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        u32::MAX
    }
    fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
        0
    }
    fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        amount
    }
}

/// A sink that destroys what goes in and holds nothing (`VoidingEnergyHandler`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: a unit marker
pub struct VoidingEnergyHandler;
impl EnergyHandler for VoidingEnergyHandler {
    fn amount(&self, _: &mut dyn ServerHost) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        u32::MAX
    }
    fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        amount
    }
    fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
        0
    }
}

/// Another handler with at most `max_insert` going in and `max_extract` coming out per call
/// (`LimitingEnergyHandler`): each call is clamped, then forwarded in the same transaction, as a
/// cable's throughput.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LimitingEnergyHandler {
    inner: Box<dyn EnergyHandler>,
    max_insert: u32,
    max_extract: u32,
}
impl LimitingEnergyHandler {
    /// `inner`, limited to `max_insert` in and `max_extract` out per call.
    pub fn new(inner: Box<dyn EnergyHandler>, max_insert: u32, max_extract: u32) -> Self {
        Self {
            inner,
            max_insert,
            max_extract,
        }
    }
}
impl EnergyHandler for LimitingEnergyHandler {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        self.inner.amount(host)
    }
    fn capacity(&self, host: &mut dyn ServerHost) -> u32 {
        self.inner.capacity(host)
    }
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        self.inner.insert(host, tx, amount.min(self.max_insert))
    }
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        self.inner.extract(host, tx, amount.min(self.max_extract))
    }
}

#[cfg(test)]
#[path = "energy_capability_tests.rs"]
mod tests;
