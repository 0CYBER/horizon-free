//! The export over a small fixture API, and over the real tree.
use super::*;
use std::collections::BTreeMap;

/// The fixture: path below the root, and text.
const FILES: &[(&str, &str)] = &[
    (
        "crates/horizon-mod-api/src/lib.rs",
        "pub use mod_vocab::*;\n",
    ),
    (
        "crates/horizon-mod-vocab/src/lib.rs",
        r#"
pub mod entity;
pub use entity::EntityRef;
pub use horizon_mod_api_macros::{Wire, horizon_mod};
#[doc(hidden)]
pub use events::*;
#[doc(hidden)]
pub mod internal {
    pub struct Secret;
}
mod events {
    use super::*;
    /// The event of `ping`.
    pub struct PingEvent<'a> {
        pub who: EntityRef,
        pub lines: &'a mut Vec<(Text, u32)>,
        pub cancelled: bool,
    }
    /// The event of `tick_block`.
    pub struct TickBlockEvent {
        pub pos: Point,
    }
}
/// A text: parts of parts.
#[derive(Clone, Debug, Wire)]
pub struct Text {
    pub parts: Vec<Text>,
    style: u8,
}
/// A point.
#[derive(Clone, Copy, Wire)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
/// Limits a mod reads but cannot build.
#[non_exhaustive]
pub struct Limits {
    pub max: u32,
}
/// A colour.
#[non_exhaustive]
pub enum Colour {
    Red,
    Rgb(u8, u8, u8),
}
/// A mode.
pub enum Mode {
    On,
    Off { reason: String },
}
/// What a mod answers.
pub trait Hooks {
    /// Pinged.
    fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>) {
        let _ = (host, event);
    }
    /// Must be written.
    fn name(&self) -> &'static str;
}
/// The game.
pub trait Host {
    /// The time.
    fn time(&self) -> u64;
}
/// The server's game.
pub trait ServerHost: Host {
    /// Saves.
    ///
    /// Not on wasm: it writes
    /// files.
    fn save(&mut self, slot: u8);
    /// Sends a text.
    fn send(&mut self, text: Text) -> bool;
}
/// A block's part.
pub trait BlockPart {}
/// Finds the ground.
pub fn ground(at: EntityRef) -> Option<Point> {
    let _ = at;
    None
}
/// The first part.
pub fn first<'a>(text: &'a Text) -> &'a Text {
    text
}
/// A capability key.
pub struct CapabilityKey<T: ?Sized> {
    pub name: &'static str,
    pub handler: std::marker::PhantomData<T>,
}
/// Fluids, through a handler.
pub trait FluidHandler {}
/// Fluids.
pub const FLUIDS: CapabilityKey<dyn FluidHandler> = CapabilityKey {
    name: "fixture:fluids",
    handler: std::marker::PhantomData,
};
/// Fluids, made by a constructor.
pub const MORE_FLUIDS: CapabilityKey<dyn FluidHandler> = CapabilityKey::new("fixture:more_fluids");
pub const LIMIT: u32 = 3;
/// Makes a key.
#[macro_export]
macro_rules! key {
    () => {};
}
/// A result of the fixture.
pub type Outcome<T> = Result<T, Colour>;
impl Default for Point {
    fn default() -> Self {
        Self { x: 0, y: 0 }
    }
}
impl Point {
    /// The origin.
    pub const ORIGIN: Point = Point { x: 0, y: 0 };
}
/// A key whose Wire impl a macro writes.
pub struct SlotKey(u32);
macro_rules! slot_keys {
    ($width:ty; $($key:ty),+ $(,)?) => {$(
        impl Wire for $key {
            layout_fields!("slot" => $width);
        }
    )+};
}
slot_keys!(u32; SlotKey);
/// An identifier, encoded by hand.
pub struct Ident {
    cache: u64,
    pub namespace: String,
}
impl Wire for Ident {
    layout_fields!("namespace" => String);
}
/// A name that crosses as a string.
pub struct Name(String);
impl Wire for Name {
    fn layout(visiting: &mut Vec<&'static str>) -> u64 {
        String::layout(visiting)
    }
}
/// Context that crosses by a hand-written impl alone.
pub struct Context;
impl Wire for Context {}
/// Where Parts are attached.
pub trait Registrar {}
impl dyn Registrar + '_ {
    /// Attaches a part to the block `key` names.
    pub fn attach(&mut self, key: SlotKey) {
        let _ = key;
    }
}
/// Data with an entry type.
pub trait DataMap {
    /// Its entry.
    type Entry;
}
macro_rules! moves {
    ($to:ident, $from:ident) => {
        impl Point {
            /// Moves to.
            pub fn $to(&self) {}
            /// Moves from.
            pub fn $from(&self) {}
        }
    };
}
moves!(move_to, move_from);
macro_rules! draws {
    ($($host:ident),+) => { $(
        impl dyn $host + '_ {
            /// Draws.
            pub fn draw(&self) {}
        }
    )+ };
}
draws!(Host, Registrar);
macro_rules! outer {
    ($name:ident) => {
        moves!($name, inner_from);
        missing_inner!();
    };
}
outer!(outer_to);
unknown!(Point);
#[cfg(feature = "extra")]
impl Point {
    /// Only with `extra`.
    pub fn extra(&self) {}
}
/// A module reached by more routes than the walk follows.
pub mod shared {
    /// A thing.
    pub struct Thing;
}
pub use shared as s1;
pub use shared as s2;
pub use shared as s3;
pub use shared as s4;
pub use shared as s5;
pub use shared as s6;
pub use shared as s7;
pub use shared as s8;
pub use shared as s9;
pub use shared as s10;
pub use shared as s11;
pub use shared as s12;
pub use shared as s13;
pub use shared as s14;
pub use shared as s15;
pub use shared as s16;
pub use shared as s17;
"#,
    ),
    (
        "crates/horizon-mod-vocab/src/entity.rs",
        r#"
/// A live entity.
#[derive(Clone, Copy, Wire)]
pub struct EntityRef(u32);
impl EntityRef {
    /// Its id.
    pub fn id(self) -> u32 {
        self.0
    }
    /// Its file.
    ///
    /// Not on wasm: it reads a file.
    pub fn path(self) -> crate::Outcome<String> {
        Ok(String::new())
    }
    fn private(self) {}
}
"#,
    ),
    (
        "crates/horizon-mod-api-macros/src/lib.rs",
        r#"
/// The derive.
#[proc_macro_derive(Wire, attributes(wire))]
pub fn derive_wire(input: TokenStream) -> TokenStream { input }
/// Marks the mod.
#[proc_macro_attribute]
pub fn horizon_mod(_: TokenStream, input: TokenStream) -> TokenStream { input }
"#,
    ),
    (
        "crates/horizon-mod-package/src/manifest.rs",
        r#"
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New registries.
    Registries,
}
"#,
    ),
];

