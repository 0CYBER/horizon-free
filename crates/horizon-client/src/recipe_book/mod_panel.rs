//! Borrowed screen projection of a validated mod book. No callbacks or JSON
//! interpretation occur while choosing tabs or searching a panel.
use super::{ModBookIndex, RecipeBookCategory};
use mod_api::item::recipe_book::RecipeBookTypeKey;

/// A panel is bound to both the screen's menu and its registered book identity.
/// The negotiated settings snapshot must be available before enabling it.
pub struct ModBookPanel<'a> {
    index: &'a ModBookIndex,
    book: &'a RecipeBookTypeKey,
}

impl ModBookIndex {
    /// Resolve a screen binding. A key that belongs to another menu is refused,
    /// even when it is a valid registered book. Empty books have no panel.
    pub fn panel<'a>(
        &'a self,
        menu: &str,
        book: &'a RecipeBookTypeKey,
    ) -> Option<ModBookPanel<'a>> {
        let (target, _, _) = self.books.get(book.as_str())?;
        (target == menu).then_some(ModBookPanel { index: self, book })
    }
}

impl<'a> ModBookPanel<'a> {
    /// Stable category order, shared by tabs and the per-book search tab.
    pub fn categories(&self) -> impl Iterator<Item = &'a RecipeBookCategory> + '_ {
        self.index
            .search_categories(self.book.as_str())
            .iter()
            .filter_map(|id| self.index.category(id))
    }

    /// The search category declared for this book, never a global search union.
    pub fn search_group(&self) -> &'a str {
        &self.index.books[self.book.as_str()].1
    }

    /// Resolve a category selection only within this panel. A stale tab from a
    /// previously opened book must not expose the other book's recipes.
    pub fn category(&self, id: &str) -> Option<&'a RecipeBookCategory> {
        self.categories().find(|category| category.id == id)
    }
}
