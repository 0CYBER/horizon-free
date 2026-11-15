use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The stacks a mod adds to the creative tab `tab` while the tabs' contents are built
    /// (`BuildCreativeModeTabContentsEvent`, `CreativeModeTabs.buildAllTabContents`): push each
    /// stack to `items`, in order. `has_permissions` is the build's `hasPermissions`. Asked for
    /// every item tab, the first time the creative inventory opens after the resources load and
    /// whenever `has_permissions` changes. Client-side.
    pub(in crate::game) fn native_build_creative_mode_tab_contents(
        &mut self,
        tab: &str,
        has_permissions: bool,
        items: &mut Vec<ItemStack>,
    ) {
        self.fire_build_creative_mode_tab_contents(tab, has_permissions, items);
    }
}