const HOOKS: &str = r#"
pub trait Hooks {
    /// Something pinged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancelled",
        target = "horizon_client::game::Game::ping",
        neoforge = "PingEvent"
    )]
    fn ping();
    /// A block ticks.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "pos",
        target = "horizon_client::game::GameLevel::tick_block",
        neoforge = "none"
    )]
    fn tick_block();
}
"#;

fn boundary() -> Boundary {
    let mut boundary = Boundary::default();
    boundary.roles.push(("Hooks".into(), Role::Implement));
    boundary.roles.push(("Host".into(), Role::Call));
    boundary.roles.push(("ServerHost".into(), Role::Call));
    boundary.sides.push(("ServerHost".into(), Side::Server));
    boundary.unbound.push((
        "ServerHost".into(),
        "send".into(),
        "Text does not cross".into(),
    ));
    boundary
}

/// An edit that moves its file to the path given as `to`.
const MOVE: &str = "<move>";

/// The export of the fixture with `edits` (file, from, to) applied.
fn export_fixture(edits: &[(&str, &str, &str)], hooks: &str) -> Export {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (path, text) in FILES {
        let mut text = text.to_string();
        let mut path = *path;
        for (file, from, to) in edits {
            if *file != path {
                continue;
            }
            if *from == MOVE {
                path = to;
            } else {
                assert!(text.contains(from), "{from} not in {path}");
                text = text.replacen(from, to, 1);
            }
        }
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let catalog = Catalog::parse(&[("hooks/ping.rs".into(), hooks.into())]).unwrap();
    let mut index = Index::new(root, &[]);
    build(root, &mut index, &catalog.hooks, &boundary()).unwrap()
}

const LIB: &str = "crates/horizon-mod-vocab/src/lib.rs";

fn hashes(export: &Export) -> BTreeMap<String, String> {
    export
        .surface
        .entries()
        .map(|(id, hash)| (id.to_owned(), hash.to_owned()))
        .collect()
}

/// The ids whose hash changed, and the ids added, from the base fixture to the edited one.
fn changes(edits: &[(&str, &str, &str)], hooks: &str) -> (Vec<String>, Vec<String>) {
    let before = hashes(&export_fixture(&[], HOOKS));
    let after = hashes(&export_fixture(edits, hooks));
    let changed = before
        .iter()
        .filter(|(id, hash)| after.get(*id).is_some_and(|new| new != *hash))
        .map(|(id, _)| id.clone())
        .collect();
    let added = after
        .keys()
        .filter(|id| !before.contains_key(*id))
        .cloned()
        .collect();
    (changed, added)
}

/// `value` with every `hash` and every present `shape` written as `*`.
fn masked(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| match key.as_str() {
                    "hash" | "shape" | "fields_hash" if !value.is_null() => {
                        (key, serde_json::Value::from("*"))
                    }
                    _ => (key, masked(value)),
                })
                .collect(),
        ),
        serde_json::Value::Array(list) => {
            serde_json::Value::Array(list.into_iter().map(masked).collect())
        }
        value => value,
    }
}

