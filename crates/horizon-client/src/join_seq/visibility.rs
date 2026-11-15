//! Snapshot visibility of owners temporarily absent from their resident storage.
//! Independent of residency producers: a sink holds no Game or entity roster.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// Process-wide outstanding entity passes, including unbound owners and parked levels.
/// Admission is serialized with lending on the game thread.
static PASSES: AtomicUsize = AtomicUsize::new(0);

pub fn passes_settled() -> bool {
    PASSES.load(Ordering::Relaxed) == 0
}

/// Lexical obligation for rows held outside their authoritative storage.
#[derive(Debug)]
pub struct PassLease(());
impl PassLease {
    pub fn enter() -> Self {
        PASSES.fetch_add(1, Ordering::Relaxed);
        Self(())
    }
}
impl Drop for PassLease {
    fn drop(&mut self) {
        PASSES.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct Epoch(AtomicUsize);

/// One game's shared visibility signal, including parked dimensions.
/// Snapshot admission and owner lending must be serialized on the Game thread:
/// `settled` followed by saving is NOT atomic against a concurrent `enter`.
#[derive(Clone, Debug, Default)]
pub struct Visibility(Arc<Mutex<Arc<Epoch>>>);
impl Visibility {
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// Borrow an owner until this lease drops. Nested borrows are independent.
    pub fn enter(&self) -> Lease {
        let epoch = self.0.lock().unwrap();
        epoch.0.fetch_add(1, Ordering::Relaxed);
        Lease {
            epoch: epoch.clone(),
            _pass: PassLease::enter(),
        }
    }
    /// Whether this epoch has no tracked outstanding leases. This describes
    /// visibility only if callers have not expired a still-borrowed owner.
    pub fn settled(&self) -> bool {
        self.0.lock().unwrap().0.load(Ordering::Relaxed) == 0
    }
    /// Expire abandoned markers at a settled tick/epoch boundary. Returns their
    /// count for the caller's existing imbalance reporting; this does not restore
    /// entities. Old leases can subsequently drop without touching the new epoch.
    /// The caller MUST first establish that no real borrowed owner survives the
    /// boundary. Never use expiry as a pre-save imbalance reset: it can report
    /// settled while an old owner still exists outside resident storage.
    pub fn expire(&self) -> usize {
        let mut epoch = self.0.lock().unwrap();
        let count = epoch.0.load(Ordering::Relaxed);
        if count != 0 {
            *epoch = Arc::default();
        }
        count
    }
}

/// Owned visibility obligation. Transfer this value instead of entering twice.
#[derive(Debug)]
pub struct Lease {
    epoch: Arc<Epoch>,
    _pass: PassLease,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.epoch.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(super) struct Binding {
    pub sink: Visibility,
    pub lease: Option<Lease>,
}
impl Binding {
    pub fn resident(sink: &Visibility) -> Self {
        Self {
            sink: sink.clone(),
            lease: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_seq::{JoinList, JoinSeq, ResidentList, ResidentSlot, RestoreOrder};
    #[derive(Clone, Debug)]
    struct Row {
        join: JoinSeq,
    }
    crate::impl_joined!(Row);
    fn row() -> Row {
        Row {
            join: JoinSeq::UNJOINED,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn unbound_passes_block_until_restore_consumption_and_unwind() {
        // The admission signal is process-wide. Isolate this test from other
        // tests legitimately lifting owners concurrently in the same runner.
        const CHILD: &str = "HORIZON_SAVE_LEASE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--include-ignored", "--exact", "join_seq::visibility::tests::unbound_passes_block_until_restore_consumption_and_unwind"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "test=join_seq::visibility::tests::unbound_passes_block_until_restore_consumption_and_unwind exit_code={:?} category=fixture-child-failed",
                output.status.code()
            );
            return;
        }
        assert!(passes_settled());
        let mut list = JoinList::<Row>::default();
        list.push(row());
        let mut pass = list.lift();
        assert!(!passes_settled());
        let owned = pass.take_owned_pass();
        drop(pass);
        assert!(!passes_settled());
        list.restore(owned, RestoreOrder::SurvivorsFirst);
        assert!(passes_settled());
        let mut iter = list.lift().into_iter();
        let _row = iter.next().unwrap();
        assert!(!passes_settled());
        drop(iter);
        assert!(passes_settled());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _pass = list.lift();
            let _bare = PassLease::enter();
            assert!(!passes_settled());
            panic!("listener");
        }));
        assert!(panic.is_err());
        assert!(passes_settled());
    }

    #[test]
    fn intrinsic_lifts_transfer_restore_and_drop_without_double_counting() {
        let sink = Visibility::default();
        let mut list = JoinList::<Row>::default();
        list.bind_visibility(&sink);
        list.push(row());
        let mut pass = list.lift();
        assert!(!sink.settled());
        let owned = pass.take_owned_pass();
        drop(pass);
        assert_eq!(sink.expire(), 1, "handoff is one lift, not two");
        list.restore(owned, RestoreOrder::SurvivorsFirst);
        assert!(sink.settled());
        for order in [RestoreOrder::SurvivorsFirst, RestoreOrder::SpawnsFirst] {
            let pass = list.lift();
            list.push(row());
            assert!(!sink.settled());
            list.restore(pass, order);
            assert!(sink.settled());
        }
        let pass = list.lift();
        let mut clone = pass.clone();
        let clone_pass = clone.lift();
        assert!(!sink.settled(), "cloning cannot release the owner lease");
        drop(pass);
        assert!(sink.settled(), "snapshot clones must not publish a lift");
        drop(clone_pass);
    }

    #[test]
    fn consuming_loops_and_bare_slot_extraction_hold_explicit_leases() {
        let sink = Visibility::default();
        let mut list = JoinList::<Row>::default();
        list.bind_visibility(&sink);
        list.push(row());
        let mut iter = list.lift().into_iter();
        let _row = iter.next().unwrap();
        assert!(!sink.settled());
        drop(iter);
        assert!(sink.settled());
        let mut rebuilt = JoinList::default();
        rebuilt.push(row());
        list.restore(rebuilt, RestoreOrder::SurvivorsFirst);
        let pass = list.lift();
        assert!(
            !sink.settled(),
            "unbound reconstruction keeps resident binding"
        );
        list.restore(pass, RestoreOrder::SurvivorsFirst);
        let mut slot = ResidentSlot::default();
        slot.bind_visibility(&sink);
        slot.install(row()).unwrap();
        let lease = slot.visibility_lease();
        let extracted = slot.extract().unwrap();
        assert!(!sink.settled());
        slot.put_back(extracted);
        drop(lease);
        assert!(sink.settled());
        let mut offwalk = ResidentList::default();
        offwalk.bind_visibility(&sink);
        offwalk.push(row());
        let pass = offwalk.lift();
        assert!(!sink.settled());
        offwalk.restore(pass, RestoreOrder::SurvivorsFirst);
        assert!(sink.settled());
    }

    #[test]
    fn nested_leases_unwind_and_expired_leases_cannot_close_a_new_epoch() {
        let sink = Visibility::default();
        let other = Visibility::default();
        let outer = sink.enter();
        assert!(!sink.settled());
        assert!(other.settled());
        let result = std::panic::catch_unwind(|| {
            let _inner = sink.enter();
            panic!("native payload");
        });
        assert!(result.is_err());
        assert_eq!(sink.expire(), 1, "only the outer obligation remains");
        assert!(
            sink.settled(),
            "expiry releases abandoned markers before reporting"
        );
        let fresh = sink.enter();
        drop(outer);
        assert!(!sink.settled(), "stale drops cannot close a fresh borrow");
        drop(fresh);
        assert!(sink.settled());
    }
}
