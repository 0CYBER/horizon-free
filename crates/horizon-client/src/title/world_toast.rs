//! The system toast a world backup or a world that cannot be locked shows on the world list.
use super::*;

pub(super) enum WorldToast {
    Created {
        folder: String,
        bytes: u64,
    },
    Failed(String),
    /// `SystemToast.onWorldAccessFailure`: the folder of a world that couldn't be locked.
    AccessFailure(String),
}
impl WorldToast {
    pub(super) fn text(&self) -> (Text, Text) {
        match self {
            Self::Created { folder, bytes } => (
                Text::tr("selectWorld.edit.backupCreated", &[folder]),
                Text::tr(
                    "selectWorld.edit.backupSize",
                    &[&((*bytes as f64 / 1_048_576.0).ceil() as u64)],
                ),
            ),
            Self::Failed(error) => (
                Text::translate("selectWorld.edit.backupFailed", &[]),
                Text::literal(error),
            ),
            Self::AccessFailure(folder) => (
                Text::translate("selectWorld.access_failure", &[]),
                Text::literal(folder),
            ),
        }
    }
}
