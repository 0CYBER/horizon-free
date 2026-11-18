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
                // A shot with an HDR copy gets the line linking both here, as `finish_screenshot`'s
                // message: its signature is a mod point's.
                let message = write.message.or_else(|| {
                    let hdr = result.as_ref().ok()?.hdr_path.as_deref()?;
                    Some(super::chat::screenshot_saved_line(&path, Some(hdr)))
                });
                self.ui.screenshot_messages.insert(0, path);
                let result = result.map(|saved| saved.size);
                self.finish_screenshot(&result, message);
            } else if let Err(error) = result {
                log::warn!("Horizon: couldn't save the world icon: {error}");
            }
        }
    }
}
