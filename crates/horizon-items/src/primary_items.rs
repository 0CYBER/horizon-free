//! The mods' `isPrimaryItemFor` answers (`ItemPart::is_primary_item_for`), held by the
//! enchantment registry snapshot they were resolved for and read by enchantment selection
//! ([`Enchantments::available_results`](crate::enchant::Enchantments::available_results)).
//!
//! Selection runs inside the loot, trade and spawn-gear rolls, which hold the level's random,
//! so it can't ask a part there. The logical server asks each item's parts once per
//! registered enchantment when its parts install (world open and `/reload`) and stores the
//! answers that differ from vanilla's on the [`Enchantments`](crate::enchant::Enchantments)
//! its stations, items and loot share, as `MobData` holds the resolved `isPathfindable`
//! answers. Each snapshot keeps its own answers; the server retires them when its world
//! closes. Selection reads them with one check when there are none.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// Per item id, the enchantments whose `isPrimaryItem` answer a part changed, and the answer.
pub type PrimaryAnswers = HashMap<Arc<str>, HashMap<Arc<str>, bool>>;

/// One registry snapshot's resolved answers: written when the server installs its parts,
/// read by every selection over that snapshot.
#[derive(Default)]
pub struct PrimaryAnswerTable {
    answers: RwLock<PrimaryAnswers>,
    /// Whether `answers` holds any answer: selection's one check when none does.
    any: AtomicBool,
}

impl Clone for PrimaryAnswerTable {
    fn clone(&self) -> Self {
        let answers = self.read().clone();
        let any = AtomicBool::new(self.any.load(Ordering::Acquire));
        Self {
            answers: RwLock::new(answers),
            any,
        }
    }
}

impl PrimaryAnswerTable {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, PrimaryAnswers> {
        self.answers.read().unwrap_or_else(PoisonError::into_inner)
    }
    /// Replace the stored answers with `answers`: only those that differ from vanilla's. An
    /// empty table retires them.
    pub fn install(&self, mut answers: PrimaryAnswers) {
        answers.retain(|_, by_enchantment| !by_enchantment.is_empty());
        let any = !answers.is_empty();
        let mut stored = self.answers.write().unwrap_or_else(PoisonError::into_inner);
        *stored = answers;
        self.any.store(any, Ordering::Release);
    }
    /// Retire every stored answer: selection is vanilla's again.
    pub fn clear(&self) {
        self.install(PrimaryAnswers::new());
    }
    /// Whether any answer is stored.
    pub fn is_empty(&self) -> bool {
        !self.any.load(Ordering::Acquire)
    }
    /// The part's answer for `item` and `enchantment`, when it differs from vanilla's.
    pub fn answer(&self, item: &str, enchantment: &str) -> Option<bool> {
        if self.is_empty() {
            return None;
        }
        self.read().get(item)?.get(enchantment).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchant::tests::{sample, sample_items};

    fn answers(sharpness: &Arc<str>, item: &str, primary: bool) -> PrimaryAnswers {
        HashMap::from([(item.into(), HashMap::from([(sharpness.clone(), primary)]))])
    }

    #[test]
    fn selection_reads_the_snapshots_own_answers() {
        const PROBE: &str = "test:primary_probe";
        const SWORD: &str = "minecraft:iron_sword";
        let (e, items) = (sample(), sample_items());
        let sharpness: Arc<str> = "minecraft:sharpness".into();
        let at_30 = |e: &crate::enchant::Enchantments, item: &str| {
            e.available_results(&items, 30, item, std::slice::from_ref(&sharpness))
        };
        let level_3 = [(sharpness.clone(), 3)];
        // Vanilla: the probe item isn't primary for sharpness; an iron sword gets level 3,
        // the highest whose window (23 to 43) holds 30.
        assert!(at_30(&e, PROBE).is_empty());
        assert_eq!(at_30(&e, SWORD), level_3);
        e.primary_answers()
            .install(answers(&sharpness, PROBE, true));
        assert_eq!(at_30(&e, PROBE), level_3);
        assert_eq!(at_30(&e, SWORD), level_3);
        // A reload installs the new answers in place of the old.
        e.primary_answers()
            .install(answers(&sharpness, SWORD, false));
        assert!(at_30(&e, SWORD).is_empty());
        assert!(at_30(&e, PROBE).is_empty());
        // Retired: vanilla again.
        e.primary_answers().clear();
        assert_eq!(at_30(&e, SWORD), level_3);
        assert!(e.primary_answers().is_empty());
    }

    #[test]
    fn two_snapshots_keep_their_own_answers() {
        let items = sample_items();
        let (a, b) = (sample(), sample());
        let sharpness: Arc<str> = "minecraft:sharpness".into();
        let at_30 = |e: &crate::enchant::Enchantments| {
            e.available_results(
                &items,
                30,
                "minecraft:iron_sword",
                std::slice::from_ref(&sharpness),
            )
        };
        a.primary_answers()
            .install(answers(&sharpness, "minecraft:iron_sword", false));
        // b never resolved: vanilla. Installing into b leaves a's answer alone.
        assert!(at_30(&a).is_empty());
        assert_eq!(at_30(&b), [(sharpness.clone(), 3)]);
        b.primary_answers().install(PrimaryAnswers::new());
        assert!(at_30(&a).is_empty());
        // A clone carries the answers it was copied with, and is its own table after.
        let copy = a.clone();
        a.primary_answers().clear();
        assert!(at_30(&copy).is_empty());
        assert_eq!(at_30(&a), [(sharpness.clone(), 3)]);
    }
}
