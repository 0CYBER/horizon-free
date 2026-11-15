//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

mod array_declarators;
mod array_initializer_lists;
mod atomic_member_reads;
mod callable_macro_boundary;
mod global_constant_division;
mod legacy_texture_size;
mod length_array_sizes;
mod local_constants;
mod naga_glsl_atomics;
mod naga_glsl_overloads;
mod naga_hlsl_offsets;
mod naga_hlsl_zero_lod;
mod pack_compat;
mod real_packs;
mod reversed_culling_feature;
mod signed_builtin_indices;
mod stage_local_reachability;
mod terrain_fade;
mod uniform_initial_values;
