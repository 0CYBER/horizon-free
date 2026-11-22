//! `PlayerTrigger.LOCATION`, with the criteria that only ask where the player is remembered.
use super::*;

/// What the last evaluation of the `minecraft:location` trigger found about the criteria that only
/// ask where the player is: for the same tree, dimension and biome there, those not granted then
/// are all unmet, so only `rest` (the other criteria, in tree order) are asked. Kept in the
/// player's progress, so it goes with the player; a revoke of the progress forgets it (a criterion
/// that was granted when it was made is asked again; so does a `/reload`, which can change the
/// tags the criteria read without making another tree), a grant cannot make it wrong (a granted
/// criterion is never asked).
struct LocationMemo {
    tree: Arc<Tree>,
    /// `AdvState::reloads` when it was made.
    reloads: u32,
    dimension: String,
    biome: u16,
    rest: Vec<(usize, usize)>,
}

#[cfg(test)]
thread_local! {
    /// Test only: no evaluation is skipped (the way it was).
    pub(super) static NO_MEMO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test only: the evaluations that skipped the criteria that only ask where the player is.
    /// Test only: the parts of the key a memo does not look at (1 tree, 2 dimension, 4 biome,
    /// 8 reload count), as controls.
    pub(super) static IGNORED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    pub(super) static SKIPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "location_memo_tests.rs"]
mod tests;

/// Whether a criterion's conditions only ask where the player is by biome or dimension
/// (`entity_properties` of `this` with a `location` predicate of those alone): what they answer
/// depends on the place and nothing else of the player. A `structures` condition is not one: the
/// structure data of a loaded column can change without anything the memo keys on changing.
fn asks_only_where(conditions: &Value) -> bool {
    let Some(only) = conditions.as_object().filter(|o| o.len() == 1) else {
        return false;
    };
    let Some(player) = only.get("player").and_then(Value::as_object) else {
        return false;
    };
    if player.len() != 3
        || player.get("type").and_then(Value::as_str) != Some("minecraft:entity_properties")
        || player.get("entity").and_then(Value::as_str) != Some("this")
    {
        return false;
    }
    let Some(predicate) = player.get("predicate").and_then(Value::as_object) else {
        return false;
    };
    predicate.len() == 1
        && predicate
            .get("minecraft:location")
            .and_then(Value::as_object)
            .is_some_and(|location| {
                !location.is_empty()
                    && location
                        .keys()
                        .all(|k| matches!(k.as_str(), "biomes" | "dimension"))
            })
}

impl Game<'_> {
    /// `ServerPlayer.doTick`'s `CriteriaTriggers.LOCATION.trigger`.
    pub(super) fn tick_location(&mut self) {
        if self.lan_acting_fake || self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let Some(list) = tree.by_trigger.get("minecraft:location") else {
            return;
        };
        let me = self.player_entity_for("minecraft:location");
        let test = |c: &Value, w: &dyn adv::World| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &ctx, w)
        };
        // The criteria that only ask where the player is were all found unmet at this biome, for
        // this tree, the last time: they are not asked again. The others are asked every time.
        let place = self.location_place();
        let memo = self.adv.progress.memo.clone();
        let known = place
            .as_ref()
            .zip(memo.as_ref().and_then(|m| m.downcast_ref::<LocationMemo>()));
        #[cfg(test)]
        let known = known.filter(|_| !NO_MEMO.get());
        if let Some(((dimension, biome), m)) = known
            && m.is(&tree, self.adv.reloads, dimension, *biome)
        {
            #[cfg(test)]
            SKIPS.set(SKIPS.get() + 1);
            self.trigger_criteria(&tree, &m.rest, test);
            return;
        }
        // The memo is stored before the criteria are granted: a reward function that revokes one
        // of them clears it (`Progress::revoke`), and must not find it put back.
        if let Some((dimension, biome)) = place {
            let rest = list
                .iter()
                .copied()
                .filter(|&(ai, ci)| !asks_only_where(&tree.list[ai].criteria[ci].conditions))
                .collect();
            self.adv.progress.memo = Some(Arc::new(LocationMemo {
                tree: tree.clone(),
                reloads: self.adv.reloads,
                dimension: dimension.to_owned(),
                biome,
                rest,
            }));
        }
        self.trigger_criteria(&tree, list, test);
    }

    /// The dimension and the biome there of the player, which the criteria that only ask where
    /// the player is answer from; none when the column is not loaded (what is there is not known
    /// yet).
    fn location_place(&self) -> Option<(&'static str, u16)> {
        let p = self.player.as_ref()?;
        let at = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let chunk = self.world.chunks.get(&(at[0] >> 4, at[2] >> 4))?;
        Some((self.dim.id(), chunk.biome(at[0], at[1], at[2])))
    }
}

impl LocationMemo {
    /// Whether it was worked out from the same tree (the same one, not an equal one), the same
    /// reload of the data (a reload that adds no advancements keeps the tree, not the tags),
    /// dimension and biome.
    fn is(&self, tree: &Arc<Tree>, reloads: u32, dimension: &str, biome: u16) -> bool {
        #[cfg(test)]
        let ignored = IGNORED.get();
        #[cfg(not(test))]
        let ignored = 0u8;
        (ignored & 8 != 0 || self.reloads == reloads)
            && (ignored & 1 != 0 || Arc::ptr_eq(&self.tree, tree))
            && (ignored & 2 != 0 || self.dimension == dimension)
            && (ignored & 4 != 0 || self.biome == biome)
    }
}
