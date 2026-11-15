//! Admit a complete candidate before changing the live functions, tables or pack selection.
use super::{PackStack, registry_graph};
use mod_api::__private::reload::ReloadStageError;

/// The lines vanilla logs for a refused registry: an error when a reference does not bind, a
/// warning for the validation problems after it.
fn report_failure(error: &registry_graph::Error) {
    for line in &error.lines {
        if error.binding {
            log::error!("{line}");
        } else {
            log::warn!("{line}");
        }
    }
}

impl PackStack {
    /// `ReloadableServerRegistries.reload`: bind and validate the selected registries before the
    /// candidate is published (`/reload`, `/datapack`, a world's first open). A refusal leaves the
    /// previous library, content tables and timer queue live. The resolved predicate tags are kept
    /// for `admit_store`, which hands them to the predicate evaluator.
    pub(super) fn validate(&self) -> Result<&registry_graph::BoundTags, ReloadStageError> {
        if let Some(tags) = self.tags.get() {
            return Ok(tags);
        }
        let packs = self
            .builtin
            .iter()
            .chain(self.enabled_packs())
            .collect::<Vec<_>>();
        match registry_graph::check(&packs) {
            Ok(tags) => Ok(self.tags.get_or_init(|| tags)),
            Err(error) => {
                report_failure(&error);
                Err(ReloadStageError::DataSource(error.to_string()))
            }
        }
    }
}