#[test]
fn the_fixture_exports_the_expected_document() {
    let export = export_fixture(&[], HOOKS);
    let written: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixture_expected.json")).unwrap();
    assert_eq!(masked(written), expected, "{}", export.surface.to_json());
    assert_eq!(
        export.notes,
        [
            "0 public items are not public at their definition path: their id is their shortest public path, which a shorter re-export added later would change",
            "3 items are reachable only through #[doc(hidden)] items or modules: recorded only where a recorded signature names them",
            "constant LIMIT: no record kind for constants",
            "impl Point in mod_api: compiled out of a default native build (cfg(feature = \"extra\")), not recorded",
            "missing_inner! called in mod_api (inside another macro's expansion): not expanded, the items it writes are not recorded",
            "type:Context: no shape, because Context crosses by a hand-written Wire impl that declares no layout, or does not cross",
            "unknown! called in mod_api: not expanded, the items it writes are not recorded",
        ]
    );
}

#[test]
fn macro_calls_are_expanded_or_noted_and_ids_ignore_routes() {
    let export = export_fixture(&[], HOOKS);
    let handles: Vec<&str> = export
        .surface
        .handles
        .iter()
        .map(|h| h.id.as_str())
        .collect();
    for id in [
        "handle:Point::move_to",
        "handle:Point::move_from",
        "handle:Host::draw",
        "handle:Registrar::draw",
        "handle:Point::outer_to",
        "handle:Point::inner_from",
    ] {
        assert!(handles.contains(&id), "{id} in {handles:?}");
    }
    assert!(!handles.contains(&"handle:Point::extra"));
    let thing = export
        .surface
        .types
        .iter()
        .find(|t| t.id.ends_with("Thing"))
        .unwrap();
    assert_eq!(thing.id, "type:shared::Thing");
    assert!(thing.paths.contains(&"s1::Thing".to_owned()));
}

#[test]
fn two_exports_are_byte_identical() {
    let a = export_fixture(&[], HOOKS).surface.to_json();
    let b = export_fixture(&[], HOOKS).surface.to_json();
    assert_eq!(a, b);
    assert!(a.ends_with("}\n"));
}

