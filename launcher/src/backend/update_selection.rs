//! Compatibility selection written for launchers that predate per-instance pins.
use super::*;

/// Whether a version installed as an update becomes the selected one: when the player had none,
/// was on the newest installed Nightly, or on the newest installed release (so a player of the
/// release channel moves
/// to the Nightly at the first check, docs/release-flow.md). An older release picked by hand
/// stays, and so does an imported build (`build_import`) or a snapshot (installed by its tag
/// alone); an instance pinned to a version keeps it either way.
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) if crate::nightly::date_of(s).is_some() => installed_before
            .iter()
            .find(|t| crate::nightly::date_of(t).is_some())
            .is_none_or(|newest| newest == s),
        Some(s)
            if crate::build_import::is_imported(s)
                || crate::developer_install::uses_developer(s)
                || horizon_release_sign::Channel::of(s)
                    == Some(horizon_release_sign::Channel::Snapshot) =>
        {
            false
        }
        Some(s) => installed_before
            .iter()
            .find(|t| is_game_tag(t))
            .is_none_or(|newest| newest == s),
    }
}

/// Why a version was installed, for [`selection_after_install`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallCause {
    /// Installed as the launcher's update.
    pub update: bool,
    /// Installed because an instance plays it: `Some(true)` when an instance on the Nightly
    /// channel is pinned to it, `Some(false)` for any other (a Stable instance, Home's Stable
    /// update). None when a person installed it from Versions or the command line.
    pub instance: Option<bool>,
}

/// Whether `tag`, just installed for `cause`, becomes the selected version, and if so whether
/// the launcher then follows it: `Some(true)` selects and follows it (the caller saves it in
/// `Settings::following`), `Some(false)` selects it as a person's pick, None keeps the selection.
///
/// Older launchers follow the newest only while `selected` is the build the launcher chose
/// (`following`). An update moves a follower, and otherwise keeps [`select_after_update`]'s rule.
/// A newer Nightly installed for a Nightly instance pinned to it also moves a follower. A Stable
/// build installed for an instance keeps an existing selection.
/// `installed` is every installed version, newest first, `tag` among them, and `before` the
/// same without `tag`.
pub fn selection_after_install(
    selected: Option<&str>,
    following: Option<&str>,
    tag: &str,
    before: &[String],
    installed: &[String],
    cause: InstallCause,
) -> Option<bool> {
    let follower = selected.is_some() && selected == following;
    let newest = installed.first().map(String::as_str) == Some(tag);
    let has_selection = selected.is_some_and(|s| installed.iter().any(|t| t == s));
    let by_update = !cause.update || select_after_update(selected, before) || (follower && newest);
    let for_instance = match cause.instance {
        None => true,
        Some(_) if !has_selection => true,
        Some(nightly_pin) => {
            nightly_pin
                && follower
                && newest
                && selected != Some(tag)
                && crate::nightly::date_of(tag).is_some()
        }
    };
    (by_update && for_instance).then_some(cause.update || cause.instance.is_some())
}
