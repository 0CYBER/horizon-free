//! Compiler guidance for declared internals.
pub(super) fn with_internals_hint(error: String) -> String {
    if error.contains("internals")
        && ["E0432", "E0433", "E0603"]
            .iter()
            .any(|code| error.contains(code))
    {
        format!("{error}\n{}", crate::i18n::tr("Game internals must be declared: run cargo horizon-mod internals sync in the mod's folder, then build again."))
    } else {
        error
    }
}