#[test]
fn each_declaration_aspect_changes_exactly_its_records_hash() {
    let cases: &[(&str, &[(&str, &str, &str)], &str, &[&str])] = &[
        (
            "a method's argument type",
            &[(
                LIB,
                "fn save(&mut self, slot: u8);",
                "fn save(&mut self, slot: u16);",
            )],
            HOOKS,
            &["method:ServerHost::save"],
        ),
        (
            "a lifetime",
            &[(LIB, "-> &'a Text {", "-> &'static Text {")],
            HOOKS,
            &["fn:first"],
        ),
        (
            "a field added to a constructible struct: its fields_hash, not its hash",
            &[(
                LIB,
                "    pub y: i32,\n",
                "    pub y: i32,\n    pub z: i32,\n",
            )],
            HOOKS,
            &[],
        ),
        (
            "a variant added to an exhaustive enum",
            &[(LIB, "    On,\n", "    On,\n    Paused,\n")],
            HOOKS,
            &["type:Mode"],
        ),
        (
            "non_exhaustive toggled",
            &[(LIB, "/// A mode.\n", "/// A mode.\n#[non_exhaustive]\n")],
            HOOKS,
            &["type:Mode"],
        ),
        (
            "a derive added: one more of its impls, outside its hash",
            &[(LIB, "/// A mode.\n", "/// A mode.\n#[derive(Debug)]\n")],
            HOOKS,
            &[],
        ),
        (
            "a required method added to a trait mods implement",
            &[(
                LIB,
                "    fn name(&self)",
                "    fn more(&self);\n    fn name(&self)",
            )],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a required method added to a trait the boundary lists do not name (role none)",
            &[(
                LIB,
                "pub trait BlockPart {}",
                "pub trait BlockPart {\n    fn tick(&self);\n}",
            )],
            HOOKS,
            &["trait:BlockPart"],
        ),
        (
            "a handle method's return type",
            &[(
                "crates/horizon-mod-vocab/src/entity.rs",
                "pub fn id(self) -> u32",
                "pub fn id(self) -> u64",
            )],
            HOOKS,
            &["handle:entity::EntityRef::id"],
        ),
        (
            "a hook's side",
            &[],
            &HOOKS.replacen("side = \"server\"", "side = \"both\"", 1),
            &["hook:ping"],
        ),
        (
            "an argument added to a hook",
            &[(
                LIB,
                "pub cancelled: bool,",
                "pub cancelled: bool,\n        pub extra: u8,",
            )],
            HOOKS,
            &[],
        ),
        (
            "a hook argument's type",
            &[(LIB, "pub cancelled: bool,", "pub cancelled: u8,")],
            HOOKS,
            &["hook:ping.cancelled", "type:PingEvent.cancelled"],
        ),
        (
            "a supertrait bound on a trait mods call",
            &[(
                LIB,
                "pub trait ServerHost: Host {",
                "pub trait ServerHost: Host + Send {",
            )],
            HOOKS,
            &[],
        ),
        (
            "a supertrait bound on a trait mods implement",
            &[(LIB, "pub trait Hooks {", "pub trait Hooks: Send {")],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a defaulted method becoming required on a trait mods implement",
            &[(
                LIB,
                "fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>) {\n        let _ = (host, event);\n    }",
                "fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>);",
            )],
            HOOKS,
            &["method:Hooks::ping", "trait:Hooks"],
        ),
        (
            "an associated type added to a trait mods implement",
            &[(
                LIB,
                "    /// Must be written.",
                "    /// Its settings.\n    type Settings;\n    /// Must be written.",
            )],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a receiver becoming mutable",
            &[(LIB, "fn time(&self) -> u64;", "fn time(&mut self) -> u64;")],
            HOOKS,
            &["method:Host::time"],
        ),
        (
            "a hook's cancellable flag",
            &[],
            &HOOKS.replacen(
                "neoforge = \"PingEvent\"",
                "neoforge = \"PingEvent\",\n        cancellable = true",
                1,
            ),
            &["hook:ping"],
        ),
        (
            "a trait impl added: outside the hash",
            &[(
                LIB,
                "/// A mode.\npub enum Mode {",
                "impl Default for Mode {\n    fn default() -> Self {\n        Self::On\n    }\n}\n/// A mode.\npub enum Mode {",
            )],
            HOOKS,
            &[],
        ),
        (
            "a handle's impl bounds",
            &[(
                "crates/horizon-mod-vocab/src/entity.rs",
                "impl EntityRef {",
                "impl EntityRef where Self: Copy {",
            )],
            HOOKS,
            &[
                "handle:entity::EntityRef::id",
                "handle:entity::EntityRef::path",
            ],
        ),
        (
            "trait methods reordered",
            &[(
                LIB,
                "    /// Saves.\n    ///\n    /// Not on wasm: it writes\n    /// files.\n    fn save(&mut self, slot: u8);\n    /// Sends a text.\n    fn send(&mut self, text: Text) -> bool;",
                "    /// Sends a text.\n    fn send(&mut self, text: Text) -> bool;\n    /// Saves.\n    ///\n    /// Not on wasm: it writes\n    /// files.\n    fn save(&mut self, slot: u8);",
            )],
            HOOKS,
            &[],
        ),
        (
            "a file moved under the same public path",
            &[(
                "crates/horizon-mod-vocab/src/entity.rs",
                MOVE,
                "crates/horizon-mod-vocab/src/entity/mod.rs",
            )],
            HOOKS,
            &[],
        ),
        (
            "a new root re-export",
            &[(
                LIB,
                "pub use entity::EntityRef;",
                "pub use entity::EntityRef;\npub use entity::EntityRef as Handle;",
            )],
            HOOKS,
            &[],
        ),
        (
            "a not-on-wasm reason reworded",
            &[(
                LIB,
                "/// Not on wasm: it writes\n    /// files.",
                "/// Not on wasm: it writes to the disk.",
            )],
            HOOKS,
            &[],
        ),
        (
            "a doc line",
            &[(LIB, "/// A point.", "/// A point on a plane.")],
            HOOKS,
            &[],
        ),
        (
            "a hook's doc, engine target and counterpart",
            &[],
            &HOOKS
                .replace("Something pinged.", "Something was pinged.")
                .replace("Game::ping", "Game::pinged")
                .replace("neoforge = \"PingEvent\"", "neoforge = \"PingedEvent\""),
            &[],
        ),
        (
            "a method added to a trait mods call",
            &[(
                LIB,
                "    fn time(&self) -> u64;",
                "    fn time(&self) -> u64;\n    fn day(&self) -> u64;",
            )],
            HOOKS,
            &[],
        ),
        (
            "a defaulted method added to a trait mods implement",
            &[(
                LIB,
                "    fn name(&self)",
                "    fn more(&self) {}\n    fn name(&self)",
            )],
            HOOKS,
            &[],
        ),
        (
            "a field added to a type mods cannot build",
            &[(
                LIB,
                "    style: u8,\n",
                "    style: u8,\n    pub size: u8,\n",
            )],
            HOOKS,
            &[],
        ),
        (
            "a variant added to a non-exhaustive enum",
            &[(LIB, "    Red,\n", "    Red,\n    Blue,\n")],
            HOOKS,
            &[],
        ),
    ];
    for (aspect, edits, hooks, expected) in cases {
        let (changed, added) = changes(edits, hooks);
        assert_eq!(changed, *expected, "{aspect}");
        if aspect.starts_with("trait methods") || aspect.starts_with("a file moved") {
            assert_eq!(added, Vec::<String>::new(), "{aspect}");
        }
    }
    // A new hook (with its event type) adds records and changes none.
    let pong = HOOKS.replace(
        "fn tick_block();\n}",
        "fn tick_block();\n    /// Ponged.\n    #[hook(side = \"client\", contract = \"listen\", target = \"x::pong\", neoforge = \"none\")]\n    fn pong();\n}",
    );
    let (changed, added) = changes(
        &[(
            LIB,
            "mod events {\n    use super::*;",
            "mod events {\n    use super::*;\n    /// The event of `pong`.\n    pub struct PongEvent { pub n: u8 }",
        )],
        &pong,
    );
    assert_eq!(changed, Vec::<String>::new());
    assert!(added.contains(&"hook:pong".to_owned()), "{added:?}");
}

