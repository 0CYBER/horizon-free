//! The creative mode toolbars of `hotbar.nbt` (`HotbarManager`,
//! `Hotbar`): nine saved rows of nine stacks, read through the `HOTBAR`
//! data fixers when the file is from an older version.
use crate::{items::ItemStack, save};
use horizon_world::nbt::{JavaString, Tag};
use std::path::Path;

/// `HotbarManager.NUM_HOTBAR_GROUPS`.
pub const GROUPS: usize = 9;
/// `Inventory.getSelectionSize`: the stacks of one saved row.
pub const SIZE: usize = 9;

/// One saved toolbar row.
pub type Hotbar = Vec<Option<ItemStack>>;

fn empty() -> Hotbar {
    vec![None; SIZE]
}

/// `Hotbar.CODEC` and `Hotbar.load` on the row's tag: a row of fewer than nine
/// entries is lost, a longer one is cut (`Util.fixedSize` returns the first
/// nine as a partial result), an entry that is not a stack is empty.
fn row(tag: Option<&Tag>, known: &dyn Fn(&str) -> bool) -> Hotbar {
    let Some(Tag::List(items)) = tag else {
        return empty();
    };
    if items.len() < SIZE {
        return empty();
    }
    items.iter().take(SIZE).map(|t| stack(t, known)).collect()
}

/// `ItemStack.OPTIONAL_CODEC` of a saved stack: the count is lenient (1
/// unless it is a number from 1 to 99), an item that is not registered is an
/// error.
fn stack(tag: &Tag, known: &dyn Fn(&str) -> bool) -> Option<ItemStack> {
    let stack = save::item_from_nbt(tag)?;
    if !known(&stack.item) {
        return None;
    }
    let count = match tag.get("count") {
        Some(Tag::Byte(n)) => i32::from(*n),
        Some(Tag::Short(n)) => i32::from(*n),
        Some(Tag::Int(n)) => *n,
        _ => 1,
    };
    Some(ItemStack {
        count: if (1..=99).contains(&count) {
            count as u32
        } else {
            1
        },
        ..stack
    })
}

/// `HotbarManager.load` on the file's tag.
pub fn from_tag(mut tag: Tag, known: &dyn Fn(&str) -> bool) -> Vec<Hotbar> {
    let version = match tag.get("DataVersion") {
        Some(Tag::Int(v)) => *v,
        _ => 1343,
    };
    // Until the last fixer that rewrites item stacks (`UnfilledBuriedTreasureMapFix`,
    // 5016) the fixers read each row as a list of stacks: an entry that is
    // none is dropped from its row (a bare item id is a stack from 3818.5 on)
    // and a row that is no list ends the reading. After that fixer nothing
    // reads the rows and bare ids are no stacks.
    const LAST_STACK_FIXER: i32 = 5016;
    if version < horizon_world::DATA_VERSION
        && let Tag::Compound(rows) = &mut tag
    {
        if version < LAST_STACK_FIXER {
            // A row that is no list stops the traversal there: it and the
            // rows after it (in key order) are lost.
            let broken = (0..GROUPS).find(|i| {
                rows.get(&JavaString::from(i.to_string().as_str()))
                    .is_some_and(|r| {
                        !matches!(
                            r,
                            Tag::List(_) | Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_)
                        )
                    })
            });
            if let Some(broken) = broken {
                for i in broken..GROUPS {
                    rows.remove(&JavaString::from(i.to_string().as_str()));
                }
            }
            // Entries that are no stacks are dropped from their row.
            for row in rows.values_mut() {
                if let Tag::List(items) = row {
                    items.retain(|i| {
                        matches!(i, Tag::Compound(_))
                            || (version > 3818 && matches!(i, Tag::String(_)))
                    });
                }
            }
        } else {
            for row in rows.values_mut() {
                if let Tag::List(items) = row {
                    for item in items.iter_mut().filter(|i| matches!(i, Tag::String(_))) {
                        *item = Tag::Compound(Default::default());
                    }
                }
            }
        }
    }
    horizon_world::datafix::update_hotbar(&mut tag);
    (0..GROUPS)
        .map(|i| row(tag.get(&i.to_string()), known))
        .collect()
}

/// `HotbarManager.load` of the `hotbar.nbt` at `path`: every row empty when
/// the file is missing or unreadable.
pub fn load(path: &Path, known: &dyn Fn(&str) -> bool) -> Vec<Hotbar> {
    match save::read_dat(path) {
        Some(tag @ Tag::Compound(_)) => from_tag(tag, known),
        _ => (0..GROUPS).map(|_| empty()).collect(),
    }
}

/// `HotbarManager.save`: the nine rows with the current data version, each
/// stack as `ItemStack.OPTIONAL_CODEC` writes it (an empty one is `{}`).
pub fn save(path: &Path, rows: &[Hotbar]) -> crate::Result<()> {
    let mut entries = vec![(
        JavaString::from("DataVersion"),
        Tag::Int(horizon_world::DATA_VERSION),
    )];
    for (i, row) in rows.iter().enumerate() {
        let stacks = row.iter().map(|s| match s {
            Some(s) => save::item_to_nbt(s),
            None => Tag::compound(Vec::<(&str, Tag)>::new()),
        });
        entries.push((
            JavaString::from(i.to_string().as_str()),
            Tag::List(stacks.collect()),
        ));
    }
    save::write_dat(path, Tag::compound(entries))
}
