//! Release notes parsed once per release list. A Nightly's notes run to some 1,500 items: parsed
//! and laid out whole on every redraw, they took Home's patch log 7 ms a frame, more than the 6
//! a 165 Hz display allows, so the patch log and Patch notes keep them here and draw only the
//! rows in view (`cull.rs`).
use crate::cull::Cull;
use crate::glass_home_tabs::PatchLog;
use crate::markdown::Notes;
use horizon_launcher::backend::Release;
use horizon_launcher::i18n::{self, Language};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A patch log row is a line or two of 21 px text, 6 px apart.
const PATCH_ROW: f32 = 36.;
/// A block of notes is mostly one item of 21.7 px lines.
const NOTES_BLOCK: f32 = 32.;

/// The release played (None for a Java instance) and the language the log's line is in.
type PatchKey = (Option<String>, Language);

pub(super) struct NotesCache {
    patch: RefCell<Option<(PatchKey, Rc<PatchLog>)>>,
    patch_rows: Cull,
    /// Each opened release's notes and their rows, by tag.
    releases: RefCell<HashMap<String, (Rc<Notes>, Cull)>>,
}

impl Default for NotesCache {
    fn default() -> Self {
        NotesCache {
            patch: RefCell::default(),
            patch_rows: Cull::estimated(PATCH_ROW),
            releases: RefCell::default(),
        }
    }
}

impl NotesCache {
    /// The release list changed: what was parsed from the old one goes.
    pub(super) fn clear(&mut self) {
        *self = NotesCache::default();
    }

    /// The patch log for `played`, made by `make` when the release list, the release played or
    /// the language changed; with the rows it draws.
    pub(super) fn patch_log(
        &self,
        played: Option<&str>,
        make: impl FnOnce() -> PatchLog,
    ) -> (Rc<PatchLog>, Cull) {
        let key = (played.map(str::to_owned), i18n::current());
        let mut patch = self.patch.borrow_mut();
        let log = match &*patch {
            Some((made_for, log)) if *made_for == key => log.clone(),
            _ => {
                let log = Rc::new(make());
                self.patch_rows.reset();
                *patch = Some((key, log.clone()));
                log
            }
        };
        (log, self.patch_rows.clone())
    }

    /// `release`'s notes, parsed the first time they open.
    pub(super) fn notes(&self, release: &Release) -> (Rc<Notes>, Cull) {
        let mut releases = self.releases.borrow_mut();
        let (notes, rows) = releases.entry(release.tag.clone()).or_insert_with(|| {
            (
                Rc::new(Notes::parse(&release.body)),
                Cull::estimated(NOTES_BLOCK),
            )
        });
        (notes.clone(), rows.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn the_patch_log_is_made_once_per_release_list_and_played_release() {
        let made = Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            PatchLog::empty()
        };
        let mut cache = NotesCache::default();
        cache.patch_log(Some("nightly-2026-11-13-9f42064"), make);
        cache.patch_log(Some("nightly-2026-11-13-9f42064"), make);
        assert_eq!(made.get(), 1, "a redraw reuses the parsed log");
        cache.patch_log(None, make);
        assert_eq!(made.get(), 2, "another instance plays another release");
        cache.clear();
        cache.patch_log(None, make);
        assert_eq!(made.get(), 3, "a new release list is parsed again");
    }
}
