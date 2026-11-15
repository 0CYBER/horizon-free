//! Borrow the storage a player's active guest turn actually owns.

use super::super::guest_list::GuestList;
use crate::{advancements::Progress, recipe_book::Settings};
use std::collections::BTreeSet;

/// The advancement fields swapped together when entering and leaving a guest.
pub struct AdvancementState<'a> {
    pub progress: &'a mut Progress,
    pub changed: &'a mut BTreeSet<usize>,
    pub unlocked: &'a mut Vec<String>,
    pub book: &'a mut Settings,
}

/// Active turns, in entry order, and the current turn's live storage. Other
/// players use their hub record, including suspended turns whose record holds
/// displaced progress; resolving those turns fully is tracked separately.
#[derive(Default)]
pub struct AdvancementOwners<'a> {
    turns: &'a [u32],
    live: Option<AdvancementState<'a>>,
}

impl<'a> AdvancementOwners<'a> {
    pub fn active(turns: &'a [u32], live: AdvancementState<'a>) -> Self {
        Self {
            turns,
            live: Some(live),
        }
    }

    /// Use the live storage for the acting player; other players use their hub record.
    pub fn state<'b>(
        &'b mut self,
        guests: &'b mut GuestList,
        index: usize,
    ) -> Option<AdvancementState<'b>> {
        let conn = guests.get(index)?.conn;
        if self.turns.last() == Some(&conn) {
            let live = self.live.as_mut()?;
            return Some(AdvancementState {
                progress: live.progress,
                changed: live.changed,
                unlocked: live.unlocked,
                book: live.book,
            });
        }
        guests.advancement_state(index)
    }
}
