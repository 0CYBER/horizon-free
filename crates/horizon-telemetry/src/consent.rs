//! The player's answer, kept in `telemetry/consent.json` in the game folder: one switch per
//! kind of data, all off until they turn one on, with the consent version and when they answered.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CONSENT_VERSION, Kind};

/// What the player allows, kind by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    /// Performance summaries: frame-time percentiles and buckets, tick times, memory, startup.
    pub performance: bool,
    /// Crash reports, without paths, names or addresses.
    pub crashes: bool,
    /// Bug reports the player writes and sends themselves.
    pub bugs: bool,
    /// Benchmark reports from the launcher and the game.
    pub benchmarks: bool,
}

impl Choices {
    pub const NONE: Self = Self {
        performance: false,
        crashes: false,
        bugs: false,
        benchmarks: false,
    };
    pub const ALL: Self = Self {
        performance: true,
        crashes: true,
        bugs: true,
        benchmarks: true,
    };

    pub fn allows(self, kind: Kind) -> bool {
        match kind {
            Kind::Perf => self.performance,
            Kind::Crash => self.crashes,
            Kind::Bug => self.bugs,
            Kind::Benchmark => self.benchmarks,
        }
    }

    pub fn set(&mut self, kind: Kind, on: bool) {
        match kind {
            Kind::Perf => self.performance = on,
            Kind::Crash => self.crashes = on,
            Kind::Bug => self.bugs = on,
            Kind::Benchmark => self.benchmarks = on,
        }
    }

    pub fn any(self) -> bool {
        Kind::ALL.into_iter().any(|k| self.allows(k))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Consent {
    /// The [`CONSENT_VERSION`] the player answered; 0 if they never did.
    pub version: u32,
    /// When they answered, in seconds since 1970 (UTC).
    pub answered: u64,
    pub choices: Choices,
    /// Random, made at the first yes and replaced on reset; it ties a player's items together
    /// so they can be deleted, and nothing else. Empty until then.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub install_id: String,
}

impl Consent {
    pub fn path(home: &Path) -> PathBuf {
        crate::dir(home).join("consent.json")
    }

    /// The saved answer; a missing or unreadable file is "never asked", which allows nothing.
    pub fn load(home: &Path) -> Self {
        std::fs::read(Self::path(home))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, home: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::write_atomic(&Self::path(home), &json)
    }

    /// Whether the launcher should ask: never answered, or what is collected has changed since.
    pub fn needs_asking(&self) -> bool {
        self.version < CONSENT_VERSION
    }

    /// Whether items of `kind` may be kept and sent. Never before an answer to the current
    /// question (a newer launcher's answer covers an older game's).
    pub fn allows(&self, kind: Kind) -> bool {
        !self.needs_asking() && !self.install_id.is_empty() && self.choices.allows(kind)
    }

    /// Records the player's answer now. The install ID is made at the first yes and kept when
    /// they turn everything off, so "Delete my data" can still name it.
    pub fn answer(&mut self, choices: Choices) {
        self.version = CONSENT_VERSION;
        self.answered = crate::now_secs();
        self.choices = choices;
        if choices.any() && self.install_id.is_empty() {
            self.install_id = new_install_id();
        }
    }

    /// A new install ID (after "Delete my data"): nothing sent later links to what came before.
    pub fn reset_install_id(&mut self) {
        self.install_id = if self.choices.any() {
            new_install_id()
        } else {
            String::new()
        };
    }
}

/// 128 random bits in hex.
pub fn new_install_id() -> String {
    crate::random_hex(16)
}

/// Whether `id` looks like one of ours: 32 lowercase hex digits.
pub fn is_install_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_allowed_before_the_player_answers() {
        let home = crate::test_home::new("consent-default");
        let consent = Consent::load(&home);
        assert!(consent.needs_asking());
        assert!(Kind::ALL.iter().all(|&k| !consent.allows(k)));
        // A broken file is the same as none.
        std::fs::create_dir_all(crate::dir(&home)).unwrap();
        std::fs::write(Consent::path(&home), b"{not json").unwrap();
        assert_eq!(Consent::load(&home), Consent::default());
    }

    #[test]
    fn an_answer_allows_only_what_was_chosen_and_is_saved() {
        let home = crate::test_home::new("consent-answer");
        let mut consent = Consent::load(&home);
        consent.answer(Choices {
            crashes: true,
            ..Choices::NONE
        });
        consent.save(&home).unwrap();
        let back = Consent::load(&home);
        assert_eq!(back, consent);
        assert!(!back.needs_asking());
        assert!(back.allows(Kind::Crash));
        assert!(!back.allows(Kind::Perf) && !back.allows(Kind::Bug));
        assert!(!back.allows(Kind::Benchmark));
        assert!(is_install_id(&back.install_id));
        assert!(back.answered > 1_700_000_000);
    }

    #[test]
    fn an_older_answer_allows_nothing_until_asked_again() {
        let mut consent = Consent::default();
        consent.answer(Choices::ALL);
        consent.version = CONSENT_VERSION - 1;
        assert!(consent.needs_asking());
        assert!(Kind::ALL.iter().all(|&k| !consent.allows(k)));
    }

    #[test]
    fn no_id_is_made_for_a_no_and_a_reset_changes_it() {
        let mut consent = Consent::default();
        consent.answer(Choices::NONE);
        assert!(consent.install_id.is_empty());
        consent.answer(Choices::ALL);
        let first = consent.install_id.clone();
        consent.answer(Choices::NONE);
        assert_eq!(consent.install_id, first, "kept so the data can be deleted");
        consent.answer(Choices::ALL);
        consent.reset_install_id();
        assert!(is_install_id(&consent.install_id) && consent.install_id != first);
    }
}
