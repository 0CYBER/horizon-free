//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// `MinecartTNT.explode`: with explosions disabled, only a primed cart is
    /// discarded. An unprimed cart survives, including its current tick.
    pub(super) fn explode_cart(&mut self, c: &mut Minecart, speed_sqr: f64) -> bool {
        let explodes = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES);
        if !explodes && c.fuse < 0 {
            return false;
        }
        let target = super::super::commands::Target::Cart(c.id);
        if self.persist.destroying_vehicles.contains(&target) {
            return true;
        }
        let power = if explodes {
            let speed = speed_sqr.sqrt().min(5.0);
            (f64::from(c.explosion_power)
                + f64::from(c.explosion_speed_factor) * c.random.next_double() * 1.5 * speed)
                as f32
        } else {
            0.0
        };
        let at = [c.position.x, c.position.y, c.position.z];
        // Publish the entire tick state, including the random draw, before
        // dismount hooks run. store_back transfers the resident owner's
        // attachment lifetime to the replacement; removing that owner later
        // also preserves any changes made by the hooks.
        if let Some(i) = self.carts.iter().position(|x| x.id == c.id)
            && !self.store_cart_back(i, c.clone())
        {
            return true;
        }
        // The cart is the explosion's source: it is gone before the blast
        // looks for entities, so it can't hit itself.
        let silent = explodes && self.entity_silent(c.id);
        if self
            .with_destroying_vehicle(target, |game| {
                game.dismount_destroyed_vehicle_players(c.id);
                if let Some(i) = game
                    .carts
                    .iter()
                    .position(|x| x.id == c.id && x.join == c.join)
                {
                    game.eject_cart_mobs(i);
                }
                if game.carts.iter().any(|x| x.id == c.id && x.join == c.join) {
                    game.release_vehicle_container(c.id);
                }
                if let Some(i) = game
                    .carts
                    .iter()
                    .position(|x| x.id == c.id && x.join == c.join)
                {
                    let removed = game.carts.remove(i);
                    game.retain_removed_vehicle(
                        c.id,
                        super::super::persist::RemovedVehicle::Cart(removed),
                    );
                }
            })
            .is_none()
        {
            return true;
        }
        if !explodes {
            return true;
        }
        // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
        let interaction =
            self.destroy_type(horizon_client::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY);
        // MinecartTNT.isPrimed (fuse > -1): a cart that ran into a wall or
        // fell unprimed blows rails up like any blast.
        self.combat.rail_proof_blast = c.fuse >= 0;
        self.combat.explosion_in_water = self.bounds_in_water(c.bounds());
        self.combat.explosion_direct = Some(c.id);
        let _ = self.with_hive_emptying_blast(|g| {
            g.server_explode_from(
                at,
                power,
                interaction,
                false,
                super::super::explosion::ExplosionSource::default()
                    .with_silence(silent)
                    .with_entity(c.id, "minecraft:tnt_minecart"),
            )
        });
        self.combat.explosion_direct = None;
        self.combat.explosion_in_water = false;
        self.combat.rail_proof_blast = false;
        true
    }
    /// Accepted damage is distinct from survival, destruction, or TNT priming.
    pub(in crate::game) fn hurt_cart_result(
        &mut self,
        i: usize,
        mut damage: f32,
        creative: bool,
        fire: bool,
        hit: CartHit,
    ) -> bool {
        let id = self.carts[i].id;
        let join = self.carts[i].join;
        let target = crate::game::commands::Target::Cart(id);
        if self.persist.destroying_vehicles.contains(&target) {
            return false;
        }
        let kind = self.carts[i].kind.id();
        let mut cancel = false;
        self.native_vehicle_damage(kind, &mut damage, &mut cancel);
        if cancel {
            return false;
        }
        let Some(i) = self.carts.iter().position(|c| {
            c.id == id
                && c.join == join
                && !self
                    .persist
                    .removals
                    .contains(&crate::game::commands::Target::Cart(id))
        }) else {
            return true;
        };
        let destroyed = self.carts.update(i, |c| {
            c.hurt_dir = -c.hurt_dir;
            c.hurt_time = 10;
            c.damage += damage * 10.0;
            // MinecartTNT.shouldSourceDestroy: fire and explosions destroy
            // (so ignite) a TNT cart whatever damage it has taken.
            cart_is_destroyed(creative, c.damage, fire && c.kind == CartKind::Tnt)
        });
        // VehicleEntity.hurtServer: `gameEvent(ENTITY_DAMAGE, source.getEntity())`.
        let hurt_at = self.carts[i].position.components();
        self.entity_event_by("minecraft:entity_damage", hurt_at, hit.by);
        if destroyed {
            let speed = self.carts[i].velocity.x * self.carts[i].velocity.x
                + self.carts[i].velocity.z * self.carts[i].velocity.z;
            let tnt_prime =
                self.carts[i].kind == CartKind::Tnt && (fire || speed >= f64::from(0.01f32));
            if tnt_prime {
                let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
                let mut c = self.carts.extract(self.carts[i].join).unwrap();
                // MinecartTNT.destroy: prime instead of breaking.
                if c.fuse < 0 {
                    self.prime_cart(&mut c);
                    c.fuse = c.random.next_int_bound(20).unwrap_or(0)
                        + c.random.next_int_bound(20).unwrap_or(0);
                }
                self.carts.put_back(i, c);
            } else {
                let join = self.carts[i].join;
                let at = self.carts[i].position.components();
                self.native_vehicle_destroy(self.carts[i].kind.id(), at);
                if !self.carts.iter().any(|c| {
                    c.id == id && c.join == join && !self.persist.removals.contains(&target)
                }) {
                    return true;
                }
                let _ = self.with_destroying_vehicle(target, |game| {
                    game.dismount_destroyed_vehicle_players(id);
                    let Some(i) = game.carts.iter().position(|c| {
                        c.id == id && c.join == join && !game.persist.removals.contains(&target)
                    }) else {
                        return;
                    };
                    game.eject_cart_mobs(i);
                    if !game.carts.iter().any(|c| {
                        c.id == id && c.join == join && !game.persist.removals.contains(&target)
                    }) {
                        return;
                    }
                    game.release_vehicle_container(id);
                    let Some(_i) = game.carts.iter().position(|c| {
                        c.id == id && c.join == join && !game.persist.removals.contains(&target)
                    }) else {
                        return;
                    };
                    game.unpack_loot(super::super::container_loot::LootTarget::Cart(id), None);
                    let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
                    let Some(i) = game.carts.iter().position(|c| c.id == id && c.join == join)
                    else {
                        return;
                    };
                    let c = game.carts.remove(i);
                    let at = c.position.components();
                    // AbstractMinecartContainer.remove: contents drop even when
                    // entity drops are disabled.
                    game.drop_vehicle_contents(at, &c.items);
                    if !creative {
                        game.entity_killed_event(at, false);
                    }
                    if !creative
                        && game
                            .ui
                            .rules
                            .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
                    {
                        game.spawn_item_own(game.cart_drop(c.kind, c.id), at, 10);
                        let empty = vec![None; c.items.len()];
                        game.drop_vehicle_contents(at, &empty);
                        // ...and angers the piglins that see the player who broke it.
                        game.cart_destroy_angers(hit, c.kind);
                    }
                    game.retain_removed_vehicle(id, super::super::persist::RemovedVehicle::Cart(c));
                });
            }
        }
        true
    }
}
