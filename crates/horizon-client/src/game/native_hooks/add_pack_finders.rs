use super::*;

impl Game<'_> {
    /// Register sources when a resource or data pack repository is discovered.
    pub(in crate::game) fn native_add_pack_finders(
        &mut self,
        packs: &mut dyn mod_api::PackFinders,
    ) {
        self.fire_add_pack_finders(packs);
    }
}
