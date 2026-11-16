//! `[[native.mixin]]`: a source-level edit of a game function, the only form that compiles game code.
//! Each row is one horizon-mixins `[[injection]]`, forwarded to the pinned tool, which owns the
//! vocabulary beyond the keys named here. Native only.
use serde::{Deserialize, Serialize};
use std::fmt;

/// One `[[native.mixin]]` row. `options` holds every other key of the tool's `[[injection]]`
/// (`selector`, `ordinal`, `bindings`, `cancellable`, `require`, `slice`, `variable`...), which
/// the tool validates when the game is built.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Mixin {
    pub id: String,
    /// The game function's path, as the pinned source spells it.
    pub target: String,
    pub kind: Kind,
    /// The helper's source file in this package, compiled inside the target function.
    pub helper: String,
    #[serde(flatten)]
    pub options: toml::Table,
}

/// A horizon-mixins table forwarded whole (`[[native.bridge]]`, `[[native.state]]`); only its
/// id is the package's to check.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Forwarded {
    pub id: String,
    #[serde(flatten)]
    pub fields: toml::Table,
}

/// horizon-mixins' injection kinds, spelled in upper case in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Head,
    Return,
    BeforeCall,
    AfterCall,
    AfterAssign,
    ModifyArg,
    ModifyArgs,
    ModifyConstant,
    ModifyVariable,
    ModifyExpressionValue,
    WrapOperation,
    Overwrite,
}

impl Kind {
    pub const ALL: [Self; 12] = [
        Self::Head,
        Self::Return,
        Self::BeforeCall,
        Self::AfterCall,
        Self::AfterAssign,
        Self::ModifyArg,
        Self::ModifyArgs,
        Self::ModifyConstant,
        Self::ModifyVariable,
        Self::ModifyExpressionValue,
        Self::WrapOperation,
        Self::Overwrite,
    ];

    /// The manifest spelling, which is the tool's.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "HEAD",
            Self::Return => "RETURN",
            Self::BeforeCall => "BEFORE_CALL",
            Self::AfterCall => "AFTER_CALL",
            Self::AfterAssign => "AFTER_ASSIGN",
            Self::ModifyArg => "MODIFY_ARG",
            Self::ModifyArgs => "MODIFY_ARGS",
            Self::ModifyConstant => "MODIFY_CONSTANT",
            Self::ModifyVariable => "MODIFY_VARIABLE",
            Self::ModifyExpressionValue => "MODIFY_EXPRESSION_VALUE",
            Self::WrapOperation => "WRAP_OPERATION",
            Self::Overwrite => "OVERWRITE",
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = std::borrow::Cow::<str>::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown mixin kind `{name}`, expected one of {}",
                    Self::ALL.map(Self::name).join(", ")
                ))
            })
    }
}

/// Why a package's mixins are refused before any tool runs; `id` is `package:mixin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixinError {
    Invalid {
        id: String,
        field: &'static str,
        fix: &'static str,
    },
}

impl fmt::Display for MixinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { id, field, fix } => {
                write!(f, "E_MIXIN_INVALID: {id}: invalid {field}; {fix}")
            }
        }
    }
}
impl std::error::Error for MixinError {}

