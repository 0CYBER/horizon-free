//! Convert collected Part refusals into the normal startup error.
pub(in crate::game) fn admit_parts(mods: &[mod_api::ModEntry]) -> crate::Result<()> {
    super::api_values::install_parts(mods)
        .map_err(|refused| format!("Mod parts refused:\n{}", refused.join("\n")).into())
}
