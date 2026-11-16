//! The design's storage, persistence, copy and replace acceptance cases (arismoko/horizon#2415).
use super::*;
use crate::recipe_book::{BookType, Settings};

const POT: &str = "demo:cooking_pot";
const OVEN: &str = "demo:oven";
const OPEN: BookFlags = BookFlags {
    open: true,
    filtering: false,
};
const FILTERING: BookFlags = BookFlags {
    open: false,
    filtering: true,
};

/// A saved `recipeBook` compound: the vanilla lists, the furnace book open and `extension`.
fn saved(extension: Option<Tag>) -> Tag {
    let mut entries = vec![
        ("recipes", Tag::List(vec![])),
        ("toBeDisplayed", Tag::List(vec![])),
        ("isFurnaceGuiOpen", Tag::Byte(1)),
    ];
    entries.extend(extension.map(|tag| (EXTENSION, tag)));
    Tag::compound(entries)
}

fn entry(open: Tag, filtering: Tag) -> Tag {
    Tag::compound([("open", open), ("filtering", filtering)])
}

#[test]
fn a_fresh_book_reads_closed_and_one_book_leaves_another_alone() {
    let mut books = CustomBooks::default();
    assert_eq!(books.get(POT), BookFlags::default());
    assert_eq!(books.get(OVEN), BookFlags::default());
    assert_eq!(books.set(POT, OPEN), Ok(true));
    assert_eq!(books.get(POT), OPEN);
    assert_eq!(books.get(OVEN), BookFlags::default());
    assert_eq!(books.set(POT, OPEN), Ok(false), "an unchanged pair");
    assert_eq!(books.set(OVEN, BookFlags::default()), Ok(false));
    assert_eq!(books.len(), 1, "a default pair adds no entry");
}

#[test]
fn a_saved_book_loads_exactly_and_the_vanilla_pairs_stay() {
    let mut settings = Settings::read(Some(&saved(None)));
    settings.filtering[BookType::Crafting.index()] = true;
    settings.custom.set(POT, OPEN).unwrap();
    let vanilla = (settings.open, settings.filtering);
    let loaded = Settings::read(Some(&settings.write()));
    assert_eq!(loaded.custom.get(POT), OPEN);
    assert_eq!((loaded.open, loaded.filtering), vanilla);
    assert_eq!(loaded.custom, settings.custom);
}

#[test]
fn both_flags_are_written_zero_too_in_id_order() {
    let mut books = CustomBooks::default();
    books.set(POT, FILTERING).unwrap();
    books.set("demo:anvil", OPEN).unwrap();
    let Some(Tag::Compound(written)) = books.write() else {
        panic!("an extension");
    };
    let ids: Vec<String> = written.keys().map(JavaString::to_string_lossy).collect();
    assert_eq!(ids, ["demo:anvil", POT]);
    assert_eq!(
        written.get(&JavaString::from(POT)),
        Some(&entry(Tag::Byte(0), Tag::Byte(1)))
    );
    assert_eq!(
        CustomBooks::default().write(),
        None,
        "no books, no extension"
    );
}

#[test]
fn a_book_without_its_mod_is_kept_through_save_and_load() {
    // Nothing is registered here: storage keeps every well-formed entry, so the pot's flags
    // are back when its mod is installed again.
    let tag = saved(Some(Tag::compound([(
        POT,
        entry(Tag::Byte(1), Tag::Byte(0)),
    )])));
    let loaded = Settings::read(Some(&tag));
    assert_eq!(loaded.custom.get(POT), OPEN);
    let again = Settings::read(Some(&loaded.write()));
    assert_eq!(again.custom.get(POT), OPEN);
}

