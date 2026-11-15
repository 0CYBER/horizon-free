//! Exclusive namespace ownership for bootstrap-record collection.
use super::*;

/// Exclusive namespace ownership, acquired without creating or upgrading a gate.
pub struct Cleanup {
    pub(crate) data: Dir,
    _gate: Locked,
}

impl Cleanup {
    pub fn existing(data: &Path) -> Result<Self, Error> {
        let data = open_directory(data)?;
        filesystem::local(&data)?;
        let directory = Dir::from_std_file(retained(
            data.open_dir_nofollow("run-leases")?.into_std_file(),
        )?);
        let gate = open_lock(
            &directory,
            "gate",
            LockMode::Exclusive,
            Release::Close,
            false,
        )?;
        Ok(Self { data, _gate: gate })
    }
}

#[cfg(test)]
mod tests;