#[test]
fn shapes_follow_the_wire_layout() {
    let base = export_fixture(&[], HOOKS);
    let shape = |export: &Export, id: &str| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .shape
            .clone()
    };
    // Reordering fields changes the shape, not the contract of a type mods cannot build.
    let reordered = export_fixture(
        &[(
            LIB,
            "    pub parts: Vec<Text>,\n    style: u8,",
            "    style: u8,\n    pub parts: Vec<Text>,",
        )],
        HOOKS,
    );
    assert_ne!(shape(&base, "type:Text"), shape(&reordered, "type:Text"));
    // A nested type's change reaches the types that hold it.
    let nested = export_fixture(&[(LIB, "    pub y: i32,\n}", "    pub y: i64,\n}")], HOOKS);
    let holder = |e: &Export| shape(e, "type:Text");
    assert!(holder(&base).is_some());
    let with_point = export_fixture(
        &[(
            LIB,
            "    style: u8,\n}",
            "    style: u8,\n    at: Point,\n}",
        )],
        HOOKS,
    );
    let with_wider_point = export_fixture(
        &[
            (
                LIB,
                "    style: u8,\n}",
                "    style: u8,\n    at: Point,\n}",
            ),
            (LIB, "    pub y: i32,\n}", "    pub y: i64,\n}"),
        ],
        HOOKS,
    );
    assert_ne!(holder(&with_point), holder(&with_wider_point));
    assert_ne!(shape(&base, "type:Point"), shape(&nested, "type:Point"));
    // A hand-written impl: its declared fields are its shape, its Rust fields are not.
    let ident = |e: &Export| shape(e, "type:Ident");
    let cache = export_fixture(
        &[(
            LIB,
            "    cache: u64,\n    pub namespace",
            "    cache: u32,\n    pub namespace",
        )],
        HOOKS,
    );
    assert_eq!(ident(&base), ident(&cache));
    let declared = export_fixture(
        &[(
            LIB,
            "layout_fields!(\"namespace\" => String);",
            "layout_fields!(\"namespace\" => String, \"path\" => String);",
        )],
        HOOKS,
    );
    assert_ne!(ident(&base), ident(&declared));
    // A type that crosses as another has that type's shape inside `via`, and one with no
    // declared layout has none.
    let types: BTreeMap<&str, &Type> = base
        .surface
        .types
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    let form = |id: &str| types[id].contract.wire_form.clone();
    assert_eq!(form("type:Name").as_deref(), Some("via:String"));
    assert_eq!(form("type:Ident").as_deref(), Some("fields"));
    assert_eq!(form("type:SlotKey").as_deref(), Some("fields"));
    assert_eq!(form("type:Context").as_deref(), Some("manual"));
    assert_eq!(form("type:Mode"), None);
    assert!(types["type:Context"].contract.wire && types["type:Context"].shape.is_none());
    assert!(types["type:Mode"].shape.is_none());
    // Renaming an unrelated type changes no other shape.
    let renamed = export_fixture(&[(LIB, "pub enum Mode {", "pub enum Toggle {")], HOOKS);
    assert_eq!(shape(&base, "type:Text"), shape(&renamed, "type:Text"));
    assert_eq!(shape(&base, "type:Point"), shape(&renamed, "type:Point"));
}

