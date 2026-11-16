//! Release notes and measured rows cached by release tag. Note panels draw only the rows
//! in view (`cull.rs`).
use crate::cull::Cull;
use crate::markdown::Notes;
use horizon_launcher::backend::Release;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A block of notes is mostly one item of 21.7 px lines.
const NOTES_BLOCK: f32 = 32.;

#[derive(Default)]
pub(super) struct NotesCache {
    /// Each opened release's notes and their rows, by tag.
    releases: RefCell<HashMap<String, (Rc<Notes>, Cull)>>,
}

impl NotesCache {
    /// The release list changed: what was parsed from the old one goes.
    pub(super) fn clear(&mut self) {
        *self = NotesCache::default();
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
