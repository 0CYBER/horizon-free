//! The transaction journal fake: what a host keeps on each transaction it opens.

// no wire: in-process testing harness state, not a mod boundary payload.
/// A test host's transaction journal, as the game's keeps its storages: the state each open
/// transaction began with, put back when it aborts, and every close in order.
///
/// [`FakeHost`](super::FakeHost) keeps one over its players and level. A test host of its own
/// keeps one over its state: it returns what [`open`](Self::open) gives from `transaction_open`,
/// and puts back what [`close`](Self::close) returns from `transaction_close`. Like the game's,
/// it names each transaction by a write token and a close token, refuses a close whose seal
/// names no open transaction, and aborts the transactions left open inside the one it closes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeJournal<S> {
    /// Each open transaction's write token, close token and starting state, outermost first.
    kept: Vec<(u64, u64, S)>,
    closed: Vec<(usize, bool)>,
    next: u64,
}

impl<S> Default for FakeJournal<S> {
    fn default() -> Self {
        Self {
            kept: Vec::new(),
            closed: Vec::new(),
            next: 1,
        }
    }
}

impl<S> FakeJournal<S> {
    /// Opens a transaction, nested in the open one if any, that began with state `now`, and
    /// gives back the seal that ends it.
    pub fn open(&mut self, now: S) -> crate::TransactionSeal {
        let token = self.next;
        // A close token no write token equals, as the game's are drawn apart.
        let close = token | 1 << 63;
        self.next += 1;
        self.kept.push((token, close, now));
        crate::item::transaction::transaction_seal(token, close)
    }
    /// Closes the transaction `seal` ends, committed or aborted, aborting first every one
    /// opened inside it and left open. What the host puts back: the state the first transaction
    /// aborted began with, if any. Nothing when `seal` names no open transaction.
    pub fn close(&mut self, seal: &crate::TransactionSeal, commit: bool) -> Option<S> {
        let (token, close) = crate::item::transaction::seal_tokens(seal);
        let at = self
            .kept
            .iter()
            .position(|(t, c, _)| *t == token && *c == close)?;
        let mut back = None;
        while self.kept.len() > at + 1 {
            let (_, _, was) = self.kept.pop()?;
            self.closed.push((self.kept.len() + 1, false));
            back = Some(was);
        }
        let (_, _, was) = self.kept.pop()?;
        self.closed.push((at + 1, commit));
        if !commit {
            back = Some(was);
        }
        back
    }
    /// Whether `tx` names an open transaction, for a write to check.
    pub fn holds(&self, tx: &crate::Transaction) -> bool {
        self.kept.iter().any(|(token, _, _)| *token == tx.0)
    }
    /// The innermost open transaction, its write token only.
    pub fn current(&self) -> Option<crate::Transaction> {
        let (token, _, _) = self.kept.last()?;
        Some(crate::Transaction(*token))
    }
    /// How deeply `tx` is nested, 0 for the outermost; none when it is not open.
    pub fn depth_of(&self, tx: &crate::Transaction) -> Option<u32> {
        let at = self.kept.iter().position(|(token, _, _)| *token == tx.0)?;
        u32::try_from(at).ok()
    }
    /// How many transactions are open.
    pub fn depth(&self) -> usize {
        self.kept.len()
    }
    /// Each transaction closed, in order: the depth it was open at (1 for the outermost), and
    /// whether it committed.
    pub fn closed(&self) -> &[(usize, bool)] {
        &self.closed
    }
}
