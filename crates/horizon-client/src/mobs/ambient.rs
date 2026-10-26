//! What a client's copy of a mob makes on its own, in the
//! `level().isClientSide()` branches of `aiStep`: nothing of it reaches the
//! server or goes over the network, so the window's game runs it, whether it
//! simulates the world (in-frame) or follows a server.
use super::{Mob, MobEvent, Species};
use horizon_core::random::RandomSource;

/// `Enderman.aiStep`, `Endermite.aiStep` and `Blaze.aiStep` on a client:
/// two portal particles a tick around endermen and endermites; two large
/// smoke particles a tick around blazes and, one tick in 24, their burn.
pub fn client_ai_step(m: &mut Mob, out: &mut Vec<MobEvent>) {
    let species = m.species();
    if !matches!(
        species,
        Species::Enderman | Species::Endermite | Species::Blaze
    ) {
        return;
    }
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    let p = m.position;
    if species == Species::Blaze && m.random.next_int_bound(24).unwrap_or(1) == 0 && !m.silent {
        let volume = 1.0 + m.random.next_float();
        let pitch = m.random.next_float() * 0.7 + 0.3;
        out.push(MobEvent::LocalSound(
            "entity.blaze.burn",
            [p.x + 0.5, p.y + 0.5, p.z + 0.5],
            volume,
            pitch,
        ));
    }
    for _ in 0..2 {
        // Entity.getRandomX(0.5), getRandomY(), getRandomZ(0.5).
        let r = &mut m.random;
        let x = p.x + w * (2.0 * r.next_double() - 1.0) * 0.5;
        let y = p.y + h * r.next_double();
        let z = p.z + w * (2.0 * r.next_double() - 1.0) * 0.5;
        let (kind, at, velocity) = if species == Species::Blaze {
            ("large_smoke", [x, y, z], [0.0; 3])
        } else {
            // The enderman's are a quarter block lower.
            let y = if species == Species::Enderman {
                y - 0.25
            } else {
                y
            };
            let velocity = [
                (r.next_double() - 0.5) * 2.0,
                -r.next_double(),
                (r.next_double() - 0.5) * 2.0,
            ];
            ("portal", [x, y, z], velocity)
        };
        out.push(MobEvent::ParticleAt { kind, at, velocity });
    }
}
