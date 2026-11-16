use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An F2 frame is read back; mods may change its image, cancel, rename it or replace its line.
    pub(in crate::game) fn native_screenshot(
        &mut self,
        image: &mut ScreenshotImage,
        file_name: &mut String,
        message: &mut Option<Text>,
        cancel: &mut bool,
    ) {
        self.fire_screenshot(image, file_name, message, cancel);
    }
}
