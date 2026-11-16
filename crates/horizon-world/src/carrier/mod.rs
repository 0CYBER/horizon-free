//! Moving-structure carriers. Slices of the moving structures design
//! (`docs/design/mod-api/moving-structures.md`) landed so far as
//! pure, disabled pieces with no gameplay wiring: the rigid contact
//! kernel (slice 5) as a geometry harness, and the carrier data model
//! with its bounded NBT codec (slice 1, board issue 1).

pub mod codec;
pub mod contact;
pub mod model;
pub mod region_strict;
