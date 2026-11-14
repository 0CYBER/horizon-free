//! A client mod leaving the world, NeoForge's `IClientCommonPacketListenerExtension.disconnect`.
//! The server half is `native_player_disconnect`.
use super::Game;
use horizon_client::{Result, text::Text};
use mod_api::{DisconnectError, WorldEpoch};

/// A client mod's request to leave, carried out at the end of the frame.
pub(super) struct LeaveRequest {
    /// The world it was made in: a request made while that world closes is dropped.
    epoch: WorldEpoch,
    reason: Option<Text>,
}

impl Game<'_> {
    /// Asks to leave the joined server or world at the end of this frame.
    /// Unavailable with no world joined, or while the world is already closing.
    pub(super) fn disconnect(
        &mut self,
        reason: Option<Text>,
    ) -> std::result::Result<(), DisconnectError> {
        let epoch = self.native_lifecycle.epoch();
        let (Some(epoch), Some(_), None) = (epoch, self.remote.as_ref(), &self.pending_save) else {
            return Err(DisconnectError::Unavailable);
        };
        self.native_net.leave = Some(LeaveRequest { epoch, reason });
        Ok(())
    }

    /// What follows the frame's render: the cursor icon, then a mod's `disconnect` (below).
    pub(in crate::game) fn native_frame_end(&mut self) -> Result<()> {
        horizon_client::spans::time("pr_cursor", || self.apply_cursor_icon());
        self.native_leave_if_asked()
    }

    /// The end of the frame: a mod's `disconnect` leaves now, outside every hook. With a
    /// reason, the disconnect screen shows it; with none, the pause menu's Disconnect runs. A
    /// request from a world that has closed since is dropped.
    pub(in crate::game) fn native_leave_if_asked(&mut self) -> Result<()> {
        let Some(request) = self.native_net.leave.take() else {
            return Ok(());
        };
        if self.remote.is_none() || self.native_lifecycle.epoch() != Some(request.epoch) {
            return Ok(());
        }
        let left = match request.reason {
            Some(reason) => self.leave_server(Some(reason.plain())),
            None => self.close_world(),
        };
        // A hook the leaving fired (a disconnect or unload listener) may have asked again: that
        // request belongs to the world just closed.
        self.native_net.leave = None;
        left
    }
}
