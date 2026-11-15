use super::{Game, ItemStack};
use horizon_protocol::item::Stack;

impl Game<'_> {
    /// `DisplayInfo.fromNetwork`: convert the template once, not while drawing it.
    pub(super) fn remote_advancement_icon(&self, mut icon: Stack) -> ItemStack {
        let remote = self.remote.as_ref().expect("remote advancement");
        if !horizon_protocol::item::custom::permitted(&icon, remote.registries.custom_components)
            .unwrap_or(false)
        {
            // Keep the advancement, item and count. Refusing an unnegotiated custom
            // component (including nested components), or a malformed nested template,
            // strips the ENTIRE patch: all additions (including damage and glint), all
            // removals, and cached wire bytes. No refused component reaches rendering.
            log::warn!("Drawing advancement icon without its refused component patch");
            icon.added.clear();
            icon.removed.clear();
            icon.wire = Default::default();
        }
        horizon_client::item_sync::from_network(&icon, &remote.builtin, &remote.registries)
    }
}

#[cfg(test)]
mod tests;
