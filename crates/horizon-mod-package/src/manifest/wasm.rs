use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

/// The most a `mod.wasm` may weigh.
pub const MAX_WASM: u64 = 64 << 20;

/// A wasm mod's `[wasm]` section, written by `cargo horizon-mod build --wasm` (or a mod converter),
/// never by hand: what its one `mod.wasm` is and what it was built and linked against.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(try_from = "Fields")]
pub struct Wasm {
    sha512: String,
    toolchain: String,
    linked: std::collections::BTreeMap<String, String>,
    built_with: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    sha512: String,
    toolchain: String,
    #[serde(default)]
    linked: std::collections::BTreeMap<String, String>,
    built_with: Option<String>,
}

impl TryFrom<Fields> for Wasm {
    type Error = String;

    fn try_from(fields: Fields) -> Result<Self, Self::Error> {
        sha512_hex(&fields.sha512).map_err(|e| format!("[wasm] sha512 {e}"))?;
        if fields.toolchain.is_empty() {
            return Err("[wasm] toolchain is empty".into());
        }
        for (id, hash) in &fields.linked {
            sha512_hex(hash).map_err(|e| format!("[wasm] linked {id} {e}"))?;
        }
        if fields.built_with.as_deref().is_some_and(str::is_empty) {
            return Err("[wasm] built_with is empty".into());
        }
        Ok(Self {
            sha512: fields.sha512,
            toolchain: fields.toolchain,
            linked: fields.linked,
            built_with: fields.built_with,
        })
    }
}

/// Checks a SHA-512 written as 128 lowercase hex digits, the form `[[jar]]` uses too.
pub(crate) fn sha512_hex(value: &str) -> Result<(), &'static str> {
    if value.len() == 128
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err("must be 128 lowercase hex digits")
    }
}

/// The SHA-512 of `bytes`, as `[wasm]` writes it.
pub fn sha512_of(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a package's `mod.wasm` is refused, named on the Mods page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WasmError {
    /// `[wasm]` declared, no `mod.wasm` beside it.
    Missing,
    /// A `mod.wasm` without `[wasm]`.
    Undeclared,
    TooLarge,
    /// Not a WebAssembly file (no `\0asm` header).
    NotWasm,
    /// Not a valid shared group member with the guest entry points.
    InvalidMember,
    /// The bytes aren't the ones `[wasm] sha512` names.
    Hash,
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "[wasm] is declared but the package has no mod.wasm",
            Self::Undeclared => "mod.wasm needs a [wasm] section",
            Self::TooLarge => "mod.wasm exceeds 64 MiB",
            Self::NotWasm => "mod.wasm is not a WebAssembly file",
            Self::InvalidMember => "mod.wasm is not a valid shared group member",
            Self::Hash => "mod.wasm doesn't match its [wasm] sha512",
        })
    }
}

impl std::error::Error for WasmError {}

impl Wasm {
    /// The SHA-512 of the package's `mod.wasm`.
    pub fn sha512(&self) -> &str {
        &self.sha512
    }

    /// The exact toolchain that built it; every member of a linked group shares one.
    pub fn toolchain(&self) -> &str {
        &self.toolchain
    }

    /// The exact `mod.wasm` (by SHA-512) of each dependency it was linked against.
    pub fn linked(&self) -> &std::collections::BTreeMap<String, String> {
        &self.linked
    }

    /// The converter that made it from a Java mod (`<converter>@<commit>`), when it has no
    /// Rust source of its own.
    pub fn built_with(&self) -> Option<&str> {
        self.built_with.as_deref()
    }

    /// Checks `bytes` are this section's `mod.wasm`. The bytes are hashed on every call; a module
    /// is validated once per process, since the game reads a package several times as it starts.
    pub fn check(&self, bytes: &[u8]) -> Result<(), WasmError> {
        if bytes.len() as u64 > MAX_WASM {
            return Err(WasmError::TooLarge);
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(WasmError::NotWasm);
        }
        if sha512_of(bytes) != self.sha512 {
            return Err(WasmError::Hash);
        }
        let validated = || VALIDATED.lock().unwrap_or_else(|e| e.into_inner());
        if validated().contains(&self.sha512) {
            return Ok(());
        }
        valid_member(bytes)?;
        validated().insert(self.sha512.clone());
        Ok(())
    }
}

