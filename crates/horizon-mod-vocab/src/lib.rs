//! The API vocabulary re-exported by `horizon-mod-api`.
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(ambiguous_glob_reexports)]
extern crate self as mod_api;

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};
use std::time::Duration;

mod generated_host {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated_host.rs"));
}

// no wire: type alias inherits the Wire policy of its tuple or Result target.
/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
}

include!(concat!(env!("OUT_DIR"), "/mod_api_exports_root.rs"));
include!(concat!(env!("OUT_DIR"), "/wire_generated.rs"));

/// The mod's hooks for this crate's unit test binary, which drives no guest code. In a workspace build
/// `horizon-mod-api` has `wasm-guest` on and declares `__horizon_mod_hooks`; MSVC's linker refuses the
/// unresolved symbol even though nothing calls it (`horizon-mod-api`'s `tests/support/no_hooks.rs`).
#[cfg(test)]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn __horizon_mod_hooks() -> Box<dyn ::mod_api::Hooks> {
    struct Empty;
    impl ::mod_api::Hooks for Empty {
        fn subscribed(&self) -> &'static [Option<::mod_api::HookId>] {
            &[]
        }
    }
    Box::new(Empty)
}
