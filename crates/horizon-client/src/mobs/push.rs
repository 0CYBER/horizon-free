//! The pushes of `LivingEntity.pushEntities`: the stack a mob rides, and the server's cap on how
//! many mobs one mob pushes per tick (`[entities] max_collisions`, off = vanilla).
use super::{Mob, MobWorld, Who};

/// `Entity.getRootVehicle` of the mob `id` riding `vehicle`: the bottom of
/// its stack of vehicles (itself when it rides nothing).
pub(super) fn root_vehicle(w: &MobWorld, id: u32, vehicle: Option<u32>) -> u32 {
    let (mut root, mut next) = (id, vehicle);
    // A stack of riders is short; the bound only guards a broken loop.
    for _ in 0..16 {
        let Some(v) = next else { break };
        root = v;
        next = w.seen(v).and_then(|s| s.vehicle);
    }
    root
}

/// `HORIZON_MAX_ENTITY_COLLISIONS` (the server's `[entities] max_collisions`): the most other mobs a
/// mob pushes in a tick, the pushes it receives counted too. 0, or unset, is vanilla: no cap.
pub fn max_collisions() -> u32 {
    #[cfg(test)]
    if let Some(cap) = TEST_CAP.with(std::cell::Cell::get) {
        return cap;
    }
    static CAP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("HORIZON_MAX_ENTITY_COLLISIONS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    })
}
#[cfg(test)]
thread_local!(static TEST_CAP: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) });
/// Runs `f` with the cap at `cap` on this thread.
#[cfg(test)]
pub(crate) fn with_cap<T>(cap: u32, f: impl FnOnce() -> T) -> T {
    TEST_CAP.with(|c| c.set(Some(cap)));
    let out = f();
    TEST_CAP.with(|c| c.set(None));
    out
}

/// What the cap says of the next mob a mob would push.
pub(super) enum Verdict {
    Push,
    /// The other mob has had its share this tick.
    Skip,
    /// This mob has pushed its share: the rest keep vanilla's order and are left out.
    Stop,
}
/// One mob's push loop under the cap: the collisions it has this tick (the pushes it took from
/// mobs ticked before it, which the snapshot counts, and the ones it gives). Only mobs pushing
/// mobs are capped: the player and anything else pass.
pub(super) struct Cap {
    cap: u32,
    count: u32,
}
impl Cap {
    pub fn new(m: &Mob, w: &MobWorld) -> Self {
        let cap = max_collisions();
        let count = if cap > 0 {
            w.seen(m.id).map_or(0, |s| s.collisions)
        } else {
            0
        };
        Self { cap, count }
    }
    pub fn check(&self, who: Who, w: &MobWorld) -> Verdict {
        match who {
            Who::Mob(_) if self.cap == 0 => Verdict::Push,
            Who::Mob(_) if self.count >= self.cap => Verdict::Stop,
            Who::Mob(id) if w.seen(id).is_some_and(|o| o.collisions >= self.cap) => Verdict::Skip,
            _ => Verdict::Push,
        }
    }
    /// A push of `who` was made: the mob loop adds it to both sides after the turn (`m.pushed`).
    pub fn record(&mut self, m: &mut Mob, who: Who) {
        if let (true, Who::Mob(id)) = (self.cap > 0, who) {
            self.count += 1;
            m.pushed.push(id);
        }
    }
}