/// The SHA-512s of the modules [`Wasm::check`] has found valid members in this process: whether
/// a module is one depends on its bytes alone.
static VALIDATED: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Whether `bytes` are a valid shared group member with the guest entry points.
fn valid_member(bytes: &[u8]) -> Result<(), WasmError> {
    wasmparser::Validator::new()
        .validate_all(bytes)
        .map_err(|_| WasmError::InvalidMember)?;
    let mut shared = false;
    let mut exports = [false; 3];
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload.map_err(|_| WasmError::InvalidMember)? {
            wasmparser::Payload::Version {
                encoding: wasmparser::Encoding::Component,
                ..
            } => return Err(WasmError::InvalidMember),
            wasmparser::Payload::CustomSection(section) if section.name() == "dylink.0" => {
                if shared {
                    return Err(WasmError::InvalidMember);
                }
                let wasmparser::KnownCustom::Dylink0(subsections) = section.as_known() else {
                    return Err(WasmError::InvalidMember);
                };
                let mut memory_info = false;
                for subsection in subsections {
                    if let wasmparser::Dylink0Subsection::MemInfo(_) =
                        subsection.map_err(|_| WasmError::InvalidMember)?
                    {
                        if memory_info {
                            return Err(WasmError::InvalidMember);
                        }
                        memory_info = true;
                    }
                }
                if !memory_info {
                    return Err(WasmError::InvalidMember);
                }
                shared = true;
            }
            wasmparser::Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.map_err(|_| WasmError::InvalidMember)?;
                    // Each API call's own import, which the game links by name:
                    // `Owner::method#signature`.
                    let call = import.module == "horizon"
                        && import.name.contains("::")
                        && import.name.contains('#')
                        && matches!(import.ty, wasmparser::TypeRef::Func(_));
                    let allowed = matches!(
                        (import.module, import.name, import.ty),
                        ("env", "memory", wasmparser::TypeRef::Memory(_))
                            | (
                                "env",
                                "__indirect_function_table",
                                wasmparser::TypeRef::Table(_)
                            )
                            | (
                                "env",
                                "__stack_pointer" | "__memory_base" | "__table_base",
                                wasmparser::TypeRef::Global(_),
                            )
                            | ("env", _, wasmparser::TypeRef::Func(_))
                            | ("GOT.mem" | "GOT.func", _, wasmparser::TypeRef::Global(_))
                            | (
                                "horizon",
                                "host" | "host_reply",
                                wasmparser::TypeRef::Func(_)
                            )
                    );
                    if !allowed && !call {
                        return Err(WasmError::InvalidMember);
                    }
                }
            }
            wasmparser::Payload::ExportSection(section) => {
                for export in section {
                    let export = export.map_err(|_| WasmError::InvalidMember)?;
                    if export.kind == wasmparser::ExternalKind::Func {
                        for (index, name) in ["horizon_layout", "horizon_in", "horizon_hook"]
                            .iter()
                            .enumerate()
                        {
                            if export.name == *name {
                                exports[index] = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !shared || !exports.into_iter().all(|found| found) {
        return Err(WasmError::InvalidMember);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(bytes: &[u8]) -> Wasm {
        Wasm::try_from(Fields {
            sha512: sha512_of(bytes),
            toolchain: "1.99.1".into(),
            linked: Default::default(),
            built_with: None,
        })
        .unwrap()
    }

    #[test]
    fn a_module_is_checked_by_header_size_and_hash() {
        let module = b"\0asm\x01\0\0\0".to_vec();
        let wasm = section(&module);
        assert_eq!(wasm.check(&module), Err(WasmError::InvalidMember));
        let mut changed = module.clone();
        changed[7] = 1;
        assert_eq!(wasm.check(&changed), Err(WasmError::Hash));
        assert_eq!(section(b"MZ").check(b"MZ"), Err(WasmError::NotWasm));
    }

    #[test]
    fn member_requires_shared_layout_guest_exports_and_allowed_imports() {
        let module = |imports: &str, exports: &str, dylink: &str| {
            wat::parse_str(format!("(module {dylink} {imports} {exports})")).unwrap()
        };
        let dylink = r#"(@custom "dylink.0" "\01\04\00\00\00\00")"#;
        let exports = r#"(func (export "horizon_layout")) (func (export "horizon_in")) (func (export "horizon_hook"))"#;
        let imports = r#"(import "horizon" "host" (func)) (import "env" "memory" (memory 1))"#;
        let good = module(imports, exports, dylink);
        assert_eq!(section(&good).check(&good), Ok(()));
        let call = module(
            r#"(import "horizon" "Host::level#00000000000000ff" (func)) (import "env" "memory" (memory 1))"#,
            exports,
            dylink,
        );
        assert_eq!(section(&call).check(&call), Ok(()));
        for bad in [
            module(imports, exports, ""),
            module(imports, exports, r#"(@custom "dylink.0" "\01\01\00")"#),
            module(imports, exports, r#"(@custom "dylink.0" "")"#),
            module(
                r#"(import "GOT.other" "thing" (global i32))"#,
                exports,
                dylink,
            ),
            module(r#"(import "env" "unknown" (global i32))"#, exports, dylink),
            module(imports, "", dylink),
            module(
                r#"(import "wasi_snapshot_preview1" "fd_write" (func))"#,
                exports,
                dylink,
            ),
            module(r#"(import "horizon" "other" (func))"#, exports, dylink),
        ] {
            assert_eq!(section(&bad).check(&bad), Err(WasmError::InvalidMember));
        }
    }

    #[test]
    fn the_section_refuses_malformed_hashes() {
        let text = |sha: &str| format!("sha512 = \"{sha}\"\ntoolchain = \"1.99.1\"\n");
        assert!(toml::from_str::<Wasm>(&text(&"a".repeat(128))).is_ok());
        assert!(toml::from_str::<Wasm>(&text(&"A".repeat(128))).is_err());
        assert!(toml::from_str::<Wasm>(&text("abc")).is_err());
    }
}
