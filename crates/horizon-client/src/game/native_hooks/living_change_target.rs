use super::*;

/// The actual read host of a hook fired during a mob's own tick, either the
/// game or the tick's scoped view of the ticking mob and its snapshot.
pub(in crate::game) struct NativeHookHost<'a> {
    pub(in crate::game) host: &'a dyn mod_api::Host,
}

impl NativeHookHost<'_> {
    /// A mob is about to acquire or clear a target.
    pub(in crate::game) fn native_living_change_target(
        &self,
        call: mod_api::__private::LivingChangeTargetCall<'_>,
    ) {
        self.fire_living_change_target(call);
    }
    /// A mob is about to run `Mob.checkDespawn`.
    pub(in crate::game) fn native_mob_despawn(
        &self,
        dimension: &str,
        mob: MobRef,
        kind: &str,
        pos: [f64; 3],
        result: &mut Despawn,
    ) {
        self.fire_mob_despawn(dimension, mob, kind, pos, result);
    }
}
