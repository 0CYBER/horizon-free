//! The open and filter flags of mods' own recipe books (arismoko/horizon#2415), beside the four vanilla
//! `RecipeBookType` pairs of [`Settings`](super::Settings): one map from a book's namespaced id
//! to its two flags. A book missing from it reads `(false, false)`. The map is saved in the
//! player's `recipeBook` compound under [`EXTENSION`]. Its entries are kept while their mod is
//! absent, so reinstalling the mod finds them again.
use horizon_core::nbt::{self, JavaString, Limits, NamedTag, Tag};
use std::collections::BTreeMap;

/// The key of the map inside `recipeBook`.
pub const EXTENSION: &str = "horizon:book_settings";
/// At most this many books, saved or registered.
pub const MAX_BOOKS: usize = 1024;
/// At most this many UTF-8 bytes in a book's id.
pub const MAX_ID_BYTES: usize = 256;
/// At most this many bytes of the saved extension, as NBT encodes it.
pub const MAX_ENCODED_BYTES: usize = 512 * 1024;

/// How the NBT reader reads [`EXTENSION`] in a saved file's `recipeBook`: at most
/// [`MAX_ENCODED_BYTES`], two levels deep (the map and a book's flags), and no key twice. The
/// same key under any other compound reads as the plain reader reads it.
pub const READ_BUDGET: nbt::Bounded = nbt::Bounded {
    parents: &[&["recipeBook"], &["Data", "Player", "recipeBook"]],
    key: EXTENSION,
    max_bytes: MAX_ENCODED_BYTES,
    max_depth: 2,
};

/// A saved `.dat` file's root, with [`READ_BUDGET`] on mods' recipe-book flags: an extension
/// over it, or with a key twice, is read past without being built, and mods' books then read
/// as closed. Everything else reads as `nbt::read_named` with `Limits::saved()` reads it.
pub fn read_saved(raw: &[u8]) -> Result<(NamedTag, usize), nbt::Error> {
    let (named, used, rejected) = nbt::read_named_bounded(raw, Limits::saved(), READ_BUDGET)?;
    if rejected > 0 {
        log::warn!(
            "Horizon: skipped {rejected} {EXTENSION} over its limits or with a repeated key;              mods' recipe books read as closed"
        );
    }
    Ok((named, used))
}

/// A mod book's two flags, as each vanilla book has: whether its screen shows the book
/// (`open`), and whether it lists only what can be crafted (`filtering`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookFlags {
    pub open: bool,
    pub filtering: bool,
}

/// Why a book's flags were not set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookError {
    /// The id is not a namespaced resource id of at most [`MAX_ID_BYTES`] bytes.
    InvalidId(String),
    /// A new book would pass [`MAX_BOOKS`]. Saved books are never dropped to make room.
    Full,
}

/// The flags of mods' books, by id, in id order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomBooks {
    books: BTreeMap<String, BookFlags>,
}

impl CustomBooks {
    /// The flags of book `id`: `(false, false)` for a book that has none.
    pub fn get(&self, id: &str) -> BookFlags {
        self.books.get(id).copied().unwrap_or_default()
    }
    /// Sets book `id`'s flags; whether they changed. A new book past [`MAX_BOOKS`] changes
    /// nothing.
    pub fn set(&mut self, id: &str, flags: BookFlags) -> Result<bool, BookError> {
        if !valid_book_id(id) {
            return Err(BookError::InvalidId(id.to_owned()));
        }
        if let Some(old) = self.books.get_mut(id) {
            return Ok(std::mem::replace(old, flags) != flags);
        }
        if self.books.len() >= MAX_BOOKS {
            return Err(BookError::Full);
        }
        if flags == BookFlags::default() {
            return Ok(false);
        }
        self.books.insert(id.to_owned(), flags);
        Ok(true)
    }
    /// Every book with saved flags and its flags, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, BookFlags)> {
        self.books.iter().map(|(id, flags)| (id.as_str(), *flags))
    }
    /// How many books have saved flags.
    pub fn len(&self) -> usize {
        self.books.len()
    }
    /// Whether no book has saved flags.
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }
    /// The map saved in `recipe_book` (the player's `recipeBook` compound). It is empty when
    /// the compound has no [`EXTENSION`], and when the extension is over a limit: more than
    /// [`MAX_BOOKS`] books, an id over [`MAX_ID_BYTES`] bytes, or more than
    /// [`MAX_ENCODED_BYTES`]. Those limits are checked before anything is read from it. A
    /// malformed entry is skipped: an invalid id, a value that is not a compound, or a flag
    /// that is not the byte 0 or 1.
    pub fn read(recipe_book: Option<&Tag>) -> Self {
        let Some(extension) = recipe_book.and_then(|tag| tag.get(EXTENSION)) else {
            return Self::default();
        };
        let Tag::Compound(entries) = extension else {
            log::warn!("Horizon: {EXTENSION} is not a compound; mods' recipe books read as closed");
            return Self::default();
        };
        if entries.len() > MAX_BOOKS
            || entries.keys().any(|id| utf8_len(id) > MAX_ID_BYTES)
            || !encoded_within(extension, MAX_ENCODED_BYTES)
        {
            log::warn!(
                "Horizon: {EXTENSION} is over its limits ({MAX_BOOKS} books, {MAX_ID_BYTES}-byte \
                 ids, {MAX_ENCODED_BYTES} bytes); mods' recipe books read as closed"
            );
            return Self::default();
        }
        let mut books = BTreeMap::new();
        for (id, value) in entries {
            let id = id.to_string_lossy();
            match read_flags(value) {
                Some(flags) if valid_book_id(&id) => {
                    books.insert(id, flags);
                }
                _ => log::warn!("Horizon: skipped the malformed {EXTENSION} entry {id:?}"),
            }
        }
        Self { books }
    }
    /// The [`EXTENSION`] compound to save, both flags of every book written (zero too), in id
    /// order. None when no book has flags, as a missing extension reads empty.
    pub fn write(&self) -> Option<Tag> {
        (!self.books.is_empty()).then(|| {
            Tag::compound(self.books.iter().map(|(id, flags)| {
                let entry = Tag::compound([
                    ("open", Tag::Byte(i8::from(flags.open))),
                    ("filtering", Tag::Byte(i8::from(flags.filtering))),
                ]);
                (id.as_str(), entry)
            }))
        })
    }
}

