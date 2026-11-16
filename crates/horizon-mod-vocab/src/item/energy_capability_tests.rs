use super::*;
use crate::{HostTransaction, testing::FakeHost};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

/// A mod's own buffer of a capacity, shared with the test to read back.
#[derive(Clone)]
struct Buffer(Arc<AtomicU32>, u32);
impl Buffer {
    fn new(held: u32, capacity: u32) -> Self {
        Self(Arc::new(AtomicU32::new(held)), capacity)
    }
    fn held(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}
impl EnergyHandler for Buffer {
    fn amount(&self, _: &mut dyn ServerHost) -> u32 {
        self.held()
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        self.1
    }
    fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        let went = amount.min(self.1 - self.held());
        self.0.fetch_add(went, Ordering::Relaxed);
        went
    }
    fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        let came = amount.min(self.held());
        self.0.fetch_sub(came, Ordering::Relaxed);
        came
    }
}

fn capability(handler: impl EnergyHandler + 'static) -> Capability<dyn EnergyHandler> {
    Capability {
        handler: Box::new(handler),
        owner: None,
    }
}

#[test]
fn a_limiting_handler_passes_at_most_its_limit_per_call() {
    let buffer = Buffer::new(0, 100);
    let limited = LimitingEnergyHandler::new(Box::new(buffer.clone()), 10, 0);
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let went = host.transaction(|host, tx| Ok::<_, ()>(limited.insert(host, tx, 50)));
    assert_eq!(went, Ok(10));
    assert_eq!(buffer.held(), 10);
    let came = host.transaction(|host, tx| Ok::<_, ()>(limited.extract(host, tx, 5)));
    assert_eq!(came, Ok(0));
}

#[test]
fn moving_from_an_infinite_source_fills_the_target_and_leaves_the_source_endless() {
    let buffer = Buffer::new(0, 64);
    let source = capability(InfiniteEnergyHandler);
    let target = capability(buffer.clone());
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    // The buffer isn't journaled, so ask only what it takes: a partial take would be undone and
    // retried, and the retry would find it already full.
    let moved = host.transaction(|host, tx| move_energy(host, &source, &target, 64, tx));
    assert_eq!(moved, Ok(64));
    assert_eq!(buffer.held(), 64);
    assert!(target.is_full(host));
    assert_eq!(source.amount(host), u32::MAX);
}

#[test]
fn the_comparator_reads_0_empty_15_full_and_vanillas_steps_between() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let signal = |host: &mut dyn ServerHost, held| {
        capability(Buffer::new(held, 100)).comparator_signal(host)
    };
    assert_eq!(signal(&mut *host, 0), 0);
    assert_eq!(signal(&mut *host, 100), 15);
    // 1 + floor(14 * 50 / 100).
    assert_eq!(signal(&mut *host, 50), 8);
    assert_eq!(signal(&mut *host, 1), 1);
}