/// The real tree exports with unique ids and every catalog hook.
#[test]
#[ignore = "daily"]
fn the_real_tree_exports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let export = export(&root).unwrap();
    let text = export.surface.to_json();
    let read = Surface::read(text.as_bytes()).unwrap();
    let catalog = Catalog::load(&root).unwrap();
    let ids: BTreeSet<&str> = read.hooks.iter().map(|h| h.id.as_str()).collect();
    for hook in &catalog.hooks {
        assert!(
            ids.contains(format!("hook:{}", hook.name).as_str()),
            "{}",
            hook.name
        );
    }
    assert!(read.traits.iter().any(|t| t.id == "trait:Hooks"));
}

#[test]
fn a_derive_and_a_hand_written_impl_are_the_same_impl() {
    let point = |export: &Export| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Point")
            .cloned()
            .unwrap()
    };
    let before = point(&export_fixture(&[], HOOKS));
    let derived = point(&export_fixture(
        &[
            (
                LIB,
                "#[derive(Clone, Copy, Wire)]\npub struct Point",
                "#[derive(Clone, Copy, Default, Wire)]\npub struct Point",
            ),
            (
                LIB,
                "impl Default for Point {\n    fn default() -> Self {\n        Self { x: 0, y: 0 }\n    }\n}\n",
                "",
            ),
        ],
        HOOKS,
    ));
    assert_eq!(before.impls, ["Clone", "Copy", "Default"]);
    assert_eq!(derived.impls, before.impls);
    assert_eq!(derived.hash, before.hash);
}