/// Whether `id` is a namespaced resource id a book can have: `namespace:path`, both nonempty,
/// the namespace of `[a-z0-9_.-]` and the path of `[a-z0-9_.-/]`, at most [`MAX_ID_BYTES`] bytes.
pub fn valid_book_id(id: &str) -> bool {
    id.len() <= MAX_ID_BYTES && super::resource_id::valid(id)
}

/// A book's saved flags: a compound whose `open` and `filtering` are each the byte 0 or 1.
fn read_flags(value: &Tag) -> Option<BookFlags> {
    let flag = |key: &str| match value.get(key)? {
        Tag::Byte(0) => Some(false),
        Tag::Byte(1) => Some(true),
        _ => None,
    };
    let Tag::Compound(_) = value else {
        return None;
    };
    Some(BookFlags {
        open: flag("open")?,
        filtering: flag("filtering")?,
    })
}

fn utf8_len(id: &JavaString) -> usize {
    char::decode_utf16(id.0.iter().copied())
        .map(|c| c.map_or(3, char::len_utf8))
        .sum()
}

/// Whether `tag`, as a compound's value, takes at most `budget` bytes in NBT's encoding. The
/// walk stops as soon as it passes the budget.
fn encoded_within(tag: &Tag, budget: usize) -> bool {
    fn string(s: &JavaString) -> usize {
        // Java's modified UTF-8: two bytes of length, then one to three bytes a unit.
        2 + s
            .0
            .iter()
            .map(|&u| match u {
                0x01..=0x7F => 1,
                0x00 | 0x80..=0x7FF => 2,
                _ => 3,
            })
            .sum::<usize>()
    }
    fn walk(tag: &Tag, left: &mut usize) -> bool {
        let take = |left: &mut usize, n: usize| {
            *left = match left.checked_sub(n) {
                Some(rest) => rest,
                None => return false,
            };
            true
        };
        match tag {
            Tag::End => true,
            Tag::Byte(_) => take(left, 1),
            Tag::Short(_) => take(left, 2),
            Tag::Int(_) | Tag::Float(_) => take(left, 4),
            Tag::Long(_) | Tag::Double(_) => take(left, 8),
            Tag::ByteArray(v) => take(left, 4) && take(left, v.len()),
            Tag::IntArray(v) => take(left, 4) && take(left, v.len().saturating_mul(4)),
            Tag::LongArray(v) => take(left, 4) && take(left, v.len().saturating_mul(8)),
            Tag::String(s) => take(left, string(s)),
            Tag::List(items) => take(left, 5) && items.iter().all(|item| walk(item, left)),
            Tag::Compound(map) => {
                map.iter().all(|(key, value)| {
                    take(left, 1) && take(left, string(key)) && walk(value, left)
                }) && take(left, 1)
            }
        }
    }
    let mut left = budget;
    walk(tag, &mut left)
}

#[cfg(test)]
#[path = "custom_books_tests.rs"]
mod tests;
