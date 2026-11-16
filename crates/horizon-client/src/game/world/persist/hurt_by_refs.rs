//! `LivingEntity.lastHurtByMob` is an `EntityReference`: a UUID that stands for
//! the entity once it is in the level. What a mob saves of it, how a value read
//! back resolves, and the same for the player in `lastHurtByPlayer`.
use super::Game;
use horizon_client::mobs::{Mob, Who};
use horizon_core::nbt::{JavaString, Tag};

impl Game<'_> {
    /// `LivingEntity.addAdditionalSaveData`'s `last_hurt_by_mob` and
    /// `ticks_since_last_hurt_by_mob`, a PARTIAL projection at save time: both
    /// from `ai.last_hurt_by` while the attacker is a living mob in the level
    /// with a registered UUID and at most 100 ticks have passed (`baseTick`
    /// drops the record after that, or once the attacker is dead). Otherwise
    /// both keys go, a copy read from loaded data included. A player attacker
    /// is not written. A reference read back that names nothing in the level
    /// yet (`ai.last_hurt_by_ref`) is written as it is. This is not
    /// `baseTick`'s cleanup itself, the AI's record, or `#no_anger`.
    pub(in crate::game) fn put_last_hurt_by_mob(&self, m: &Mob, tag: &mut Tag) {
        let Tag::Compound(c) = tag else { return };
        // A reference read back and not resolved yet (its attacker is not in
        // the level) is kept as it is, for ever: `baseTick` cannot resolve it,
        // so it does not end it.
        if let Some(uuid) = m.ai.last_hurt_by_ref {
            c.insert("last_hurt_by_mob".into(), Tag::IntArray(uuid.to_vec()));
            c.insert(
                "ticks_since_last_hurt_by_mob".into(),
                Tag::Int((m.age as i32).wrapping_sub(m.ai.last_hurt_by_time)),
            );
            return;
        }
        let since = (m.age as i32).wrapping_sub(m.ai.last_hurt_by_time);
        let reference = match m.ai.last_hurt_by {
            Some(Who::Mob(attacker))
                if since <= 100 && self.mobs.iter().any(|a| a.id == attacker && a.alive()) =>
            {
                self.persist.uuid_of(attacker)
            }
            _ => None,
        };
        match reference {
            Some(uuid) => {
                c.insert("last_hurt_by_mob".into(), Tag::IntArray(uuid.to_vec()));
                c.insert("ticks_since_last_hurt_by_mob".into(), Tag::Int(since));
            }
            None => {
                c.remove(&JavaString::from("last_hurt_by_mob"));
                c.remove(&JavaString::from("ticks_since_last_hurt_by_mob"));
            }
        }
    }
    /// `EntityReference.getEntity` for `last_hurt_by_mob` read back from a save:
    /// a mob whose reference names an entity now in the level (a mob, or the
    /// player) takes it as `last_hurt_by`, and `baseTick` ends it as it does
    /// any other. One naming nothing here stays a bare UUID, still saved. A
    /// reference to the mob itself resolves like any other (`getEntity` looks
    /// up the UUID and does not exclude the holder).
    pub(in crate::game) fn resolve_last_hurt_by_refs(&mut self) {
        let resolved: Vec<(usize, Who)> = self
            .mobs
            .iter()
            .enumerate()
            .filter_map(|(i, m)| Some((i, self.who_by_uuid(m.ai.last_hurt_by_ref?)?)))
            .collect();
        for (i, who) in resolved {
            let ai = &mut self.mobs[i].ai;
            ai.last_hurt_by = Some(who);
            ai.last_hurt_by_ref = None;
        }
    }
}
