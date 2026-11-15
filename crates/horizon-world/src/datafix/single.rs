//! One block entity's or entity's saved data upgraded on its own, for callers that hold it outside a chunk (a mod
//! reading a schematic or an old structure): the fixes the chunk loaders run on it, between a depth check of the
//! input and one of the result.
use super::{CURRENT, Ty, key, schema, update};
use horizon_core::nbt::{Compound, JavaString, Tag};

/// The deepest nesting accepted: a list or compound at depth `MAX_DEPTH` (the root compound is at 0) is refused.
/// The depth cap of every NBT read the game makes (`nbt::Limits`, vanilla's `NbtAccounter` default of 512), counted
/// on the encoded form: the compound the NBT writer puts around an element of a mixed list is a level of its own,
/// so such an element sits two levels below its list. There is no size cap: vanilla puts no quota on saved data
/// and never limits fixing or writing it.
pub const MAX_DEPTH: usize = 512;

/// Why a value was not upgraded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeError {
    /// The data version is below zero.
    NegativeVersion(i32),
    /// The data version is newer than the game's ([`CURRENT`]): the fixes only go forward.
    FutureVersion(i32),
    /// The input has a list or compound [`MAX_DEPTH`] levels deep or more, as encoded: refused before any fix.
    TooDeep,
    /// The fixes made the value deeper (1.9's `Riding` to `Passengers` doubles a chain's depth) and the result has a
    /// list or compound [`MAX_DEPTH`] levels deep or more, as encoded: no NBT read, vanilla's included, could take
    /// it back.
    UpgradedTooDeep,
}

impl std::fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NegativeVersion(v) => write!(f, "data version {v} is negative"),
            Self::FutureVersion(v) => {
                write!(f, "data version {v} is newer than the game's ({CURRENT})")
            }
            Self::TooDeep => write!(f, "the data nests {MAX_DEPTH} levels deep or more"),
            Self::UpgradedTooDeep => {
                write!(f, "the upgraded data nests {MAX_DEPTH} levels deep or more")
            }
        }
    }
}

impl std::error::Error for UpgradeError {}

/// One block entity's NBT saved at `data_version`, upgraded as the chunk loader upgrades the block entities of a
/// chunk of that version: each block entity type's fixes, up to [`CURRENT`]. Data of the current version comes back
/// as it is, and so does a block entity whose `id` that version doesn't know, as a chunk keeps it. Fixes of the
/// whole chunk (the blocks around it, the removal of bed block entities) have no chunk to act on and don't run. A
/// tag the fixes can't read comes back as it was saved. Refused before any fix runs: a negative or future version,
/// and a value with a list or compound [`MAX_DEPTH`] deep ([`UpgradeError::TooDeep`]); after the fixes, a result
/// that deep ([`UpgradeError::UpgradedTooDeep`]).
pub fn upgrade_block_entity(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let tag = checked(data_version, tag)?;
    let id = Tag::compound([(
        "id",
        tag.get(&JavaString::from("id"))
            .cloned()
            .unwrap_or(Tag::End),
    )]);
    // Before 1.13 the known types come from the fixers' tables too: a failure there keeps the data, as a fix's would.
    let known =
        std::panic::catch_unwind(|| schema::block_entity_known(&id, key(data_version as u32, 0)));
    if known.unwrap_or(false) {
        upgraded(Ty::BlockEntity, tag, data_version)
    } else {
        Ok(tag)
    }
}

/// One entity's NBT saved at `data_version`, upgraded as the entity chunk loader upgrades the entities of a chunk of
/// that version: each entity type's fixes, its passengers' included, up to [`CURRENT`]. Data of the current version
/// comes back as it is; a tag the fixes can't read comes back as it was saved. Refused as
/// [`upgrade_block_entity`] refuses.
pub fn upgrade_entity(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let tag = checked(data_version, tag)?;
    upgraded(Ty::EntityTree, tag, data_version)
}

/// `tag` when its version can be upgraded from and it is within [`MAX_DEPTH`]. A refused value is taken apart
/// without recursion, as a value too deep could not be dropped otherwise.
fn checked(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let refusal = if data_version < 0 {
        Some(UpgradeError::NegativeVersion(data_version))
    } else if data_version > CURRENT {
        Some(UpgradeError::FutureVersion(data_version))
    } else {
        (!within_depth(&tag)).then_some(UpgradeError::TooDeep)
    };
    match refusal {
        None => Ok(tag),
        Some(error) => {
            dismantle(Tag::Compound(tag));
            Err(error)
        }
    }
}

/// [`update`] of one value of type `root` up to [`CURRENT`]. A value the fixes fail on (a legacy fixer's panic on
/// data it can't read) comes back as it was saved; the input check has already kept anything too deep away. A
/// result past [`MAX_DEPTH`] is refused, and taken apart without recursion.
fn upgraded(root: Ty, tag: Compound, data_version: i32) -> Result<Compound, UpgradeError> {
    if data_version == CURRENT {
        return Ok(tag);
    }
    let mut value = Tag::Compound(tag.clone());
    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        update(root, &mut value, data_version, CURRENT);
    }))
    .is_ok();
    match value {
        Tag::Compound(upgraded) if done => {
            dismantle(Tag::Compound(tag));
            if within_depth(&upgraded) {
                Ok(upgraded)
            } else {
                dismantle(Tag::Compound(upgraded));
                Err(UpgradeError::UpgradedTooDeep)
            }
        }
        value => {
            dismantle(value);
            Ok(tag)
        }
    }
}

/// Whether no list or compound of `tag` sits [`MAX_DEPTH`] levels deep or more as the NBT writer encodes it, walked
/// without recursion. A list written as a list of compounds (all compounds, or mixed types) puts each element that
/// is no compound, or is a compound that looks like a wrapper (one entry with an empty name), in a compound of its
/// own: that wrapper is a level, as it is on disk and on the wire, and the element sits below it.
fn within_depth(tag: &Compound) -> bool {
    let mut stack: Vec<(&Tag, usize)> = tag.values().map(|value| (value, 1)).collect();
    while let Some((value, depth)) = stack.pop() {
        match value {
            Tag::List(items) => {
                if depth >= MAX_DEPTH {
                    return false;
                }
                let first = items.first().map_or(0, Tag::id);
                let compounds = first == 10 || items.iter().any(|item| item.id() != first);
                for item in items {
                    let wrapper = matches!(item, Tag::Compound(c)
                        if c.len() == 1 && c.contains_key(&JavaString::default()));
                    if compounds && (item.id() != 10 || wrapper) {
                        // The wrapper at `depth + 1`, the element inside it at `depth + 2`.
                        if depth + 1 >= MAX_DEPTH {
                            return false;
                        }
                        stack.push((item, depth + 2));
                    } else {
                        stack.push((item, depth + 1));
                    }
                }
            }
            Tag::Compound(entries) => {
                if depth >= MAX_DEPTH {
                    return false;
                }
                stack.extend(entries.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    true
}

/// Drops `tag` without recursion: every nested list and compound is emptied into a work list first.
fn dismantle(tag: Tag) {
    let mut pending = vec![tag];
    while let Some(value) = pending.pop() {
        match value {
            Tag::List(items) => pending.extend(items),
            Tag::Compound(entries) => pending.extend(entries.into_values()),
            _ => {}
        }
    }
}