#[test]
fn a_copy_is_independent_and_a_replace_drops_what_only_the_destination_had() {
    let mut a = Settings::default();
    a.custom.set(POT, OPEN).unwrap();
    let b = a.clone();
    a.custom.set(POT, FILTERING).unwrap();
    assert_eq!(b.custom.get(POT), OPEN, "the copy is a value");
    let mut destination = Settings::default();
    destination
        .custom
        .set(
            OVEN,
            BookFlags {
                open: true,
                filtering: true,
            },
        )
        .unwrap();
    destination = b.clone();
    assert_eq!(destination.custom.iter().collect::<Vec<_>>(), [(POT, OPEN)]);
    assert_eq!(destination.custom.get(OVEN), BookFlags::default());
}

#[test]
fn a_book_without_the_extension_reads_empty() {
    assert!(Settings::read(Some(&saved(None))).custom.is_empty());
    assert!(Settings::read(None).custom.is_empty());
    assert!(
        Settings::default().write().get(EXTENSION).is_none(),
        "a vanilla-only book writes what it wrote before"
    );
}

#[test]
fn lists_that_do_not_decode_keep_the_mods_flags() {
    let tag = Tag::compound([(
        EXTENSION,
        Tag::compound([(POT, entry(Tag::Byte(1), Tag::Byte(0)))]),
    )]);
    let loaded = Settings::read(Some(&tag));
    assert!(loaded.recipes.is_empty(), "no lists: vanilla reads nothing");
    assert_eq!(loaded.custom.get(POT), OPEN);
}

#[test]
fn a_malformed_entry_is_skipped_and_the_rest_read() {
    let extension = Tag::compound([
        (POT, entry(Tag::Byte(1), Tag::Byte(0))),
        ("demo:two", entry(Tag::Byte(2), Tag::Byte(0))),
        ("demo:int", entry(Tag::Int(1), Tag::Byte(0))),
        ("demo:half", Tag::compound([("open", Tag::Byte(1))])),
        ("demo:flat", Tag::Byte(1)),
        ("Demo:Upper", entry(Tag::Byte(1), Tag::Byte(1))),
        ("no_namespace", entry(Tag::Byte(1), Tag::Byte(1))),
    ]);
    let books = CustomBooks::read(Some(&saved(Some(extension))));
    assert_eq!(books.iter().collect::<Vec<_>>(), [(POT, OPEN)]);
}

#[test]
fn an_extension_over_a_limit_reads_empty_and_vanilla_still_reads() {
    let books = |n: usize| {
        Tag::Compound(
            (0..n)
                .map(|i| {
                    (
                        JavaString::from(format!("demo:book_{i}").as_str()),
                        entry(Tag::Byte(1), Tag::Byte(0)),
                    )
                })
                .collect(),
        )
    };
    assert_eq!(
        CustomBooks::read(Some(&saved(Some(books(MAX_BOOKS))))).len(),
        MAX_BOOKS
    );
    let over = saved(Some(books(MAX_BOOKS + 1)));
    let settings = Settings::read(Some(&over));
    assert!(settings.custom.is_empty(), "too many books");
    assert!(
        settings.open[BookType::Furnace.index()],
        "vanilla pairs still read"
    );

    let long = format!("demo:{}", "a".repeat(MAX_ID_BYTES));
    let tag = Tag::compound([
        (POT, entry(Tag::Byte(1), Tag::Byte(0))),
        (long.as_str(), entry(Tag::Byte(1), Tag::Byte(0))),
    ]);
    assert!(
        CustomBooks::read(Some(&saved(Some(tag)))).is_empty(),
        "an id too long"
    );

    let padding = Tag::ByteArray(vec![0; MAX_ENCODED_BYTES]);
    let tag = Tag::compound([
        (POT, entry(Tag::Byte(1), Tag::Byte(0))),
        ("demo:padded", Tag::compound([("pad", padding)])),
    ]);
    assert!(
        CustomBooks::read(Some(&saved(Some(tag)))).is_empty(),
        "too many bytes"
    );

    assert!(
        CustomBooks::read(Some(&saved(Some(Tag::Byte(1))))).is_empty(),
        "not a compound"
    );
}

