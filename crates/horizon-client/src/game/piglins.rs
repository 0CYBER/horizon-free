//! The game side of piglins (`PiglinAi`): spotting and picking up gold,
//! bartering when the admiring is over, and anger at players who open
//! their chests or mine their gold.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Species, Who, piglin::ADMIRE},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

impl Game<'_> {
    fn piglin_loved(&self, item: &str) -> bool {
        self.res
            .items()
            .item_tags
            .get("minecraft:piglin_loved")
            .is_some_and(|t| t.contains(item))
    }
    /// `PiglinAi.isWearingSafeArmor`.
    pub(super) fn wearing_gold(&self) -> bool {
        let tag = self
            .res
            .items()
            .item_tags
            .get("minecraft:piglin_safe_armor");
        self.armor
            .iter()
            .flatten()
            .any(|s| tag.is_some_and(|t| t.contains(&*s.item)))
    }
    /// Piglins notice gold on the ground within 9 blocks, walk to it and
    /// take it (`GoToWantedItem`, `Piglin.pickUpItem`).
    pub(super) fn piglin_pickups(&mut self) {
        if !self.ui.rules.bool("mob_griefing") {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Piglin
                || !m.alive()
                || m.is_baby()
                || m.extra.using_item > 0
                || m.extra.egg_time > 0
                || m.extra.off_hand.is_some()
            {
                continue;
            }
            let pos = m.position;
            let best = self
                .dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| self.piglin_loved(&e.stack.item))
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .filter(|(_, d)| *d <= 81.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, _)) = best else {
                self.mobs[i].extra.point = None;
                continue;
            };
            let e = &self.dropped[j];
            let close = (e.position.x - pos.x).abs() <= 1.3
                && (e.position.z - pos.z).abs() <= 1.3
                && (e.position.y - pos.y).abs() <= 1.0;
            if !close || e.pickup_delay > 0 {
                self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
                continue;
            }
            // Nuggets come whole, anything else one at a time.
            let item = e.stack.item.clone();
            let (taken_item, id) = (e.clone(), self.mobs[i].id);
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(taken_item)),
                super::pickups::Collector::Mob(id),
            );
            let taken = if &*item == "minecraft:gold_nugget" {
                self.dropped.remove(j).stack.count
            } else {
                let e = &mut self.dropped[j];
                e.stack.count -= 1;
                if e.stack.count == 0 {
                    self.dropped.remove(j);
                }
                1
            };
            let _ = taken;
            self.start_admiring(i, item);
        }
    }
    fn start_admiring(&mut self, index: usize, item: std::sync::Arc<str>) {
        let m = &mut self.mobs[index];
        m.extra.off_hand = Some(item);
        m.extra.using_item = ADMIRE;
        m.extra.point = None;
        m.nav_stop();
        let at = m.center();
        self.audio
            .play("minecraft:entity.piglin.admiring_item", Some(at), 1.0, 1.0);
    }
    /// `PiglinAi.mobInteract`: hand a piglin something it loves.
    pub(super) fn give_piglin(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::Piglin
            || m.is_baby()
            || m.extra.using_item > 0
            || m.extra.egg_time > 0
            || !self.piglin_loved(item)
        {
            return false;
        }
        self.start_admiring(index, item.into());
        true
    }
    /// `PiglinAi.stopHoldingOffHandItem` after admiring: gold ingots buy a
    /// roll of the bartering table, thrown toward the player.
    pub(super) fn finish_admiring(&mut self, mob: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(item) = self.mobs[i].extra.off_hand.take() else {
            return;
        };
        let m = &self.mobs[i];
        let from = [m.position.x, m.position.y + 1.3, m.position.z];
        let items = if &*item == "minecraft:gold_ingot" && !m.is_baby() {
            self.res.items().loot_table(
                "minecraft:gameplay/piglin_bartering",
                false,
                None,
                &mut self.world.random,
            )
        } else {
            vec![ItemStack::new(&item, 1)]
        };
        // BehaviorUtils.throwItem toward the nearest visible player
        // (NEAREST_VISIBLE_PLAYER: within 16, alive, no spectators; the
        // guests count as much as the host), else nearby.
        let at = m.position;
        let host = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !self.spectator())
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let guests = self.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .map(|g| g.position)
        });
        let d = |p: &[f64; 3]| at.distance_squared(Vec3::new(p[0], p[1], p[2]));
        let target = host
            .into_iter()
            .chain(guests)
            .filter(|p| d(p) < 256.0)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
            .map(|p| [p[0], p[1] + 1.0, p[2]]);
        for stack in items {
            let to = target.unwrap_or_else(|| {
                let r = &mut self.world.random;
                [
                    from[0] + f64::from(r.next_float() * 8.0 - 4.0),
                    from[1],
                    from[2] + f64::from(r.next_float() * 8.0 - 4.0),
                ]
            });
            let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
            let v = [d[0] / len * 0.3, d[1] / len * 0.3 + 0.1, d[2] / len * 0.3];
            self.spawn_item(stack, from, v, 40);
        }
    }
    /// `PiglinAi.angerNearbyPiglins`.
    pub(super) fn anger_piglins(&mut self, only_if_seen: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if self.creative || self.spectator() {
            return;
        }
        let at = p.position;
        let eye = [at.x, at.y + 1.62, at.z];
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Piglin
                || m.is_baby()
                || !m.alive()
                || m.ai.target.is_some()
                || (m.position.x - at.x).abs() > 16.0
                || (m.position.y - at.y).abs() > 16.0
                || (m.position.z - at.z).abs() > 16.0
            {
                continue;
            }
            if only_if_seen && !self.can_see(m.center(), eye) {
                continue;
            }
            let m = &mut self.mobs[i];
            m.extra.anger = 600;
            m.ai.target = Some(Who::Player);
        }
    }
    /// A clear line of sight between two points (blocks with collision stop it).
    fn can_see(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let steps = 32;
        (1..steps).all(|i| {
            let f = f64::from(i) / f64::from(steps);
            let p = [
                from[0] + (to[0] - from[0]) * f,
                from[1] + (to[1] - from[1]) * f,
                from[2] + (to[2] - from[2]) * f,
            ];
            !self
                .res
                .facts()
                .get(self.world.block(
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ))
                .full_collision
        })
    }
    /// Whether breaking or opening this block angers piglins.
    pub(super) fn guarded_by_piglins(&self, name: &str) -> bool {
        self.res
            .items()
            .block_tags
            .get("minecraft:guarded_by_piglins")
            .is_some_and(|t| t.contains(name))
    }
}
