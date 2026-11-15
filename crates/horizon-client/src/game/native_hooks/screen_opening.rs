use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `Gui.setScreen` is about to replace the `current` screen with `new`
    /// (`ScreenEvent.Opening`; the screen `playing` is none: vanilla's null).
    /// Answer `ScreenOpening::Cancel` to keep the current one, or
    /// `ScreenOpening::ModScreen` to open the answering mod's own screen instead.
    /// A canceled prompt the server waits on (a server-pack prompt, the code
    /// of conduct) leaves it unanswered: a required pack then waits until the
    /// server gives up.
    pub(in crate::game) fn native_screen_opening(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        outcome: &mut Option<Answered<ScreenOpening>>,
    ) {
        self.fire_screen_opening(current, new, outcome);
    }
}
