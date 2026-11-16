/// For each server-side harvest read of the pot witness's block (a decorated pot) on this
/// thread since the last call, in order: whether the state it was asked about was cracked.
pub fn take_pot_harvest_reads() -> Vec<bool> {
    POT_READS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
