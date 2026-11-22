use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// Shared by every mod export in one client session, including workers that may
/// still be unwinding after an abort.
pub struct ByteBudget(AtomicU64);

impl ByteBudget {
    pub fn new(bytes: u64) -> Self {
        Self(AtomicU64::new(bytes))
    }

    pub fn fits(&self, bytes: u64) -> bool {
        self.0.load(Ordering::Relaxed) >= bytes
    }

    pub fn charge(&self, bytes: u64) -> io::Result<()> {
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(bytes)
            })
            .map(|_| ())
            .map_err(|_| io::Error::other("mod exports have used this session's budget"))
    }
    pub fn reserve(&self, bytes: u64) -> io::Result<Reservation<'_>> {
        self.charge(bytes)?;
        Ok(Reservation {
            budget: self,
            held: bytes,
        })
    }
}

/// Holds a charge until the output commits. Dropping it returns only unused bytes.
pub struct Reservation<'a> {
    budget: &'a ByteBudget,
    held: u64,
}

impl Reservation<'_> {
    fn extend(&mut self, bytes: u64) -> io::Result<()> {
        self.budget.charge(bytes)?;
        self.held += bytes;
        Ok(())
    }

    pub(super) fn spend(&mut self, bytes: u64) {
        self.held -= bytes;
    }

    pub fn commit(mut self) {
        self.held = 0;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.budget.0.fetch_add(self.held, Ordering::Relaxed);
    }
}

/// A PNG spends its RGB charge first and pays for any excess before touching disk.
pub(super) struct BudgetedWrite<'a, 'budget, W> {
    pub file: W,
    pub prepaid: u64,
    pub budget: &'a mut Reservation<'budget>,
}

impl<W: Write> Write for BudgetedWrite<'_, '_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let len = bytes.len() as u64;
        let extra = len.saturating_sub(self.prepaid);
        if extra > 0 {
            self.budget.extend(extra)?;
        }
        self.prepaid = self.prepaid.saturating_sub(len);
        self.file.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
