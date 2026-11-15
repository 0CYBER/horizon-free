//! Programs that write no buffer are dropped, but their forced flips still happen (6.1.6).

use std::cell::Cell;

use horizon_shaders::{Group, PostProgram};

use super::Pass;
use crate::gfx::pack_textures;

/// `programs` built into passes; `skip` names programs a developer leaves out.
///
/// A program `build` drops carries its forced `flip.*` bits onto the pass before it: a flip
/// swaps the buffer's two textures, so it composes with that pass's flip, and the dropped pass
/// writes nothing in between. The carry also counts as a use and as a second texture for those
/// buffers, and releases the stage's custom texture over them; all of this happens before the
/// next program is built, whose bindings read the released mask. `final`'s flips go with it:
/// they would happen after the screen draw.
pub(super) fn build_passes(
    programs: impl IntoIterator<Item = PostProgram>,
    skip: &str,
    used: &Cell<u16>,
    doubled: &Cell<u16>,
    released: &Cell<(&'static str, u16)>,
    mut build: impl FnMut(PostProgram) -> Option<Pass>,
) -> Vec<Pass> {
    let mut passes = Vec::new();
    for p in programs
        .into_iter()
        .filter(|p| !skip.split(',').any(|s| s == p.name))
    {
        let flips = p
            .flips
            .iter()
            .filter(|f| f.1)
            .fold(0u16, |m, f| m | 1 << f.0);
        let carry = p.group != Group::Final;
        let stage = pack_textures::stage(p.group);
        match build(p) {
            Some(pass) => passes.push(pass),
            None => {
                if carry
                    && flips != 0
                    && let Some(previous) = passes.last_mut()
                {
                    previous.flip ^= flips;
                    used.set(used.get() | flips);
                    doubled.set(doubled.get() | flips);
                    let (current, mask) = released.get();
                    released.set((
                        stage,
                        if current == stage {
                            mask | flips
                        } else {
                            flips
                        },
                    ));
                }
            }
        }
    }
    passes
}

/// [`build_passes`] for the shadow chain: its own buffers (`shadowcolor`) get one pair of textures.
pub(super) fn build_shadow_passes(
    programs: impl IntoIterator<Item = PostProgram>,
    skip: &str,
    doubled: &Cell<u16>,
    released: &Cell<(&'static str, u16)>,
    build: impl FnMut(PostProgram) -> Option<Pass>,
) -> Vec<Pass> {
    build_passes(programs, skip, doubled, doubled, released, build)
}

/// Whether `final` draws colortex0 to the screen, or Horizon's copy has to.
pub(super) fn shows(passes: &[Pass]) -> bool {
    passes
        .iter()
        .any(|p| p.group == Group::Final && matches!(p.pipeline, super::Pipeline::Render(_)))
}
