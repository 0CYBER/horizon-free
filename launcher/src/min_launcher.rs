//! A minimum launcher version (#18631). `min_launcher` in links.json on main names the oldest
//! launcher that may stay in use, for a release known to be broken. Every launcher already
//! downloads and checks a newer launcher as soon as an update check finds one; one older than
//! `min_launcher` that replaces itself then installs it and restarts without waiting for Update.
//! It waits while a game it started runs or a version installs, and then it installs when the
//! window closes (`update_handoff`). It tries once per start: a failed update says why, as any
//! other does, and the launcher goes on; the launcher the restart starts never restarts again for
//! the minimum, whatever it finds (`update_handoff::MIN_RESTARTED_ENV`).
use crate::backend;

/// The version `min_launcher` names: a plain `0.2.80`, or the tag `launcher@0.2.80`. None for
/// anything else: an empty value, a prerelease, a typo.
pub fn version_in(text: &str) -> Option<String> {
    let text = text.trim();
    let version = text
        .strip_prefix(backend::LAUNCHER_TAG_PREFIX)
        .unwrap_or(text);
    let parts: Vec<&str> = version.split('.').collect();
    (parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())))
    .then(|| version.to_owned())
}

/// Whether launcher `version` is older than `min`, a prerelease of `min` included.
pub fn below(version: &str, min: &str) -> bool {
    backend::compare_launcher_versions(version, min).is_lt()
}

/// What the window is doing that a restart would cut short.
#[derive(Clone, Copy, Debug, Default)]
pub struct Activity {
    /// A game it started runs, or is starting.
    pub game: bool,
    /// A version is being prepared.
    pub preparing: bool,
    /// A game version is being installed, reinstalled or removed.
    pub installing: bool,
}

impl Activity {
    pub fn busy(self) -> bool {
        self.game || self.preparing || self.installing
    }
}

/// What the window knows when it decides.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    /// Not a development build or `HORIZON_LAUNCHER_NO_SELF_UPDATE`.
    pub self_update_allowed: bool,
    /// A package manager owns this copy (`backend::package_managed_launcher`).
    pub package_managed: bool,
    /// An update is downloaded and checked.
    pub ready: bool,
    pub activity: Activity,
    /// This start already tried, or it is the launcher an earlier one restarted into.
    pub tried: bool,
}

/// Whether launcher `version` installs its staged update now and restarts.
pub fn installs_now(version: &str, min: Option<&str>, now: Now) -> bool {
    now.self_update_allowed
        && !now.package_managed
        && now.ready
        && !now.activity.busy()
        && !now.tried
        && min.is_some_and(|min| below(version, min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_is_a_plain_version_or_a_launcher_tag() {
        assert_eq!(version_in("0.2.80").as_deref(), Some("0.2.80"));
        assert_eq!(version_in(" launcher@0.2.80 ").as_deref(), Some("0.2.80"));
        for malformed in [
            "",
            "latest",
            "v2026.10.1",
            "0.2",
            "0.2.80.1",
            "0.2.80-rc.1",
            "0.2.x",
            "0..80",
            "launcher-v0.2.80",
        ] {
            assert_eq!(version_in(malformed), None, "{malformed}");
        }
    }

    #[test]
    fn versions_compare_as_releases() {
        assert!(below("0.2.79", "0.2.80"));
        assert!(below("0.2.9", "0.2.10"));
        assert!(below("0.2.80-rc.1", "0.2.80"));
        assert!(!below("0.2.80", "0.2.80"));
        assert!(!below("0.2.81", "0.2.80"));
        assert!(!below("0.3.0", "0.2.80"));
    }

    const READY: Now = Now {
        self_update_allowed: true,
        package_managed: false,
        ready: true,
        activity: Activity {
            game: false,
            preparing: false,
            installing: false,
        },
        tried: false,
    };

    #[test]
    fn only_a_launcher_below_the_minimum_with_nothing_running_installs_now() {
        let min = Some("0.2.80");
        assert!(installs_now("0.2.79", min, READY));
        assert!(installs_now("0.2.80-rc.1", min, READY));
        // At or above the minimum, or no minimum: Update or closing the window, as always.
        assert!(!installs_now("0.2.80", min, READY));
        assert!(!installs_now("0.2.79", None, READY));
        // A development build or HORIZON_LAUNCHER_NO_SELF_UPDATE never replaces itself.
        let not_allowed = Now {
            self_update_allowed: false,
            ..READY
        };
        assert!(!installs_now("0.2.79", min, not_allowed));
        // Nothing downloaded and checked yet (or that failed): nothing to install.
        let not_ready = Now {
            ready: false,
            ..READY
        };
        assert!(!installs_now("0.2.79", min, not_ready));
        // A game runs: it goes in when the window closes.
        let game = Now {
            activity: Activity {
                game: true,
                ..Activity::default()
            },
            ..READY
        };
        assert!(!installs_now("0.2.79", min, game));
    }

    /// A distribution's package (`/usr/bin/horizon-launcher`) is updated by its package manager.
    #[test]
    fn a_package_owned_launcher_does_not_restart_for_the_minimum() {
        let package = Now {
            package_managed: true,
            ..READY
        };
        assert!(!installs_now("0.2.79", Some("0.2.80"), package));
    }

    /// Exiting while a game version installs would leave it half replaced.
    #[test]
    fn an_install_in_progress_holds_the_restart() {
        let installing = Activity {
            installing: true,
            ..Activity::default()
        };
        assert!(installing.busy());
        let now = Now {
            activity: installing,
            ..READY
        };
        assert!(!installs_now("0.2.79", Some("0.2.80"), now));
    }

    /// Once per start: not again after a try this start (a failed handoff, the next check), and
    /// never in the launcher a restart for the minimum started, even if it is still below it.
    #[test]
    fn the_restart_for_the_minimum_happens_once() {
        let tried = Now {
            tried: true,
            ..READY
        };
        assert!(!installs_now("0.2.80-rc.1", Some("0.2.80"), tried));
        assert!(!installs_now("0.2.79", Some("0.2.80"), tried));
    }
}
