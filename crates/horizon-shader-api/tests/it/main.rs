//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

mod check;
mod compute;
mod kept;
mod material_reads;
mod motion;
mod names;
mod override_step;
mod package_limits;
mod reference;
mod schema;
mod sdk;