#[test]
fn a_full_map_refuses_a_new_book_without_dropping_one() {
    let mut books = CustomBooks::default();
    for i in 0..MAX_BOOKS {
        books.set(&format!("demo:book_{i}"), OPEN).unwrap();
    }
    assert_eq!(books.set(POT, OPEN), Err(BookError::Full));
    assert_eq!(books.len(), MAX_BOOKS);
    assert_eq!(books.get(POT), BookFlags::default());
    assert_eq!(
        books.set("demo:book_0", FILTERING),
        Ok(true),
        "a saved book still changes"
    );
}

#[test]
fn a_book_id_is_a_namespaced_resource_id() {
    for id in [POT, "a:b", "my-mod.x:path/to_book", "minecraft:crafting"] {
        assert!(valid_book_id(id), "{id}");
    }
    let long = format!("demo:{}", "a".repeat(MAX_ID_BYTES));
    for id in [
        "",
        "demo",
        ":pot",
        "demo:",
        "Demo:pot",
        "demo:Pot",
        "de/mo:pot",
        "demo:pot!",
        long.as_str(),
    ] {
        assert!(!valid_book_id(id), "{id:?}");
    }
    assert_eq!(
        CustomBooks::default().set("demo", OPEN),
        Err(BookError::InvalidId("demo".into()))
    );
}

#[test]
fn the_encoded_size_counts_as_nbt_writes_it() {
    let tag = Tag::compound([(POT, entry(Tag::Byte(1), Tag::Byte(0)))]);
    let bytes = horizon_core::nbt::write_network(&tag, horizon_core::nbt::Limits::saved()).unwrap();
    // `write_network` adds the root's own type byte.
    let size = bytes.len() - 1;
    assert!(encoded_within(&tag, size));
    assert!(!encoded_within(&tag, size - 1));
}

/// A saved player root in raw NBT: `recipeBook` with empty lists, the furnace book open and an
/// `EXTENSION` compound whose entries `books` writes.
fn raw_player(books: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = vec![10, 0, 0];
    key(&mut out, 10, "recipeBook");
    for list in ["recipes", "toBeDisplayed"] {
        key(&mut out, 9, list);
        out.extend([8, 0, 0, 0, 0]);
    }
    key(&mut out, 1, "isFurnaceGuiOpen");
    out.push(1);
    key(&mut out, 10, EXTENSION);
    books(&mut out);
    out.extend([0, 0, 0]);
    out
}

fn key(out: &mut Vec<u8>, id: u8, name: &str) {
    out.push(id);
    out.extend((name.len() as u16).to_be_bytes());
    out.extend(name.as_bytes());
}

fn raw_book(out: &mut Vec<u8>, id: &str, open: u8) {
    key(out, 10, id);
    key(out, 1, "open");
    out.push(open);
    key(out, 1, "filtering");
    out.push(0);
    out.push(0);
}

/// The settings a raw player file loads, and whether its `recipeBook` kept the extension.
fn load(raw: &[u8]) -> (Settings, bool) {
    let (named, used) = read_saved(raw).expect("the file reads");
    assert_eq!(used, raw.len());
    let book = named.tag.get("recipeBook");
    let kept = book.and_then(|b| b.get(EXTENSION)).is_some();
    (Settings::read(book), kept)
}

#[test]
fn a_saved_file_reads_the_extension_within_its_budget() {
    let (settings, kept) = load(&raw_player(|out| raw_book(out, POT, 1)));
    assert!(kept);
    assert_eq!(settings.custom.get(POT), OPEN);
    assert!(settings.open[BookType::Furnace.index()]);
}

#[test]
fn a_saved_extension_with_a_book_twice_is_dropped_and_vanilla_reads() {
    // A `Tag::Compound` cannot hold the same key twice, so only raw bytes show this.
    let (settings, kept) = load(&raw_player(|out| {
        raw_book(out, POT, 1);
        raw_book(out, POT, 0);
    }));
    assert!(!kept, "the reader leaves the extension out");
    assert!(settings.custom.is_empty());
    assert!(settings.open[BookType::Furnace.index()]);
}

