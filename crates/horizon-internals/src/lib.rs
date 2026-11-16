//! A native mod's declared game internals.
//!
//! The native mixin build replaces this crate with a facade generated from the mod's
//! `[[internals]]` declarations. Only declared types and functions are re-exported there.
#![forbid(unsafe_code)]
