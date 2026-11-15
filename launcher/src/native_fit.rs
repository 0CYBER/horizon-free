//! Whether a native mod fits the Horizon the player plays, said before it is installed, so a
//! mod made for another Horizon is no surprise at build time. A mod made for another build is
//! built when everything it uses is unchanged in the instance's build (SDK design §10); the
//! build tests it, since that needs the mod's own files.
use crate::i18n::{tr, trf};
use crate::instances::Instance;
use crate::{backend, native_mods};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The Horizon the selected instance plays: the commit its game was compiled from.
#[derive(Clone, Debug, PartialEq)]
pub struct Plays {
    pub revision: String,
}

/// How a native mod fits [`Plays`].
#[derive(Clone, Debug, PartialEq)]
pub enum Fit {
    /// It was made for the Horizon the player plays.
    Fits,
    /// It was made for another build: the build tests whether it fits.
    OtherRevision { pin: String },
}

/// How a mod made for commit `pin` fits the Horizon played.
pub fn fit(pin: &str, plays: &Plays) -> Fit {
    if pin == plays.revision {
        Fit::Fits
    } else {
        Fit::OtherRevision { pin: pin.into() }
    }
}

/// What the player reads about `fit`.
pub fn describe(fit: &Fit, plays: &Plays) -> String {
    match fit {
        Fit::Fits => tr("Made for the Horizon you play").to_owned(),
        Fit::OtherRevision { pin } => {
            let pin = native_mods::sdk_cache::known_name(pin);
            // A version not known here has no name to give: say only that it differs.
            match native_mods::sdk_cache::name_of(&plays.revision) {
                None => trf(
                    "Made for {pin}, not the Horizon you play. Building checks whether it fits your game: it does when nothing it uses changed.",
                    &[("pin", &pin)],
                ),
                Some(yours) => trf(
                    "Made for {pin}; you play {yours}. Building checks whether it fits your game: it does when nothing it uses changed.",
                    &[("pin", &pin), ("yours", &yours)],
                ),
            }
        }
    }
}

/// [`Plays`] by the version an instance plays: a known commit is kept, a failure for [`RETRY`].
/// Only resolving a release or Nightly's build id reads its release (its signed SDK index), once
/// for all the search's results; after that the id is kept.
type CacheKey = (String, Option<String>);
type Known = HashMap<CacheKey, (Instant, Option<Plays>)>;
static PLAYS: Mutex<Option<Known>> = Mutex::new(None);
const RETRY: Duration = Duration::from_secs(60);

fn cached(cache: &Known, key: &CacheKey) -> Option<Option<Plays>> {
    cache
        .get(key)
        .filter(|(at, plays)| plays.is_some() || at.elapsed() < RETRY)
        .map(|(_, plays)| plays.clone())
}

/// The Horizon `instance` plays; None for a Java instance, when nothing is installed, or when
/// its release doesn't say its build id.
pub fn plays(instance: &Instance) -> Option<Plays> {
    if instance.java.is_some() {
        return None;
    }
    let version = instance.resolve_version(None).ok()?;
    // Read the build record before consulting the cache so a rebuild's answer follows it.
    let native_revision = native_mods::about(&version).map(|about| about.horizon);
    let key = (version.clone(), native_revision.clone());
    let known = cached(PLAYS.lock().unwrap().get_or_insert_default(), &key);
    if let Some(plays) = known {
        return plays;
    }
    // A native build is the commit it compiled; anything else, the commit a build would use.
    let revision = match native_revision {
        Some(revision) => Some(revision),
        None => native_mods::base_for(Some(&version), &backend::installed(), false)
            .and_then(|base| native_mods::revision_of(&base).ok()),
    };
    let plays = revision.map(|revision| Plays { revision });
    PLAYS
        .lock()
        .unwrap()
        .get_or_insert_default()
        .insert(key, (Instant::now(), plays.clone()));
    plays
}

/// The line the registry browser shows for a native mod's version pinning `pin`, in `instance`;
/// None when the game's revision can't be told. This only describes fit: it never builds or
/// changes the instance or a package pin.
pub fn line(instance: &Instance, pin: &str) -> Option<String> {
    let plays = plays(instance)?;
    Some(describe(&fit(pin, &plays), &plays))
}

#[cfg(test)]
mod tests {
    use super::*;

    const YOURS: &str = "1111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222";

    fn plays() -> Plays {
        Plays {
            revision: YOURS.into(),
        }
    }

    #[test]
    fn rebuilding_a_native_version_does_not_reuse_its_previous_compatibility() {
        let version = "native-example".to_string();
        let old_key = (version.clone(), Some(YOURS.to_string()));
        let rebuilt_key = (version.clone(), Some(OTHER.to_string()));
        let unbuilt_key = (version, None);
        let old = plays();
        let mut cache = Known::new();
        cache.insert(old_key.clone(), (Instant::now(), Some(old.clone())));
        assert_eq!(cached(&cache, &old_key), Some(Some(old)));
        assert_eq!(cached(&cache, &rebuilt_key), None);
        assert_eq!(cached(&cache, &unbuilt_key), None);
    }

    #[test]
    fn a_mod_made_for_another_build_is_left_to_the_build_to_test() {
        let p = plays();
        assert_eq!(fit(YOURS, &p), Fit::Fits);
        assert_eq!(fit(OTHER, &p), Fit::OtherRevision { pin: OTHER.into() });
        let said = describe(&fit(OTHER, &p), &p);
        assert!(
            said.starts_with("Made for another Horizon version, not the Horizon you play."),
            "{said}"
        );
    }
}
