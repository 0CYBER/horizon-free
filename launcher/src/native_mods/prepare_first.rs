//! A native build started from the launcher's window whose base version isn't prepared runs the
//! window's own Prepare for that version first, then builds (#18659). The command line keeps
//! its hint to run `horizon-launcher prepare` ([`crate::backend::prepare_hint`]).

use crate::backend::NotReady;
use crate::i18n::trf;
use crate::instances::{Channel, Instance};

/// What a waiting build was started with and depends on. Taken when it starts waiting and
/// again whenever it looks again: any difference drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub instance: String,
    /// The instance's saved pin and channel, raw: a pin moved between two releases that aren't
    /// installed resolves to the same nothing.
    pub pin: Option<String>,
    pub channel: Channel,
    /// The version it plays, as the window resolves it.
    pub plays: Option<String>,
    pub base: String,
    pub back_up: bool,
    /// Whether native mods are allowed.
    pub consent: bool,
}

impl Snapshot {
    pub fn of(
        instance: &Instance,
        plays: Option<String>,
        base: String,
        back_up: bool,
        consent: bool,
    ) -> Self {
        Self {
            instance: instance.id.clone(),
            pin: instance.version.clone(),
            channel: instance.channel,
            plays,
            base,
            back_up,
            consent,
        }
    }
}

/// A window build waiting for its base's preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub snapshot: Snapshot,
    /// The version to prepare: the base, or a native base's own base ([`crate::backend::prepared_in`]).
    pub needs: String,
    /// The preparation running now was started by this build (its Stop stops it too).
    pub started: bool,
}

/// What the window is doing when a waiting build looks again.
pub struct Now<'a> {
    /// [`Snapshot::of`] the build's instance now, with its base and back-up choice; `None` once
    /// the instance is gone.
    pub snapshot: Option<Snapshot>,
    /// A preparation runs. The window runs one at a time, so this build's can't start meanwhile.
    pub preparing: bool,
    /// The version an install is writing, if one runs.
    pub installing: Option<&'a str>,
    /// The preparation that just ended: the version it prepared and its error as the window
    /// shows it.
    pub ended: Option<(&'a str, Option<&'a str>)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Build,
    /// Start the window's Prepare for [`Waiting::needs`], then look again when it ends.
    Prepare,
    /// A preparation, or an install of the version to prepare, runs: look again when it ends.
    Wait,
    /// Drop the build, saying this.
    Stop(String),
}

/// What a window build does next, from when it is pressed until it builds or stops. `ready` is
/// [`crate::backend::ready`]. A ready base builds at once; only an unready one looks at what
/// runs, and only the preparation of the version it needs can fail it. A base that must be
/// reinstalled builds, so the build says so as it always did.
pub fn next(waiting: &Waiting, now: &Now, ready: impl Fn(&str) -> Result<(), NotReady>) -> Next {
    if now.snapshot.as_ref() != Some(&waiting.snapshot) {
        return Next::Stop(dropped(&waiting.needs));
    }
    match ready(&waiting.snapshot.base) {
        Ok(()) | Err(NotReady::Reinstall(_)) => return Next::Build,
        Err(NotReady::Unprepared(_)) => {}
    }
    if let Some((tag, Some(error))) = now.ended {
        if tag == waiting.needs {
            return Next::Stop(error.to_owned());
        }
    }
    let base = &waiting.snapshot.base;
    // An install's publish replaces the folder a preparation would write into.
    if now.preparing
        || now
            .installing
            .is_some_and(|t| t == waiting.needs || t == base)
    {
        return Next::Wait;
    }
    Next::Prepare
}

