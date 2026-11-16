//! Breeding completion: credit, partner cooldown and heart delivery.
use super::*;

impl Game<'_> {
    /// `Animal.finalizeSpawnChildFromBreeding` before the experience orb.
    pub(super) fn finalize_breeding(
        &mut self,
        parent: &Mob,
        partner: u32,
        child: Option<&Mob>,
        order: OrbOrder,
    ) {
        if let Some(cause) = breeding_credit(&mut self.love_cause, parent.id, partner) {
            let parent_view = self.mob_entity(parent);
            let partner_view = self
                .mobs
                .iter()
                .find(|m| m.id == partner)
                .map(|m| self.mob_entity(m))
                .unwrap_or_else(|| parent_view.clone());
            let child_view = child.map(|child| self.mob_entity(child));
            let who = cause.map_or(
                super::super::commands::Target::Player,
                super::super::commands::Target::Guest,
            );
            self.for_player(who, |g| {
                g.award_custom_stat("animals_bred", 1);
                g.trigger_bred(&parent_view, &partner_view, child_view.as_ref());
            });
        }
        if let Some(q) = self.mobs.iter_mut().find(|m| m.id == partner) {
            q.extra.age = 6000;
            q.extra.in_love = 0;
        }
        // TurtleBreedGoal overrides Animal's finalization and never broadcasts hearts.
        if order == OrbOrder::BeforeChild && parent.species() != Species::Turtle {
            self.mob_event_particles(parent.id, 18);
        }
    }
    /// `Rabbit.getBreedOffspring`: a fresh biome variant
    /// (`Rabbit.getRandomRabbitVariant`, from the level's random) one time in
    /// twenty, else a parent's colour by the breeding parent's own coin toss.
    /// Called before the child exists, where vanilla draws it; None for any
    /// other species.
    pub(super) fn bred_rabbit_variant(
        &mut self,
        parent: u32,
        p: &Mob,
        q: Option<&Mob>,
    ) -> Option<u8> {
        if p.species() != Species::Rabbit {
            return None;
        }
        let own = p.extra.variant;
        // `partner instanceof Rabbit`: a partner of another species draws no coin.
        let mate = q
            .filter(|q| q.species() == Species::Rabbit)
            .map(|q| q.extra.variant);
        let at = [p.position.x, p.position.y, p.position.z].map(|c| c.floor() as i32);
        let facts = self.biome_facts(at[0], at[1], at[2]);
        let fresh = horizon_client::mobs::rabbit::random_variant(
            facts.white_rabbit,
            facts.gold_rabbit,
            &mut self.world.random,
        );
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == parent) else {
            return Some(own);
        };
        if m.random.next_int_bound(20).unwrap_or(1) == 0 {
            return Some(fresh);
        }
        // The coin is drawn only for a rabbit partner.
        Some(match mate {
            Some(mate) if m.random.next_boolean() => mate,
            _ => own,
        })
    }
}
