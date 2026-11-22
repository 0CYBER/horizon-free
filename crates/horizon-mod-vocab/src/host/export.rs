//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// Fixed-clock video export methods reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ExportClientHost {
    /// Side: client.
    /// The active mod-started export, or the most recent mod-started result if one has ended.
    /// During `camera_pose` and the other frame hooks, `frame` is the index being prepared.
    /// Control-socket exports do not appear here.
    #[access(ClientSetup)]
    #[game_method(mod_export_status)]
    fn export_status(&self) -> Option<ExportStatus>;
    /// Side: client.
    /// Reserves the output directory and queues a fixed-clock capture on an offscreen client.
    /// The capture starts between callbacks. The nonzero request id is unique in this client
    /// session. Each accepted request reports its outcome through `export_ended` unless the
    /// game exits first. A capture in progress from any source prevents a new one.
    #[access(WorldOnly)]
    #[game_method(start_mod_export)]
    fn start_export(&mut self, request: &ExportRequest) -> Result<u32, ExportError>;
    /// Side: client.
    /// Requests a graceful stop of the active mod export. The current frame and accepted
    /// output drain before `export_ended`; a stop before frame zero still captures that frame.
    /// Repeated stops during a mod export's drain are accepted. A control-socket export alone
    /// returns `NotRunning`.
    #[access(WorldOnly)]
    fn stop_export(&mut self) -> Result<(), ExportError>;
}
