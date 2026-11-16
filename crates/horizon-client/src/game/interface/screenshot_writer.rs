//! PNG encoding follows the owning-thread screenshot hook, never the frame thread.
use super::Game;
use horizon_client::gfx::{PendingScreenshot, ScreenshotSave};
use horizon_client::text::Text;
use std::path::PathBuf;

pub struct ScreenshotWrite {
    save: ScreenshotSave,
    message: Option<Text>,
    chat: bool,
}

impl ScreenshotWrite {
    pub(super) fn start(
        pending: PendingScreenshot,
        destination: Option<(PathBuf, String)>,
        message: Option<Text>,
        chat: bool,
    ) -> Self {
        Self {
            save: pending.save_background(destination),
            message,
            chat,
        }
    }
}

impl Game<'_> {
    pub(super) fn poll_screenshot_writes(&mut self) {
        let mut index = 0;
        while index < self.ui.screenshot_writes.len() {
            let Some((path, result)) = self.ui.screenshot_writes[index].save.try_complete() else {
                index += 1;
                continue;
            };
            let write = self.ui.screenshot_writes.remove(index);
            if write.chat {
                self.ui
                    .screenshot_messages
                    .retain(|path| *path != write.save.original);
                self.ui.screenshot_messages.insert(0, path);
                self.finish_screenshot(&result, write.message);
            } else if let Err(error) = result {
                log::warn!("Horizon: couldn't save the world icon: {error}");
            }
        }
    }
}
