//! Presentation data for the client’s saving-world frames.

/// The operation whose save is currently being presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ClosingSaveOperation {
    /// Close the local world without quitting the client.
    LeaveWorld,
    /// Save and close the client.
    QuitClient,
    /// An explicit save that does not close the world, where it uses the saving screen.
    ExplicitSave,
}

/// The save work the client is waiting for on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ClosingSaveStage {
    /// Wait for outstanding terrain generation.
    Generation,
    /// Prepare and queue chunk snapshots.
    Chunks,
    /// Prepare entity snapshots and save world metadata.
    Entities,
    /// Wait for durable writes to finish.
    Writes,
    /// Wait for the integrated server to save and stop; it does not report progress here.
    Server,
}
