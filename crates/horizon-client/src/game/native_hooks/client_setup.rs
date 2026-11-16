use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `FMLClientSetupEvent`: the client started. Fires once per game process,
    /// after the first client load has applied and before the title screen's first frame;
    /// never again on a reload (`client_resources_reloaded`) or a new world.
    pub(in crate::game) fn native_client_setup(&mut self) {
        self.fire_client_setup();
    }
}
