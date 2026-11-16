//! Every input a review of the fit test showed it answered wrongly, one row each, with what it
//! must answer now. Each row is a mod (its files), the change the API makes, and the answer:
//! refused, fits, or cannot judge. A new finding becomes a new row. Inputs a review gave at a
//! depth of 500 000 are written 50 000 deep: past the limit, any depth is the same input.
use crate::uses::*;
use crate::uses_tests::{
    DIMS_LITERAL, api_tuple, api_type, dims, entry, manifest, scan_files, settings, wider,
};
use crate::*;

/// What the API changes between the build the mod was made for and the target.
#[derive(Clone, Copy, Debug)]
enum TargetChange {
    /// `DimensionSettings` (a struct of three fields) gains a field.
    DimsField,
    /// `Pos` (a tuple struct of two fields) gains a field.
    PosField,
    /// `Settings` (a struct of one field) gains a field.
    SettingsField,
    /// The unit struct `Marker` becomes `#[non_exhaustive]`.
    MarkerSealed,
    /// `world::Biome` is no longer exported as `prelude::Biome`.
    BiomePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Refused,
    Fits,
    CannotJudge,
}
use Answer::*;
use TargetChange::*;

struct Row {
    /// The review that found it (the head it read) and the finding.
    finding: &'static str,
    files: Vec<(&'static str, String)>,
    change: TargetChange,
    answer: Answer,
}

fn marker(non_exhaustive: bool) -> Type {
    let record = api_type("type:Marker", "", true, vec![]);
    Type::new(
        record.id.clone(),
        "",
        None,
        TypeContract {
            kind: TypeKind::UnitStruct,
            constructible: !non_exhaustive,
            non_exhaustive,
            ..record.contract.clone()
        },
        vec![],
        vec![],
    )
}

fn biome(paths: &[&str]) -> Type {
    let mut record = api_type("type:world::Biome", "", false, vec![]);
    record.paths = paths.iter().map(|p| p.to_string()).collect();
    record
}

/// The API the mod was made for, or the target with `change` made.
fn surface(change: Option<TargetChange>) -> Surface {
    let mut pieces = wider();
    let mut dims = dims(if matches!(change, Some(DimsField)) {
        &["ultrawarm"]
    } else {
        &[]
    });
    dims.paths = vec![
        "DimensionSettings".into(),
        "world::DimensionSettings".into(),
    ];
    pieces.types.push(dims);
    pieces
        .types
        .push(marker(matches!(change, Some(MarkerSealed))));
    pieces
        .types
        .push(biome(if matches!(change, Some(BiomePath)) {
            &["world::Biome"]
        } else {
            &["world::Biome", "prelude::Biome"]
        }));
    match change {
        Some(PosField) => pieces.replace_type(api_tuple("type:Pos", &["i32", "i32", "i32"])),
        Some(SettingsField) => pieces.replace_type(settings(
            &[("height", "i32"), ("depth", "i32")],
            &["Clone", "Default"],
        )),
        _ => {}
    }
    pieces.build()
}

fn one(source: impl Into<String>) -> Vec<(&'static str, String)> {
    vec![("src/lib.rs", source.into())]
}

fn files(files: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    files.iter().map(|(p, t)| (*p, t.to_string())).collect()
}

fn row(
    finding: &'static str,
    files: Vec<(&'static str, String)>,
    change: TargetChange,
    answer: Answer,
) -> Row {
    Row {
        finding,
        files,
        change,
        answer,
    }
}

fn rows() -> Vec<Row> {
    let d = DIMS_LITERAL;
    let own = (
        "src/own.rs",
        "pub struct DimensionSettings { pub mine: u8 }",
    );
    let client = ("src/client/mod.rs", "mod world;");
    let declared = ("src/other/mod.rs", "mod world;\nmod api;");
    let written = format!("fn f() {{ let d = crate::world::{d}; }}");
    let imported = format!("use crate::world::DimensionSettings;\nfn f() {{ let d = {d}; }}");
    let in_macro = |body: &str| format!("macro_rules! m {{ () => {{ {body} }}; }}");
    let self_dims = format!("{{ {d} }}").replace("DimensionSettings {", "Self {");
    let widget = |text: &str| {
        format!(
            "pub struct Settings {{ pub mine: u8 }}\nfn g() -> Settings {{ Settings {{ mine: 1 }} }}\n{text}"
        )
    };
    let world_mine = (
        "src/world.rs",
        "pub struct DimensionSettings { pub mine: u8 }",
    );
    let built_mine = "use crate::world;\nfn f() { let d = world::DimensionSettings { mine: 1 }; }";
    let mut rows = vec![
        // A type alias of an API struct is no definition of the mod's own.
        row(
            "before 81c7cc0: a type alias of the API's struct",
            one(format!(
                "pub type DimensionSettings = mod_api::DimensionSettings;\nfn f() -> DimensionSettings {{ {d} }}"
            )),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a type alias imported from another file",
            vec![
                ("src/lib.rs", "mod types;\nmod world;".into()),
                (
                    "src/types.rs",
                    "pub type DimensionSettings = mod_api::DimensionSettings;".into(),
                ),
                (
                    "src/world.rs",
                    format!(
                        "use crate::types::DimensionSettings;\nfn f() -> DimensionSettings {{ {d} }}"
                    ),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a chain of aliases, another file holding one of the same name",
            vec![
                ("src/a.rs", "struct Mine { a: u8 }\ntype Api = Mine;".into()),
                (
                    "src/b.rs",
                    format!(
                        "type Api = mod_api::DimensionSettings;\npub type DimensionSettings = Api;\nfn f() -> DimensionSettings {{ {d} }}"
                    ),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a chain of aliases",
            one(format!(
                "type Api = mod_api::DimensionSettings;\npub type DimensionSettings = Api;\nfn f() -> DimensionSettings {{ {d} }}"
            )),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: an alias of a renamed import",
            one(format!(
                "use mod_api::DimensionSettings as Api;\npub type DimensionSettings = Api;\nfn f() -> DimensionSettings {{ {d} }}"
            )),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: an own alias imported through a path",
            files(&[
                ("src/lib.rs", "mod types;\nmod world;"),
                (
                    "src/types.rs",
                    "pub struct Mine { pub a: u8 }\npub type DimensionSettings = Mine;",
                ),
                (
                    "src/world.rs",
                    "use crate::types::DimensionSettings;\nfn f() { let d = DimensionSettings { a: 1 }; }",
                ),
            ]),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: an alias of the mod's own struct in its own file",
            one(
                "struct Mine { a: u8 }\ntype DimensionSettings = Mine;\nfn f() { let d = DimensionSettings { a: 1 }; }",
            ),
            DimsField,
            Fits,
        ),
        // A path inside the mod to an imported API struct.
        row(
            "before 81c7cc0: `use super::DimensionSettings` of the API's",
            vec![
                (own.0, own.1.into()),
                (
                    "src/world/mod.rs",
                    "use mod_api::DimensionSettings;\nmod gen;".into(),
                ),
                (
                    "src/world/gen.rs",
                    format!("use super::DimensionSettings;\nfn f() {{ let d = {d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: `super::DimensionSettings { … }` of the API's",
            vec![
                (own.0, own.1.into()),
                (
                    "src/world/mod.rs",
                    "use mod_api::DimensionSettings;\nmod gen;".into(),
                ),
                (
                    "src/world/gen.rs",
                    format!("fn f() {{ let d = super::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: `super::` under an API glob",
            vec![
                (own.0, own.1.into()),
                ("src/world/mod.rs", "use mod_api::*;\nmod gen;".into()),
                (
                    "src/world/gen.rs",
                    format!("fn f() {{ let d = super::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a path to an associated type of that name",
            vec![
                ("src/lib.rs", "mod a;\nmod b;".into()),
                (
                    "src/a.rs",
                    "struct Mine { a: u8 }\nimpl It for X { type DimensionSettings = Mine; }"
                        .into(),
                ),
                (
                    "src/b.rs",
                    format!("use crate::a::DimensionSettings;\nfn f() {{ let d = {d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: an import from a path that does not start inside the mod",
            vec![
                (own.0, own.1.into()),
                (
                    "src/b.rs",
                    format!("use world::DimensionSettings;\nfn f() {{ let d = {d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a literal written with a path to the mod's own struct",
            files(&[
                ("src/lib.rs", "mod own;\nmod world;"),
                own,
                (
                    "src/world/gen.rs",
                    "fn f() { let d = crate::own::DimensionSettings { mine: 1 }; }",
                ),
            ]),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: a literal through an own re-export beside a glob that does not export it",
            files(&[
                own,
                (
                    "src/world/mod.rs",
                    "use mod_api::items::*;\npub use crate::own::DimensionSettings;\nmod gen;",
                ),
                (
                    "src/world/gen.rs",
                    "fn f() { let d = super::DimensionSettings { mine: 1 }; }",
                ),
            ]),
            DimsField,
            Refused,
        ),
        // `Self` in an impl for the API's struct.
        row(
            "before 81c7cc0: `Self` in an impl for `mod_api::DimensionSettings` beside an own struct",
            one(format!(
                "struct DimensionSettings {{ mine: u8 }}\ntrait Make {{ fn make() -> Self; }}\nimpl Make for mod_api::DimensionSettings {{ fn make() -> Self {self_dims} }}"
            )),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: `Self` in an impl for an alias of the API's",
            one(format!(
                "struct DimensionSettings {{ mine: u8 }}\ntype Dims = mod_api::DimensionSettings;\ntrait Make {{ fn make() -> Self; }}\nimpl Make for Dims {{ fn make() -> Self {self_dims} }}"
            )),
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: `Self` in an impl for `super::DimensionSettings`",
            vec![
                (own.0, own.1.into()),
                (
                    "src/world/mod.rs",
                    "use mod_api::DimensionSettings;\nmod gen;".into(),
                ),
                (
                    "src/world/gen.rs",
                    format!(
                        "trait Make {{ fn make() -> Self; }}\nimpl Make for super::DimensionSettings {{ fn make() -> Self {self_dims} }}"
                    ),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "before 81c7cc0: `Self` in an impl for the file's own struct",
            one(
                "struct DimensionSettings { mine: u8 }\nimpl DimensionSettings { fn new() -> Self { Self { mine: 1 } } }",
            ),
            DimsField,
            Fits,
        ),
        // 81c7cc0, B1b: middle segments of a path inside the mod.
        row(
            "81c7cc0 B1b: `crate::world::DimensionSettings { … }` with `use mod_api::world`",
            vec![
                ("src/lib.rs", "use mod_api::world;\nmod terrain;".into()),
                (declared.0, declared.1.into()),
                (own.0, own.1.into()),
                ("src/terrain.rs", written.clone()),
            ],
            DimsField,
            Refused,
        ),
        row(
            "81c7cc0 B1b: `use crate::world::DimensionSettings` with `use mod_api::world`",
            vec![
                ("src/lib.rs", "use mod_api::world;\nmod terrain;".into()),
                (declared.0, declared.1.into()),
                (own.0, own.1.into()),
                ("src/terrain.rs", imported.clone()),
            ],
            DimsField,
            Refused,
        ),
        row(
            "81c7cc0 B1b: `crate::api::DimensionSettings` with `use mod_api as api`",
            vec![
                ("src/lib.rs", "use mod_api as api;\nmod terrain;".into()),
                (declared.0, declared.1.into()),
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ let d = crate::api::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "81c7cc0 B1b: an API glob that hands over the module `world`",
            vec![
                ("src/lib.rs", "pub use mod_api::*;\nmod terrain;".into()),
                (declared.0, declared.1.into()),
                (own.0, own.1.into()),
                ("src/terrain.rs", written.clone()),
            ],
            DimsField,
            Refused,
        ),
        row(
            "81c7cc0 B1b: `Self` in an impl for `crate::world::DimensionSettings`",
            vec![
                ("src/lib.rs", "use mod_api::world;\nmod terrain;".into()),
                (declared.0, declared.1.into()),
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!(
                        "trait Make {{ fn make() -> Self; }}\nimpl Make for crate::world::DimensionSettings {{ fn make() -> Self {self_dims} }}"
                    ),
                ),
            ],
            DimsField,
            Refused,
        ),
    ];
    // 2aad5d0, B1c: a segment bound by an import that does not record its name.
    for (finding, lib, terrain) in [
        (
            "2aad5d0 B1c: `use mod_api::world::{self}`, path written",
            "use mod_api::world::{self};\nmod terrain;",
            &written,
        ),
        (
            "2aad5d0 B1c: `use mod_api::world::{self}`, path imported",
            "use mod_api::world::{self};\nmod terrain;",
            &imported,
        ),
        (
            "2aad5d0 B1c: `pub use crate::api::world`",
            "use mod_api as api;\npub use crate::api::world;\nmod terrain;",
            &written,
        ),
        (
            "2aad5d0 B1c: `pub use self::api::world`",
            "use mod_api as api;\npub use self::api::world;\nmod terrain;",
            &written,
        ),
        (
            "2aad5d0 B1c: a glob through the renamed crate",
            "use mod_api as api;\nuse crate::api::*;\nmod terrain;",
            &written,
        ),
        // 593ebe7, B1d: a `use` the scan cannot see binds the segment.
        (
            "593ebe7 B1d: `pub use mod_api::$m` in a macro",
            "macro_rules! api { ($m:ident) => { pub use mod_api::$m; }; }\napi!(world);\nmod terrain;",
            &written,
        ),
        (
            "593ebe7 B1d: a `use` of a macro's tokens",
            "macro_rules! reexport { ($($p:tt)*) => { pub use $($p)*; }; }\nreexport!(mod_api::world);\nmod terrain;",
            &written,
        ),
        (
            "593ebe7 B1d: a dependency's macro at item level",
            "some_dep::bind!(world);\nmod terrain;",
            &written,
        ),
        (
            "593ebe7 B1d: a dependency's attribute",
            "#[some_dep::reexport(world)]\nmod terrain;",
            &written,
        ),
        (
            "593ebe7 B1d: an imported attribute",
            "#[reexport]\nmod terrain;",
            &written,
        ),
        (
            "593ebe7 B1d: a metavariable `use` called in a function body",
            "macro_rules! api { ($m:ident) => { use mod_api::$m; }; }\nfn g() { api!(world); }\nmod terrain;",
            &written,
        ),
    ] {
        rows.push(row(
            finding,
            vec![
                ("src/lib.rs", lib.into()),
                (client.0, client.1.into()),
                (own.0, own.1.into()),
                ("src/terrain.rs", terrain.clone()),
            ],
            DimsField,
            Refused,
        ));
    }
    rows.extend([
        row(
            "2aad5d0 B1c: a re-export by the mod's own module",
            vec![
                (
                    "src/lib.rs",
                    "use mod_api::world as w;\nmod shim;\nmod terrain;".into(),
                ),
                ("src/shim.rs", "pub use crate::w as world;".into()),
                (client.0, client.1.into()),
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ let d = crate::shim::world::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "593ebe7: a `pub(crate) use` chain across three files",
            vec![
                ("src/lib.rs", "mod a;\nmod b;\nmod c;\nmod terrain;".into()),
                ("src/a.rs", "pub(crate) use mod_api::world as w;".into()),
                ("src/b.rs", "pub(crate) use crate::a::w as x;".into()),
                ("src/c.rs", "pub(crate) use crate::b::x as world;".into()),
                (client.0, client.1.into()),
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ let d = crate::c::world::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "593ebe7: an extern crate renamed like a module",
            vec![
                (
                    "src/lib.rs",
                    "extern crate horizon_mod_api as world;\nmod terrain;".into(),
                ),
                (client.0, client.1.into()),
                (own.0, own.1.into()),
                ("src/terrain.rs", written.clone()),
            ],
            DimsField,
            Refused,
        ),
        row(
            "593ebe7: an own module named like the API crate",
            vec![
                ("src/lib.rs", "mod mod_api;\nmod terrain;".into()),
                (client.0, client.1.into()),
                ("src/client/mod_api.rs", String::new()),
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ let d = crate::mod_api::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "593ebe7: a segment the mod never declares",
            vec![
                (own.0, own.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ let d = crate::util::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "593ebe7: a module-local `use` beside an own module",
            vec![
                ("src/lib.rs", "mod world;\nmod terrain;".into()),
                (world_mine.0, world_mine.1.into()),
                (
                    "src/terrain.rs",
                    format!("fn f() {{ use mod_api::world; let d = world::{d}; }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
    ]);
    // Even the mod's own `world` module: a literal written with a path counts for the API's
    // struct of that name, whatever the path.
    for (finding, lib) in [
        ("593ebe7: the mod's own module", "mod world;\nmod terrain;"),
        (
            "593ebe7: the mod's own module under own globs",
            "mod world;\nmod terrain;\nuse crate::world::*;\nuse super::*;",
        ),
        (
            "593ebe7: a binding of the name to itself",
            "mod world;\nmod terrain;\nmod a;\npub use crate::world::world;",
        ),
        (
            "593ebe7: a cycle of bindings",
            "mod world;\nmod terrain;\nmod a;\nmod b;\npub use crate::a::world;\npub use crate::b::world as w2;\npub use crate::w2 as world;",
        ),
        (
            "e72e3c2: an own macro whose body writes a `use`",
            "macro_rules! inner { () => { use crate::world; }; }\nmacro_rules! outer { () => { inner!(); }; }\nouter!();\nmod world;\nmod terrain;",
        ),
        (
            "e72e3c2: standard macros and attributes only",
            "macro_rules! double { ($e:expr) => { $e * 2 }; }\nmacro_rules! quad { ($e:expr) => { double!(double!($e)) }; }\nquad!(1);\ncompile_error!(\"x\");\n#[derive(Clone)]\n#[cfg(test)]\n#[mod_api::horizon_mod]\nstruct S;\nfn f() { some_dep::bind!(world); }\nmod world;\nmod terrain;",
        ),
    ] {
        rows.push(row(
            finding,
            vec![
                ("src/lib.rs", lib.into()),
                (world_mine.0, world_mine.1.into()),
                ("src/terrain.rs", built_mine.into()),
            ],
            DimsField,
            Refused,
        ));
    }
    rows.push(row(
        "593ebe7: an own module this file neither binds nor declares",
        files(&[
            ("src/lib.rs", "mod world;\nmod terrain;"),
            world_mine,
            (
                "src/terrain.rs",
                "fn f() { let d = world::DimensionSettings { mine: 1 }; }",
            ),
        ]),
        DimsField,
        Refused,
    ));
    // e72e3c2: in a file anything may change, an unqualified name is the API's.
    for (finding, text) in [
        (
            "e72e3c2: a `use` of the name in a body",
            "fn h() { use mod_api::Settings; }",
        ),
        ("e72e3c2: a glob of the mod's", "use super::*;"),
        ("e72e3c2: an inline module", "mod inner {}"),
        (
            "e72e3c2: a `use` with a metavariable",
            "macro_rules! m { ($x:ident) => { use mod_api::$x; }; }",
        ),
        (
            "e72e3c2: a dependency's derive",
            "#[derive(bring::World)]\nstruct S;",
        ),
        (
            "e72e3c2: `cfg_attr` of a dependency's attribute",
            "#[cfg_attr(all(), bring::world)]\nstruct S;",
        ),
        (
            "e72e3c2: nested `cfg_attr`",
            "#[cfg_attr(all(), cfg_attr(all(), bring::world))]\nstruct S;",
        ),
        (
            "e72e3c2: `cfg_attr` of a dependency's derive",
            "#[cfg_attr(all(), derive(bring::World))]\nstruct S;",
        ),
        (
            "e72e3c2: an own macro writing a dependency's attribute",
            "macro_rules! wrap { () => { #[bring::world] struct S; }; }\nwrap!();",
        ),
        (
            "e72e3c2: an own macro writing an attribute it is given",
            "macro_rules! wrap { ($a:meta) => { #[$a] struct S; }; }",
        ),
        (
            "e72e3c2: a pass-through macro",
            "macro_rules! pass { ($($t:tt)*) => { $($t)* }; }\npass! { struct S; }",
        ),
        (
            "e72e3c2: a dependency's macro in a body",
            "fn h() { bring::api_world!(); }",
        ),
        (
            "e72e3c2: a dependency's attribute in a body",
            "fn h() { #[bring::world] fn g() {} }",
        ),
        (
            "e72e3c2: a call through a metavariable",
            "macro_rules! m { ($n:ident) => { $n!() }; }",
        ),
        ("e72e3c2: an inner attribute", "fn h() { #![bring::inner] }"),
        (
            "e72e3c2: an attribute the API does not have",
            "#[mod_api::not_an_api_macro]\nstruct S;",
        ),
    ] {
        rows.push(row(
            finding,
            vec![("src/widget.rs", widget(text))],
            SettingsField,
            Refused,
        ));
    }
    rows.push(row(
        "e72e3c2: the examples mod's own struct, standard derives and macros",
        vec![(
            "src/widget.rs",
            widget(
                "#[derive(Default, Clone)]\n#[mod_api::horizon_mod]\nimpl Settings { fn new() -> Self { Self { mine: format!(\"x\").len() as u8 } } }\n#[cfg_attr(test, derive(Debug))]\n#[cfg_attr(feature = \"x\", derive(Debug))]\nstruct T;\nfn k(x: bool) -> bool { if !x { return !(x); } while !x {} x }",
            ),
        )],
        SettingsField,
        Fits,
    ));
    let trusted = "pub struct Settings { pub mine: u8 }\n#[mod_api::horizon_mod]\n#[horizon_mod]\nstruct T;\nfn g() -> Settings { let _ = vec![1]; Settings { mine: 1 } }";
    for (finding, elsewhere) in [
        (
            "e72e3c2: an own `macro_rules!` named like a standard macro",
            "macro_rules! vec { ($m:ident) => { pub mod $m {} }; }",
        ),
        (
            "e72e3c2: a dependency's macro renamed to a standard name",
            "use bring::api_world as vec;",
        ),
        (
            "e72e3c2: a dependency's attribute under an API attribute's name",
            "use bring::world_attr as horizon_mod;",
        ),
        ("e72e3c2: a dependency's glob", "use bring::*;"),
        (
            "e72e3c2: `#[macro_use]`",
            "#[macro_use]\nextern crate bring;",
        ),
    ] {
        rows.push(row(
            finding,
            files(&[("src/lib.rs", elsewhere), ("src/widget.rs", trusted)]),
            SettingsField,
            Refused,
        ));
    }
    rows.push(row(
        "e72e3c2: nothing rebinds the standard and API names",
        files(&[("src/lib.rs", "mod widget;"), ("src/widget.rs", trusted)]),
        SettingsField,
        Fits,
    ));
    rows.extend([
        // e72e3c2: values and paths the scan missed.
        row("e72e3c2: `p < Pos(1, 2)`", one("fn f(p: Pos) -> bool { p < Pos(1, 2) }"), PosField, Refused),
        row("e72e3c2: `if Pos(1, 2) == p`", one("fn f(p: Pos) { if Pos(1, 2) == p {} }"), PosField, Refused),
        row("e72e3c2: `match Pos(1, 2)`", one("fn f() { match Pos(1, 2) { _ => {} } }"), PosField, Refused),
        row(
            "e72e3c2: a constructor as a block's value",
            one("fn ctor() -> fn(i32, i32) -> Pos { Pos }"),
            PosField,
            Refused,
        ),
        row(
            "e72e3c2: a unit struct as a value",
            one("fn f() { let m = mod_api::Marker; }"),
            MarkerSealed,
            Refused,
        ),
        row(
            "e72e3c2: two files rename two API structs to one name",
            files(&[
                ("src/a.rs", "use mod_api::Settings as S;\nfn f() { let s = S { height: 1 }; }"),
                ("src/b.rs", "use mod_api::ItemStack as S;"),
            ]),
            SettingsField,
            Refused,
        ),
        row(
            "e72e3c2: an API struct renamed to another API struct's name",
            one("use mod_api::Settings as ItemStack;\nfn f() { let s = ItemStack { height: 1 }; }"),
            SettingsField,
            Refused,
        ),
        row(
            "e72e3c2: an alias of an alias",
            one("type A = mod_api::Settings;\ntype B = A;\nfn f() { let s = B { height: 1 }; }"),
            SettingsField,
            Refused,
        ),
        row(
            "e72e3c2: a path through `use mod_api::prelude as p`",
            one("use mod_api::prelude as p;\nfn f(_: p::Biome) {}"),
            BiomePath,
            Refused,
        ),
        row(
            "e72e3c2: a path through `use mod_api::prelude`",
            one("use mod_api::prelude;\nfn f(_: prelude::Biome) {}"),
            BiomePath,
            Refused,
        ),
        // 3f3e8ce: an expansion the scan cannot follow.
        row(
            "3f3e8ce: a dependency's macro given the struct's name",
            one("use mod_api::DimensionSettings;\nfn f() -> DimensionSettings { dep::make!(DimensionSettings, 0, 256) }"),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: a dependency's derive on a struct holding it",
            one("#[derive(dep::Make)]\nstruct Holder { d: mod_api::DimensionSettings }"),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: a dependency's attribute on a function returning it",
            one("#[dep::build]\nfn f() -> mod_api::DimensionSettings { todo!() }"),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: named only with standard macros and attributes",
            one("#[derive(Debug)]\nstruct Holder { d: mod_api::DimensionSettings }\nfn f(d: mod_api::DimensionSettings) { println!(\"{}\", d.height); }"),
            DimsField,
            Fits,
        ),
        row(
            "3f3e8ce: a dependency's macro of the mod's own macro's name",
            files(&[
                ("src/macros.rs", "macro_rules! make { ($t:ident) => {}; }"),
                ("src/world.rs", "use dep::make;\nfn f(_: DimensionSettings) { make!(DimensionSettings); }"),
            ]),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: the mod's own macro",
            files(&[
                ("src/macros.rs", "macro_rules! make { ($t:ident) => {}; }"),
                ("src/world.rs", "fn f(_: DimensionSettings) { make!(DimensionSettings); }"),
            ]),
            DimsField,
            Fits,
        ),
        // 3f3e8ce: literals the scan read as types.
        row(
            "3f3e8ce: a path from the root",
            one("fn f() { let d = ::mod_api::world::DimensionSettings { min_y: 0, height: 256, logical_height: 256 }; }"),
            DimsField,
            Refused,
        ),
        row("3f3e8ce: `let r#type = …`", one(format!("fn f() {{ let r#type = {d}; }}")), DimsField, Refused),
        row(
            "3f3e8ce: `self.r#type = …`",
            one(format!("struct S {{ r#type: mod_api::DimensionSettings }}\nimpl S {{ fn f(&mut self) {{ self.r#type = {d}; }} }}")),
            DimsField,
            Refused,
        ),
        row("3f3e8ce: a method named `union`", one(format!("fn f(bounds: B) {{ bounds.union({d}); }}")), DimsField, Refused),
        row(
            "3f3e8ce: a variable named `union`",
            one("fn f(union: &mut Vec<Pos>) { union.push(Pos(1, 2)); }"),
            PosField,
            Refused,
        ),
        row("3f3e8ce: `ORIGIN < Pos(1, 2)`", one("fn f() -> bool { ORIGIN < Pos(1, 2) }"), PosField, Refused),
        row("3f3e8ce: `g(MIN < MAX, Pos(1, 2))`", one("fn f() { g(MIN < MAX, Pos(1, 2)); }"), PosField, Refused),
        // 3f3e8ce: definitions that are not the mod's own.
        row(
            "3f3e8ce: a definition under `#[cfg]` in an included file",
            vec![
                ("src/lib.rs", "use mod_api::world::DimensionSettings;\ninclude!(\"body.rs\");".into()),
                (
                    "src/body.rs",
                    format!("#[cfg(any())]\nstruct DimensionSettings {{ pub mine: u8 }}\nfn f() -> DimensionSettings {{ {d} }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: a definition under `#[cfg]`",
            one(format!("#[cfg(any())]\nstruct DimensionSettings {{ pub mine: u8 }}\nfn f() -> DimensionSettings {{ {d} }}")),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: an included file under a dependency's attribute",
            vec![
                ("src/lib.rs", "#[dep::wrap]\nfn g() { include!(\"body.rs\"); }".into()),
                (
                    "src/body.rs",
                    format!("struct DimensionSettings {{ pub mine: u8 }}\nfn f() -> DimensionSettings {{ {d} }}"),
                ),
            ],
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: a definition in a macro body",
            one(format!("macro_rules! m ( () => ( struct DimensionSettings; ) );\nfn f() -> DimensionSettings {{ {d} }}")),
            DimsField,
            Refused,
        ),
        row(
            "3f3e8ce: a plain definition of the mod's own",
            one("struct DimensionSettings { pub mine: u8 }\nfn f(d: DimensionSettings) -> DimensionSettings { DimensionSettings { mine: d.mine } }"),
            DimsField,
            Fits,
        ),
        row(
            "3f3e8ce: a plain definition beside a literal the scan cannot place",
            files(&[
                ("src/lib.rs", "struct DimensionSettings { pub mine: u8 }\nfn f(d: DimensionSettings) -> DimensionSettings { DimensionSettings { mine: d.mine } }"),
                ("src/m.rs", "macro_rules! m { () => { Wrapper { a: 1 } }; }"),
            ]),
            DimsField,
            Fits,
        ),
        // 3f3e8ce: paths under a glob or a re-export.
        row(
            "3f3e8ce: `prelude::Biome` under a root glob",
            one("use mod_api::*;\nfn f(_: prelude::Biome) {}"),
            BiomePath,
            Refused,
        ),
        row(
            "3f3e8ce: `crate::prelude::Biome` through the mod's re-export",
            files(&[
                ("src/lib.rs", "pub use mod_api::prelude;\nmod world;"),
                ("src/world.rs", "fn f(_: crate::prelude::Biome) {}"),
            ]),
            BiomePath,
            Refused,
        ),
        // a443e32 (round 6).
        row(
            "a443e32 B1: 500 000 closures of two parameters",
            one(format!("fn f() {{ let g = {}0; }}", "|a, b| ".repeat(50_000))),
            DimsField,
            CannotJudge,
        ),
        row(
            "a443e32 B2: a constructor passed to `map` in a macro",
            one("use mod_api::Pos;\npub fn ids(raw: Option<(i32, i32)>) -> Vec<Option<Pos>> { vec![raw.map(|(a, b)| Pos(a, b)), None.map(Pos)] }"),
            PosField,
            Refused,
        ),
        row(
            "a443e32 B2: a constructor compared in `assert_eq!`",
            one("fn f(p: mod_api::Pos) { assert_eq!(Some(p), Some(1).map(mod_api::Pos)); }"),
            PosField,
            Refused,
        ),
        row(
            "a443e32 B2: constructors in `vec!`",
            one("fn f() { let v = vec![mod_api::Pos, mod_api::Pos]; }"),
            PosField,
            Refused,
        ),
        row(
            "a443e32 B3: `Self` in an impl of a parenthesised type",
            one(format!("trait Make {{ fn make() -> Self; }}\nimpl Make for (mod_api::world::DimensionSettings) {{ fn make() -> Self {self_dims} }}")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B3: `Self` in an impl a macro writes",
            one(format!("trait Make {{ fn make() -> Self; }}\nmacro_rules! make {{ () => {{ impl Make for mod_api::world::DimensionSettings where u8: Copy {{ fn make() -> Self {self_dims} }} }}; }}\nmake!();")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B3: `Self` in an impl of a macro's type",
            one(format!("macro_rules! dims {{ () => {{ mod_api::world::DimensionSettings }}; }}\ntrait Make {{ fn make() -> Self; }}\nimpl Make for dims!() {{ fn make() -> Self {self_dims} }}")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: an alias of a parenthesised type",
            one(format!("type Dims = (mod_api::world::DimensionSettings);\npub fn d() -> Dims {{ {} }}", d.replace("DimensionSettings", "Dims"))),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: an alias of a macro's type",
            one(format!("macro_rules! dims_ty {{ () => {{ mod_api::world::DimensionSettings }}; }}\ntype Dims = dims_ty!();\npub fn d() -> Dims {{ {} }}", d.replace("DimensionSettings", "Dims"))),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: an alias of a macro's type defining the name",
            one(format!("macro_rules! dims {{ () => {{ mod_api::world::DimensionSettings }}; }}\ntype DimensionSettings = dims!();\nfn f() -> DimensionSettings {{ {d} }}")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: an alias of a macro's type in a macro body",
            one(format!("macro_rules! dims_ty {{ () => {{ mod_api::world::DimensionSettings }}; }}\nmacro_rules! m {{ () => {{ type Dims = dims_ty!(); pub fn d() -> Dims {{ {} }} }}; }}\nm!();", d.replace("DimensionSettings", "Dims"))),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: an alias of a projection",
            one(format!("trait Id {{ type This; }}\nimpl<T> Id for T {{ type This = T; }}\ntype DimensionSettings = <mod_api::world::DimensionSettings as Id>::This;\nfn f() -> DimensionSettings {{ {d} }}")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 B4: a base literal through parentheses",
            one("trait Make { fn make() -> Self; }\nimpl Make for (mod_api::world::DimensionSettings) { fn make() -> Self { Self { min_y: 0, ..Default::default() } } }\ntype Dims = (mod_api::world::DimensionSettings);\nfn d() -> Dims { Dims { height: 1, ..Default::default() } }"),
            DimsField,
            Fits,
        ),
        row(
            "a443e32 B5: `crate::Biome` through the mod's own glob",
            files(&[
                ("src/lib.rs", "pub use mod_api::prelude::*;\nmod world;"),
                ("src/world.rs", "pub fn f(_: crate::Biome) {}"),
            ]),
            BiomePath,
            Refused,
        ),
        row(
            "a443e32 B5: `use super::*` of the mod's own glob",
            files(&[
                ("src/lib.rs", "pub use mod_api::prelude::*;\nmod world;"),
                ("src/world.rs", "use super::*;\npub fn f(_: Biome) {}"),
            ]),
            BiomePath,
            Refused,
        ),
        row(
            "a443e32 B6: `include!` under another name",
            one(format!("use std::include as inc;\nstruct DimensionSettings {{ pub mine: u8 }}\nfn f() -> DimensionSettings {{ {d} }}")),
            DimsField,
            Refused,
        ),
        row(
            "a443e32 S2: a table of shifts",
            one(format!(
                "const M: [u128; 1000] = [{}];\nfn f(_: mod_api::DimensionSettings) {{}}",
                (0..1000).map(|i| format!("1 << {i}")).collect::<Vec<_>>().join(", ")
            )),
            DimsField,
            Fits,
        ),
        row(
            "a443e32 S2: a list of closures",
            one(format!(
                "fn f(_: mod_api::DimensionSettings) {{ let c = [{}]; }}",
                "|x: u8| x, ".repeat(1000)
            )),
            DimsField,
            Fits,
        ),
        row(
            "a443e32 S2: a list of negative numbers",
            one(format!(
                "fn f(_: mod_api::DimensionSettings) {{ let x = [{}]; }}",
                "-1, ".repeat(10_000)
            )),
            DimsField,
            Fits,
        ),
        row(
            "a443e32 S2: a match of guarded arms",
            one(format!(
                "fn g(v: u32) -> u32 {{ match v {{ {} _ => 0 }} }}\nfn f(_: mod_api::DimensionSettings) {{}}",
                (0..1000).map(|i| format!("v if v < {i} => {i},")).collect::<Vec<_>>().join(" ")
            )),
            DimsField,
            Fits,
        ),
        // bc51218 (round 7).
        row(
            "bc51218 B1: qualified paths after a comparison",
            one(format!("fn f() {{ let x = 1 < {}T{}; }}", "<".repeat(50_000), " as A>::B".repeat(50_000))),
            DimsField,
            CannotJudge,
        ),
        row(
            "bc51218 B1: qualified paths after a call",
            one(format!("fn f() {{ let x = f() < {}T{}; }}", "<".repeat(50_000), " as A>::B".repeat(50_000))),
            DimsField,
            CannotJudge,
        ),
        row(
            "bc51218 B2: a constructor as a field's value in a macro",
            one("use mod_api::Pos;\npub struct Maker { pub make: fn(i32, i32) -> Pos }\npub fn makers() -> Vec<Maker> { vec![Maker { make: Pos }] }"),
            PosField,
            Refused,
        ),
        row(
            "bc51218 B2: a constructor as a block's value in a macro",
            one("use mod_api::Pos;\nfn f(c: bool, other: fn(i32, i32) -> Pos) { let v = vec![if c { Pos } else { other }]; }"),
            PosField,
            Refused,
        ),
        row(
            "bc51218 B2: a unit struct as a closure's value in a macro",
            one("use mod_api::Marker;\nfn f() { let v = vec![(0..4).map(|_| Marker).count()]; }"),
            MarkerSealed,
            Refused,
        ),
        row(
            "bc51218 B3: `Self` in a macro's impl inside another impl's method",
            one(format!("trait Make {{ fn make() -> Self; }}\nstruct Wrap;\nimpl Wrap {{ fn setup() {{ macro_rules! make {{ () => {{ impl Make for mod_api::world::DimensionSettings where u8: Copy {{ fn make() -> Self {self_dims} }} }}; }} make!(); }} }}")),
            DimsField,
            Refused,
        ),
        row(
            "bc51218 B5: a relative glob of the mod's own module",
            files(&[
                ("src/lib.rs", "mod api;\nuse api::*;\npub fn f(_: Biome) {}"),
                ("src/api.rs", "pub use mod_api::prelude::*;"),
            ]),
            BiomePath,
            Refused,
        ),
        row(
            "bc51218 B5: a path through the mod's own module",
            files(&[
                ("src/lib.rs", "mod api;\npub fn f(_: api::Biome) {}"),
                ("src/api.rs", "pub use mod_api::prelude::*;"),
            ]),
            BiomePath,
            Refused,
        ),
        row(
            "bc51218 S1: prefix operators on method calls",
            one(format!("fn f() {{ let x = {}y{}; }}", format!("{}x.f(", "-".repeat(128)).repeat(128), ")".repeat(128))),
            DimsField,
            CannotJudge,
        ),
        row(
            "bc51218 S1: a sum of dereferenced elements",
            one(format!(
                "fn g(v: &[&u8]) -> u8 {{ {} }}\nfn f(_: mod_api::DimensionSettings) {{}}",
                (0..1000).map(|i| format!("*v[{i}]")).collect::<Vec<_>>().join(" + ")
            )),
            DimsField,
            Fits,
        ),
        row(
            "bc51218 S1: a chain of negated groups",
            one(format!(
                "fn g() -> bool {{ {} }}\nfn f(_: mod_api::DimensionSettings) {{}}",
                (0..1000).map(|i| format!("!(a{i})")).collect::<Vec<_>>().join(" && ")
            )),
            DimsField,
            Fits,
        ),
        // 6db10aa (round 8).
        row(
            "6db10aa B1: a constructor compared in `assert!`",
            one("use mod_api::Pos;\npub fn check(p: Pos) { assert!(p < Pos(10, 0)); }"),
            PosField,
            Refused,
        ),
        row(
            "6db10aa B1: a tuple struct pattern after `for` in a macro",
            one("use mod_api::Pos;\nmacro_rules! sum { ($v:expr) => {{ let mut s = 0; for Pos(x, y) in $v { s += x + y; } s }}; }\npub fn total(v: Vec<Pos>) -> i32 { sum!(v) }"),
            PosField,
            Refused,
        ),
        row(
            "6db10aa B1: a struct pattern after `for` in a macro",
            one(in_macro("for mod_api::DimensionSettings { min_y, height, logical_height } in v {}")),
            DimsField,
            Refused,
        ),
        row(
            "6db10aa B2: a bare `Self` in an impl a macro writes",
            one("trait Make { fn make() -> Self; }\nmacro_rules! make { () => { impl Make for mod_api::Marker { fn make() -> Self { Self } } }; }\nmake!();"),
            MarkerSealed,
            Refused,
        ),
        row(
            "6db10aa B2: `Self` as a constructor in an impl a macro writes",
            one("trait Ctor { fn ctor() -> fn(i32, i32) -> Self; }\nmacro_rules! make { () => { impl Ctor for mod_api::Pos { fn ctor() -> fn(i32, i32) -> Self { Self } } }; }\nmake!();"),
            PosField,
            Refused,
        ),
        row(
            "6db10aa B2: `Self` passed as a constructor in an impl a macro writes",
            one("trait Make { fn make() -> mod_api::Pos; }\nfn build<T>(f: fn(i32, i32) -> T) -> T { f(1, 2) }\nmacro_rules! make { () => { impl Make for mod_api::Pos { fn make() -> mod_api::Pos { build(Self) } } }; }\nmake!();"),
            PosField,
            Refused,
        ),
        row(
            "6db10aa S1: a glob of the mod's own module trusts standard macros",
            files(&[
                ("src/lib.rs", "mod util;\nuse util::*;\npub fn h(d: &mod_api::world::DimensionSettings) -> i32 { println!(\"{}\", d.height); d.min_y }"),
                ("src/util.rs", "pub fn helper() {}"),
            ]),
            DimsField,
            Fits,
        ),
        row(
            "6db10aa S2: a parameter's type in a macro",
            one(in_macro("fn on(p: mod_api::Pos) -> i32 { p.0 }")),
            PosField,
            Fits,
        ),
        row(
            "6db10aa S2: a closure parameter's type behind `&` in a macro",
            one(in_macro("let f = |p: &mod_api::Pos| p.0;")),
            PosField,
            Fits,
        ),
        row(
            "6db10aa S2: a `let` type in a macro",
            one(in_macro("let mut p: mod_api::Pos = make();")),
            PosField,
            Fits,
        ),
        row(
            "6db10aa S2: a field's type in a macro",
            one(in_macro("struct Row { id: mod_api::Pos, other: &'static mod_api::Pos }")),
            PosField,
            Fits,
        ),
        // f4e12ec (round 9).
        row(
            "f4e12ec B1: a constructor passed by key to a macro",
            one("use mod_api::Pos;\npub struct Maker { pub make: fn(i32, i32) -> Pos }\nmacro_rules! maker { (make: $e:expr) => { Maker { make: $e } }; }\npub fn m() -> Maker { maker!(make: Pos) }"),
            PosField,
            Refused,
        ),
        row(
            "f4e12ec B1: `Self` passed by key to a macro in a parsed impl",
            one("use mod_api::Pos;\npub struct Maker { pub make: fn(i32, i32) -> Pos }\nmacro_rules! maker { (make: $e:expr) => { Maker { make: $e } }; }\ntrait Make { fn m() -> Maker; }\nimpl Make for mod_api::Pos { fn m() -> Maker { maker!(make: Self) } }"),
            PosField,
            Refused,
        ),
        row(
            "f4e12ec: a generic function's parameter type in a macro",
            one(in_macro("fn on<T: Into<i32>>(p: mod_api::Pos, t: T) -> i32 { p.0 + t.into() }")),
            PosField,
            Fits,
        ),
        row(
            "bc51218 note: `..` in a macro body's parentheses is a range",
            one(in_macro("let p = mod_api::Pos(0, ..);")),
            PosField,
            Refused,
        ),
    ]);
    rows
}

#[test]
fn every_review_finding_answers_as_it_should() {
    let made_for = surface(None);
    for Row {
        finding,
        files,
        change,
        answer,
    } in rows()
    {
        let sources = files.iter().map(|(p, t)| (*p, t.as_str()));
        let got = match uses(sources, &manifest(&["registries"]), &made_for) {
            Err(_) => CannotJudge,
            Ok(uses) => match fits(uses.entries(), &surface(Some(change))) {
                Fit::Fits => Fits,
                Fit::DoesNotFit(_) => Refused,
            },
        };
        assert_eq!(got, answer, "{finding}");
    }
}

#[test]
fn a_struct_no_code_outside_the_api_can_build_is_not_built_by_an_expansion() {
    let mut sealed = wider();
    let record = settings(&[("height", "i32")], &["Clone"]);
    sealed.replace_type(Type::new(
        record.id.clone(),
        "",
        None,
        TypeContract {
            constructible: false,
            ..record.contract.clone()
        },
        record.fields.clone(),
        vec![],
    ));
    let hidden = scan_files(
        sealed,
        &[
            ("src/lib.rs", "#[dep::build]\nfn f(s: Settings) {}"),
            ("src/m.rs", "macro_rules! m { () => { Wrapper { a: 1 } }; }"),
        ],
    );
    assert_eq!(entry(&hidden, "type:Settings").built, Some(Built::None));
}