/// What the card says when a waiting build is dropped because what it was started with changed.
pub fn dropped(needs: &str) -> String {
    trf(
        "The build waiting for {version} to be prepared stopped: its instance or the permission for native mods changed meanwhile.",
        &[("version", needs)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn snapshot(pin: Option<&str>, plays: Option<&str>, consent: bool) -> Snapshot {
        Snapshot {
            instance: "a".into(),
            pin: pin.map(Into::into),
            channel: Channel::default(),
            plays: plays.map(Into::into),
            base: "v1".into(),
            back_up: true,
            consent,
        }
    }

    fn waiting() -> Waiting {
        Waiting {
            snapshot: snapshot(Some("v1"), Some("v1"), true),
            needs: "v1".into(),
            started: false,
        }
    }

    fn now<'a>() -> Now<'a> {
        Now {
            snapshot: Some(snapshot(Some("v1"), Some("v1"), true)),
            preparing: false,
            installing: None,
            ended: None,
        }
    }

    /// `v1` as the disk would answer, prepared once `prepared` is set.
    fn disk(prepared: &Cell<bool>) -> impl Fn(&str) -> Result<(), NotReady> + '_ {
        |tag| match prepared.get() {
            true => Ok(()),
            false => Err(NotReady::Unprepared(tag.into())),
        }
    }

    #[test]
    fn a_window_build_prepares_an_unprepared_base_first_then_builds() {
        let prepared = Cell::new(false);
        let w = waiting();
        assert_eq!(next(&w, &now(), disk(&prepared)), Next::Prepare);
        // Its Prepare runs: nothing starts twice.
        let running = Now {
            preparing: true,
            ..now()
        };
        assert_eq!(next(&w, &running, disk(&prepared)), Next::Wait);
        // It ended well and left v1 prepared: the build goes on.
        prepared.set(true);
        let ended = Now {
            ended: Some(("v1", None)),
            ..now()
        };
        assert_eq!(next(&w, &ended, disk(&prepared)), Next::Build);
        // A base to reinstall is no preparation: the build says so itself.
        let reinstall = |_: &str| Err(NotReady::Reinstall("reinstall v1".into()));
        assert_eq!(next(&w, &now(), reinstall), Next::Build);
    }

    #[test]
    fn a_ready_base_builds_whatever_else_runs() {
        let prepared = Cell::new(true);
        let busy = Now {
            preparing: true,
            installing: Some("v1"),
            ended: Some(("v1", Some("an older failure"))),
            ..now()
        };
        assert_eq!(next(&waiting(), &busy, disk(&prepared)), Next::Build);
    }

    #[test]
    fn a_failed_preparation_of_its_base_stops_the_build_with_its_message() {
        let prepared = Cell::new(false);
        let error = "Minecraft 26.3 was not found. Sign in with the account that owns it.";
        let ended = Now {
            ended: Some(("v1", Some(error))),
            ..now()
        };
        assert_eq!(
            next(&waiting(), &ended, disk(&prepared)),
            Next::Stop(error.into())
        );
    }

    #[test]
    fn another_versions_failed_preparation_does_not_stop_the_build() {
        let prepared = Cell::new(false);
        let w = waiting();
        // Prepare & play for v2 failed while this build waited: it prepares v1 itself.
        let ended = Now {
            ended: Some(("v2", Some("v2 failed"))),
            ..now()
        };
        assert_eq!(next(&w, &ended, disk(&prepared)), Next::Prepare);
    }

    #[test]
    fn a_changed_instance_or_permission_drops_the_build() {
        let prepared = Cell::new(true);
        let w = waiting();
        let changed = [
            // Removed.
            None,
            // Plays another version.
            Some(snapshot(Some("v2"), Some("v2"), true)),
            // Native mods no longer allowed.
            Some(snapshot(Some("v1"), Some("v1"), false)),
        ];
        for snapshot in changed {
            let now = Now { snapshot, ..now() };
            assert_eq!(next(&w, &now, disk(&prepared)), Next::Stop(dropped("v1")));
        }
    }

    #[test]
    fn a_repin_between_two_uninstalled_releases_drops_the_build() {
        let prepared = Cell::new(true);
        // Pinned to v8, not installed: the window resolves it to nothing.
        let w = Waiting {
            snapshot: snapshot(Some("v8"), None, true),
            ..waiting()
        };
        let same = Now {
            snapshot: Some(snapshot(Some("v8"), None, true)),
            ..now()
        };
        assert_eq!(next(&w, &same, disk(&prepared)), Next::Build);
        // Pinned to v9 meanwhile, not installed either: it resolves to nothing too.
        let repinned = Now {
            snapshot: Some(snapshot(Some("v9"), None, true)),
            ..now()
        };
        assert_eq!(
            next(&w, &repinned, disk(&prepared)),
            Next::Stop(dropped("v1"))
        );
    }

    #[test]
    fn an_install_of_the_version_to_prepare_is_waited_for() {
        let prepared = Cell::new(false);
        let w = waiting();
        let installing = Now {
            installing: Some("v1"),
            ..now()
        };
        assert_eq!(next(&w, &installing, disk(&prepared)), Next::Wait);
        let other = Now {
            installing: Some("v2"),
            ..now()
        };
        assert_eq!(next(&w, &other, disk(&prepared)), Next::Prepare);
    }

    #[test]
    fn each_caller_reads_its_own_words() {
        let v = crate::backend::game_version("v0");
        let failed = || crate::native_mods::Failed::NotPrepared("v0".into());
        // The command line keeps main's words, byte for byte.
        assert_eq!(
            failed().said(crate::native_mods::Caller::CommandLine),
            format!(
                "v0 is not prepared yet. Prepare it first: `horizon-launcher prepare --version v0` uses your own Minecraft {v}, or downloads it from Mojang for the signed-in account that owns it."
            )
        );
        assert_eq!(String::from(failed()), crate::backend::prepare_hint("v0"));
        // The window never names a command.
        let window = failed().said(crate::native_mods::Caller::Window);
        assert!(!window.contains("horizon-launcher"), "{window}");
    }
}
