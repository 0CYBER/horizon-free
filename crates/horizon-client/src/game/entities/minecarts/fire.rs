//! Fire damage to a minecart.
use super::*;
use horizon_core::nbt::Tag;

impl Game<'_> {
    /// `BaseFireBlock.entityInside` for a cart: each fire block the cart's box
    /// reaches into hurts it with `inFire` for the block's `fireDamage`. Fire
    /// counts as a TNT minecart's ignition source; a cart has no `baseTick`,
    /// so the fire it is lit with never burns it further. The old behavior
    /// applies block effects twice a tick, each time at the position the cart
    /// has then and each block once per application: right after its move
    /// (`AbstractMinecart.move`) and at the end of the tick
    /// (`OldMinecartBehavior.tick`). The other behavior applies them once.
    pub(super) fn cart_fire_tick(&mut self, id: u32, slot: usize) {
        let Some(i) = cart_at(&self.carts, id, slot) else {
            return;
        };
        let e = f64::from(1.0e-5_f32);
        let end = self.carts[i].position;
        let mut spots: Vec<Vec3> = self
            .carts
            .update(i, |c| c.fire_move_at.take())
            .into_iter()
            .collect();
        spots.push(end);
        let mut damages = Vec::new();
        for spot in spots {
            let (w, p) = (WIDTH / 2.0, spot);
            let b = Aabb::new(
                Vec3::new(p.x - w, p.y, p.z - w),
                Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
            )
            .deflate(Vec3::new(e, e, e));
            let (lo, hi) = (b.min_position(), b.max_position());
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        if let Some(d) = self.fire_damage_at([x, y, z]) {
                            damages.push(d);
                        }
                    }
                }
            }
        }
        if damages.is_empty() || self.remote.is_some() {
            return;
        }
        // BaseFireBlock.fireIgnite: a cart that is not fire immune is lit for
        // 8 seconds, whatever its invulnerability (a cart has no baseTick, so
        // the fire never counts down; its save writes the ticks back).
        self.ignite_cart(id, 160);
        for damage in damages {
            let Some(i) = cart_at(&self.carts, id, i) else {
                return;
            };
            let kind = self.carts[i].kind.id();
            let source = horizon_client::health::HurtSource {
                damage_type: "minecraft:in_fire",
                causing: 0,
                direct: 0,
                position: None,
            };
            let vanilla = self.saved_refuses_damage(id, "minecraft:in_fire", false);
            if !self.native_nonliving_invulnerable(id, kind, source, vanilla) {
                self.hurt_cart(i, damage, false, true, CartHit::default());
            }
        }
    }
    /// `Entity.igniteForTicks` for a cart: the saved `Fire` rises to `ticks`.
    fn ignite_cart(&mut self, id: u32, ticks: i16) {
        let raw = self
            .persist
            .raw
            .entry(id)
            .or_insert_with(|| Tag::Compound(horizon_core::nbt::Compound::new()));
        if let Tag::Compound(map) = raw {
            let fire = map
                .get(&"Fire".into())
                .and_then(horizon_client::nbt_path::int_value)
                .unwrap_or(0);
            if fire < i32::from(ticks) {
                map.insert("Fire".into(), Tag::Short(ticks));
            }
        }
    }
}
