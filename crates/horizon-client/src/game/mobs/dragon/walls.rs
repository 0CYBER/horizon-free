use super::*;

impl Game<'_> {
    /// `EnderDragon.checkWalls` over the head, neck and body. A block that is
    /// neither air nor `#dragon_transparent` breaks when griefing is allowed
    /// (`canEntityGrief` for the dragon), its parts let the dragon destroy it
    /// (unanswered: not `#dragon_immune`), no mod keeps it
    /// (`LivingDestroyBlockEvent`) and it is not `#dragon_immune`, which a yes
    /// does not bypass; any other puts the dragon in a wall.
    pub(super) fn dragon_check_walls(&mut self) {
        let mut in_wall = false;
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let griefing = self.can_entity_grief(Some(d.id));
        let boxes = [d.part_box(0), d.part_box(1), d.part_box(2)];
        let dragon_id = d.id;
        let tags = &self.items().block_tags;
        let transparent = tags
            .get("minecraft:dragon_transparent")
            .cloned()
            .unwrap_or_default();
        let immune = tags
            .get("minecraft:dragon_immune")
            .cloned()
            .unwrap_or_default();
        let mut removed = Vec::new();
        for b in boxes {
            let (lo, hi) = (b.min_position(), b.max_position());
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        let state = self.world.block(x, y, z);
                        let name = &self.res.facts().get(state).name;
                        if self.res.facts().is_air(state) || transparent.contains(&**name) {
                            continue;
                        }
                        // Griefing, then what the block's parts answer.
                        let is_immune = immune.contains(&**name);
                        if griefing && self.entity_may_destroy(dragon_id, [x, y, z], !is_immune) {
                            removed.push(([x, y, z], is_immune));
                        } else {
                            in_wall = true;
                        }
                    }
                }
            }
        }
        // LivingDestroyBlockEvent: a block a mod keeps is still in the wall.
        if !mod_api::__private::mods().is_empty() {
            removed.retain(|&(p, _)| {
                let kept = !self.may_destroy(dragon_id, "minecraft:ender_dragon", p);
                in_wall |= kept;
                !kept
            });
        }
        // #dragon_immune still holds after both: such a block stays, in the wall.
        removed.retain(|&(_, is_immune)| {
            in_wall |= is_immune;
            !is_immune
        });
        let air = self.world.air;
        let broke = !removed.is_empty();
        for (p, _) in removed {
            let _ = self.level_set(p, air);
        }
        if broke && let Some(d) = self.dragon.as_ref() {
            let p = d.parts[0].0;
            // Level event 2008: an explosion puff where a block broke.
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Explosion),
                [p.x, p.y, p.z],
                [0.0; 3],
            );
        }
        if let Some(d) = self.dragon.as_mut() {
            d.in_wall = in_wall;
        }
    }
}
