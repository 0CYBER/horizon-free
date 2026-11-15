//! The mods' `isPrimaryItemFor` answers (`ItemPart::is_primary_item_for`), resolved when the
//! game installs its parts and read by enchantment selection
//! ([`Enchantments::available_results`](crate::enchant::Enchantments::available_results)).
//!
//! Selection runs inside the loot, trade and spawn-gear rolls, which hold the level's random,
//! so it can't ask a part there. The game asks each item's parts once per registered
//! enchantment at install (world open and `/reload`) and stores the answers that differ from
//! vanilla's here, as the parts themselves are installed for the process. Selection reads
//! them with one check when there are none.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// Per item id, the enchantments whose `isPrimaryItem` answer a part changed, and the answer.
pub type PrimaryAnswers = HashMap<Arc<str>, HashMap<Arc<str>, bool>>;

static ANSWERS: RwLock<Option<PrimaryAnswers>> = RwLock::new(None);
/// Whether [`ANSWERS`] holds any answer: selection's one check when none does.
static ANY: AtomicBool = AtomicBool::new(false);

/// Replace the stored answers with `answers`: only those that differ from vanilla's. An empty
/// table clears them (no part answers).
pub fn install(answers: PrimaryAnswers) {
    let any = answers.values().any(|by_enchantment| !by_enchantment.is_empty());
    let mut stored = ANSWERS.write().unwrap_or_else(PoisonError::into_inner);
    *stored = any.then_some(answers);
    ANY.store(any, Ordering::Release);
}

/// The part's answer for `item` and `enchantment`, when it differs from vanilla's.
pub(crate) fn answer(item: &str, enchantment: &str) -> Option<bool> {
    if !ANY.load(Ordering::Acquire) {
        return None;
    }
    ANSWERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()?
        .get(item)?
        .get(enchantment)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchant::tests::{sample, sample_items};

    #[test]
    fn selection_reads_the_stored_answers() {
        // Ids no other test of this crate selects for: the answers are process-wide.
        const PROBE: &str = "test:primary_probe";
        const SWORD: &str = "minecraft:iron_sword";
        let (e, items) = (sample(), sample_items());
        let sharpness: Arc<str> = "minecraft:sharpness".into();
        let at_30 = |item: &str| e.available_results(&items, 30, item, &[sharpness.clone()]);
        let level_3 = [(sharpness.clone(), 3)];
        // Vanilla: the probe item isn't primary for sharpness; an iron sword gets level 3,
        // the highest whose window (23 to 43) holds 30.
        assert!(at_30(PROBE).is_empty());
        assert_eq!(at_30(SWORD), level_3);
        let answers = |item: &str, primary| {
            HashMap::from([(item.into(), HashMap::from([(sharpness.clone(), primary)]))])
        };
        install(answers(PROBE, true));
        assert_eq!(at_30(PROBE), level_3);
        assert_eq!(at_30(SWORD), level_3);
        install(answers(SWORD, false));
        assert!(at_30(SWORD).is_empty());
        assert!(at_30(PROBE).is_empty());
        // Cleared: vanilla again.
        install(PrimaryAnswers::new());
        assert_eq!(at_30(SWORD), level_3);
    }
}