#[test]
fn how_a_type_crosses_changes_its_shape_and_not_its_hash() {
    let text = |export: &Export| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Text")
            .cloned()
            .unwrap()
    };
    let before = text(&export_fixture(&[], HOOKS));
    // From a derive to crossing as a string.
    let via = text(&export_fixture(
        &[
            (
                LIB,
                "#[derive(Clone, Debug, Wire)]\npub struct Text",
                "#[derive(Clone, Debug)]\npub struct Text",
            ),
            (
                LIB,
                "/// A name that crosses as a string.",
                "impl Wire for Text {\n    fn layout(visiting: &mut Vec<&'static str>) -> u64 {\n        String::layout(visiting)\n    }\n}\n/// A name that crosses as a string.",
            ),
        ],
        HOOKS,
    ));
    assert_eq!(before.contract.wire_form.as_deref(), Some("derive"));
    assert_eq!(via.contract.wire_form.as_deref(), Some("via:String"));
    assert_eq!(via.hash, before.hash);
    assert!(before.shape.is_some() && via.shape.is_some());
    assert_ne!(via.shape, before.shape);
}

#[test]
fn a_hidden_public_field_is_part_of_the_field_set() {
    let point = |edits: &[(&str, &str, &str)]| {
        export_fixture(edits, HOOKS)
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Point")
            .cloned()
            .unwrap()
    };
    let before = point(&[]);
    let hidden = point(&[(
        LIB,
        "    pub y: i32,\n",
        "    pub y: i32,\n    #[doc(hidden)]\n    pub z: i32,\n",
    )]);
    assert_eq!(hidden.fields.len(), before.fields.len());
    assert_eq!(hidden.hash, before.hash);
    assert_ne!(hidden.fields_hash, before.fields_hash);
    // A private field changes neither, and makes the struct unbuildable.
    let private = point(&[(LIB, "    pub y: i32,\n", "    pub y: i32,\n    z: i32,\n")]);
    assert_eq!(private.hash, before.hash);
    assert_eq!(private.fields_hash, before.fields_hash);
    assert!(before.contract.constructible && !private.contract.constructible);
}

#[test]
fn a_listed_boundary_trait_an_older_tree_lacks_is_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (path, text) in FILES {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let catalog = Catalog::parse(&[("hooks/ping.rs".into(), HOOKS.into())]).unwrap();
    let mut index = Index::new(root, &[]);
    let mut listed = boundary();
    listed.roles.push(("FluidsHost".into(), Role::Call));
    let export = build(root, &mut index, &catalog.hooks, &listed).unwrap();
    assert!(
        export.notes.iter().any(|note| note
            == "boundary trait FluidsHost: not found in mod_api, its role is not recorded"),
        "{:?}",
        export.notes
    );
}

#[test]
fn a_hidden_type_named_only_by_an_impl_gets_its_record() {
    let secret = |edits: &[(&str, &str, &str)]| {
        export_fixture(edits, HOOKS)
            .surface
            .types
            .iter()
            .find(|t| t.id.ends_with("Secret"))
            .map(|t| t.id.clone())
    };
    assert_eq!(secret(&[]), None);
    let from = "impl From<internal::Secret> for Point {\n    fn from(_: internal::Secret) -> Self {\n        Self { x: 0, y: 0 }\n    }\n}\n/// A colour.";
    assert_eq!(
        secret(&[(LIB, "/// A colour.", from)]).as_deref(),
        Some("type:internal::Secret")
    );
}