impl Mixin {
    pub(super) fn check_all(entries: &[Self], mod_id: &str) -> Result<(), MixinError> {
        for entry in entries {
            let id = format!("{mod_id}:{}", entry.id);
            let invalid = |field, fix| MixinError::Invalid {
                id: id.clone(),
                field,
                fix,
            };
            if entry.target.trim().is_empty() {
                return Err(invalid("target", "name the game function's path"));
            }
            if !super::plain_path(&entry.helper)
                || !entry.helper.starts_with("src/")
                || !entry.helper.ends_with(".rs")
            {
                return Err(invalid(
                    "helper",
                    "name a Rust file under src/ in this package",
                ));
            }
            if entry.options.contains_key("hook") || entry.options.contains_key("hook_version") {
                return Err(invalid(
                    "hook",
                    "a named hook is a mod_api::Hooks method; a mixin names its target",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::Manifest;
    use super::*;

    const HEADER: &str =
        "id='sample'\nname='Sample'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n";
    const ROW: &str = "[[native.mixin]]\nid='autosave'\ntarget='game::Game::next_autosave'\nkind='MODIFY_EXPRESSION_VALUE'\nhelper='src/longer.rs'\n";

    fn native() -> String {
        format!(
            "[native]\ngame='26.3'\nhorizon='{0}'\nmixins='{0}'\n",
            "a".repeat(40)
        )
    }

    fn parse(code: &str, rows: &str) -> Result<Manifest, String> {
        Manifest::parse(&format!("{HEADER}{code}{rows}"))
    }

    #[test]
    fn top_level_mixin_is_unknown() {
        assert!(
            Manifest::parse(&format!("{HEADER}[[mixin]]\nid='a'\n"))
                .unwrap_err()
                .contains("unknown field `mixin`")
        );
    }

    #[test]
    fn the_tools_keys_pass_through_and_the_packages_are_checked() {
        let row = format!(
            "{ROW}selector='self.interval'\nordinal=1\nrequire=1\ncancellable=true\npriority=5\nbindings=[{{name='n',ty='u32',value='n'}}]\nslice={{from={{selector='a',ordinal=0,require=1}}}}\nvariable={{name='n',ordinal=0,at='STORE'}}\n"
        );
        let manifest = parse(&native(), &row).unwrap();
        let mixin = &manifest.native.as_ref().unwrap().mixin[0];
        assert_eq!(mixin.kind, Kind::ModifyExpressionValue);
        // What the tool reads back is the row as written.
        let forwarded: toml::Table = toml::from_str(&toml::to_string(mixin).unwrap()).unwrap();
        let written: toml::Table = toml::from_str(&row).unwrap();
        assert_eq!(
            toml::Value::Table(forwarded),
            written["native"]["mixin"].as_array().unwrap()[0]
        );
        for (old, new, field) in [
            ("target='game::Game::next_autosave'", "target=' '", "target"),
            ("helper='src/longer.rs'", "helper='../longer.rs'", "helper"),
            ("helper='src/longer.rs'", "helper='/etc/passwd'", "helper"),
            (
                "helper='src/longer.rs'",
                "helper='src/longer.toml'",
                "helper",
            ),
            ("helper='src/longer.rs'", "helper='longer.rs'", "helper"),
            ("kind=", "hook='tick'\nkind=", "hook"),
            ("kind=", "hook_version=1\nkind=", "hook"),
        ] {
            let error = parse(&native(), &ROW.replace(old, new)).unwrap_err();
            assert!(
                error.contains(&format!(
                    "E_MIXIN_INVALID: sample:autosave: invalid {field};"
                )),
                "{error}"
            );
        }
        for missing in [
            "id='autosave'\n",
            "target='game::Game::next_autosave'\n",
            "helper='src/longer.rs'\n",
        ] {
            assert!(
                parse(&native(), &ROW.replace(missing, "")).is_err(),
                "{missing}"
            );
        }
    }

    #[test]
    fn a_mixin_never_targets_another_mods_function() {
        let row = ROW.replace("game::Game::next_autosave", "chem_lib::transfer");
        let dependency = "[dependencies]\nchem_lib='^1.0'\n";
        // Without the dependency the path is a game path for the tool to resolve.
        assert!(parse(&native(), &row).is_ok());
        let error = parse(&format!("{dependency}{}", native()), &row).unwrap_err();
        assert!(
            error.ends_with(
                "E_MIXIN_INVALID: sample:autosave: invalid target; a mixin targets a game function"
            ),
            "{error}"
        );
    }

    #[test]
    fn hostile_shapes_are_refused_without_panicking() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
            "a".repeat(128)
        );
        let shapes = [
            // a kind that is no string, or an empty one
            ROW.replace("kind='MODIFY_EXPRESSION_VALUE'", "kind=3"),
            ROW.replace("kind='MODIFY_EXPRESSION_VALUE'", "kind=''"),
            // tables where arrays of tables belong, and the reverse
            ROW.replace("[[native.mixin]]", "[native.mixin]"),
            "[[native]]\ngame='26.3'\n".into(),
            "[[native.bridge]]\nid=7\n".into(),
            "[[native.state]]\nid=['a']\n".into(),
            // keys of the wrong type, blank ids, and ids that are not ASCII
            ROW.replace("helper='src/longer.rs'", "helper=1"),
            ROW.replace("helper='src/longer.rs'", "helper={path='src/longer.rs'}"),
            ROW.replace("target='game::Game::next_autosave'", "target=[]"),
            ROW.replace("id='autosave'", "id=' '"),
            ROW.replace("id='autosave'", "id=''"),
            "[[native.bridge]]\nid=' '\n".into(),
            "[[native.state]]\nid='\t'\n".into(),
            ROW.replace("helper='src/longer.rs'", "helper='src/\u{0}.rs\\..'"),
            // a duplicate key
            format!("{ROW}kind='HEAD'\n"),
        ];
        for shape in &shapes {
            for code in [native(), wasm.clone(), String::new()] {
                assert!(parse(&code, shape).is_err(), "{shape}{code}");
            }
        }
        // Unicode in an id is the tool's to judge, and is carried as written.
        let manifest = parse(&native(), &ROW.replace("id='autosave'", "id='épée-✓'")).unwrap();
        assert_eq!(manifest.native.unwrap().mixin[0].id, "épée-✓");
    }

    #[test]
    fn the_native_or_wasm_refusal_does_not_mask_another_error() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
            "a".repeat(128)
        );
        // The wasm table itself is wrong: that stays the error, whatever `[native]` holds.
        let broken = wasm.replace("toolchain='nightly'\n", "toolchain=3\n");
        for code in [broken.clone(), format!("{broken}{}", native())] {
            let error = parse(&code, ROW).unwrap_err();
            assert!(!error.contains("not both"), "{error}");
        }
        let error = parse(&format!("{broken}{}", native()), ROW).unwrap_err();
        assert!(error.contains("invalid type"), "{error}");
        // Not TOML at all.
        let error = parse(&format!("{wasm}= ="), ROW).unwrap_err();
        assert!(!error.contains("not both"), "{error}");
    }

    #[test]
    fn mixins_bridges_states_modules_share_one_id_space() {
        let tables = [
            ROW.to_owned(),
            "[[native.module]]\nid='autosave'\nfile='game.rs'\nname='runtime'\nsource='src/runtime.rs'\n".into(),
            "[[native.bridge]]\nid='autosave'\nkind='GET'\ntarget='game::Game'\nmember='world'\n".into(),
            "[[native.state]]\nid='autosave'\ntarget='game::Game'\nty='u32'\ninitial='0'\nconstructor=[]\n".into(),
        ];
        for (a, first) in tables.iter().enumerate() {
            // Top-level rows precede `[native]`; its own tables follow it.
            let text = |second: &str| {
                let (top, nested): (Vec<&str>, Vec<&str>) = [first.as_str(), second]
                    .into_iter()
                    .partition(|table| !table.starts_with("[[native."));
                format!("{HEADER}{}{}{}", top.concat(), native(), nested.concat())
            };
            assert!(Manifest::parse(&text("")).is_ok(), "{first}");
            for second in &tables[a + 1..] {
                let error = Manifest::parse(&text(second)).unwrap_err();
                assert!(error.contains("duplicate"), "{first}{second}: {error}");
                assert!(
                    Manifest::parse(&text(&second.replace("id='autosave'", "id='other'"))).is_ok(),
                    "{first}{second}"
                );
            }
        }
        let manifest = Manifest::parse(&format!(
            "{HEADER}{}{}{}",
            native(),
            tables[2],
            tables[3].replace("id='autosave'", "id='count'")
        ))
        .unwrap();
        let native = manifest.native.unwrap();
        assert_eq!(native.bridge[0].fields["member"].as_str(), Some("world"));
        assert_eq!(native.state[0].fields["ty"].as_str(), Some("u32"));
    }

    /// A bridge's id names its accessor, so it is refused when it can't, naming the mod and row.
    #[test]
    fn a_bridge_id_that_cannot_name_an_accessor_is_refused() {
        let bridge = |id: &str| {
            format!(
                "[[native.bridge]]\nid='{id}'\nkind='GET'\ntarget='game::Game'\nmember='world'\n"
            )
        };
        assert!(parse(&native(), &bridge("ticks-current_2")).is_ok());
        let error = parse(&native(), &bridge("ticks.current")).unwrap_err();
        assert!(
            error.contains("E_MIXIN_INVALID: sample:ticks.current: invalid id; a bridge id is ASCII letters, digits, `_` and `-`"),
            "{error}"
        );
    }

    #[test]
    fn two_bridge_ids_with_one_accessor_name_are_refused() {
        let bridge = |id: &str| {
            format!(
                "[[native.bridge]]\nid='{id}'\nkind='GET'\ntarget='game::Game'\nmember='world'\n"
            )
        };
        let error = parse(
            &native(),
            &format!("{}{}", bridge("ticks-a"), bridge("Ticks_A")),
        )
        .unwrap_err();
        assert!(
            error.contains("E_MIXIN_INVALID: sample:Ticks_A: invalid id; it gives the same accessor name as bridge `ticks-a`"),
            "{error}"
        );
    }
}