#[test]
fn a_saved_extension_over_its_byte_or_depth_budget_is_dropped_unbuilt() {
    let over_bytes = raw_player(|out| {
        key(out, 10, POT);
        key(out, 7, "pad");
        out.extend(((MAX_ENCODED_BYTES + 1) as i32).to_be_bytes());
        out.extend(std::iter::repeat_n(0, MAX_ENCODED_BYTES + 1));
        out.push(0);
    });
    let too_deep = raw_player(|out| {
        key(out, 10, POT);
        key(out, 10, "nested");
        out.extend([0, 0]);
    });
    for raw in [over_bytes, too_deep] {
        let (settings, kept) = load(&raw);
        assert!(!kept);
        assert!(settings.custom.is_empty());
        assert!(
            settings.open[BookType::Furnace.index()],
            "vanilla still reads"
        );
    }
}

#[test]
fn a_truncated_extension_still_fails_the_file() {
    let mut raw = raw_player(|out| raw_book(out, POT, 1));
    raw.truncate(raw.len() - 4);
    assert!(read_saved(&raw).is_err());
}

#[test]
fn the_same_key_outside_recipe_book_reads_as_before() {
    // {OtherData:{"horizon:book_settings":{x:1b, x:2b}}}: no recipe book here, so the plain
    // reader's last-duplicate-wins stands.
    let mut raw = vec![10, 0, 0];
    key(&mut raw, 10, "OtherData");
    key(&mut raw, 10, EXTENSION);
    for value in [1, 2] {
        key(&mut raw, 1, "x");
        raw.push(value);
    }
    raw.extend([0, 0, 0]);
    let (named, _) = read_saved(&raw).unwrap();
    let (plain, _) = horizon_core::nbt::read_named(&raw, Limits::saved()).unwrap();
    assert_eq!(named, plain);
    let x = named
        .tag
        .get("OtherData")
        .and_then(|d| d.get(EXTENSION))
        .and_then(|e| e.get("x"));
    assert_eq!(x, Some(&Tag::Byte(2)));
}

/// The extension with `x` twice, inside `recipeBook` under the compounds `path` names.
fn nested_book(path: &[&str]) -> Vec<u8> {
    let mut raw = vec![10, 0, 0];
    for name in path.iter().chain(["recipeBook"].iter()) {
        key(&mut raw, 10, name);
    }
    key(&mut raw, 10, EXTENSION);
    for value in [1, 2] {
        key(&mut raw, 1, "x");
        raw.push(value);
    }
    raw.extend(std::iter::repeat_n(0, path.len() + 3));
    raw
}

#[test]
fn only_the_players_own_recipe_book_is_budgeted() {
    // A `recipeBook` that is not the player's reads as the plain reader reads it.
    let raw = nested_book(&["OtherData"]);
    let (named, _) = read_saved(&raw).unwrap();
    let (plain, _) = horizon_core::nbt::read_named(&raw, Limits::saved()).unwrap();
    assert_eq!(named, plain);
    let x = ["OtherData", "recipeBook", EXTENSION, "x"]
        .iter()
        .try_fold(&named.tag, |tag, key| tag.get(key));
    assert_eq!(x, Some(&Tag::Byte(2)));
    // The player's book: the root's, or `level.dat`'s `Data.Player`.
    for path in [&[][..], &["Data", "Player"]] {
        let (named, _) = read_saved(&nested_book(path)).unwrap();
        let book = path
            .iter()
            .chain(["recipeBook"].iter())
            .try_fold(&named.tag, |tag, key| tag.get(key));
        assert_eq!(book.map(|b| b.get(EXTENSION)), Some(None), "{path:?}");
    }
}
