use super::*;

impl Game<'_> {
    /// Modify the blink passed to `Hud.extractHearts`, not the stored health history.
    pub(in crate::game) fn native_heart_blink(&mut self, blink: &mut bool) {
        self.fire_heart_blink(blink);
    }
}
