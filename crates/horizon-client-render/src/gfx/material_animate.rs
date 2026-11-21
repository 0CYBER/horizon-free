//! Material companions follow selected colour frames without colour interpolation.
use crate::sprites::SpriteAnimation;

struct Sprite {
    animation: SpriteAnimation,
    shown: [Option<usize>; 2],
}

pub(super) struct MaterialAnimations {
    sprites: Vec<Sprite>,
}

impl MaterialAnimations {
    pub(super) fn new(animations: &[SpriteAnimation]) -> Self {
        Self {
            sprites: animations
                .iter()
                .filter(|animation| animation.has_materials())
                .map(|animation| {
                    // Resource upload uses the clip's declared first frame too.
                    let first = animation.frame(0);
                    let shown = std::array::from_fn(|kind| {
                        animation
                            .material_frames(kind)
                            .map(|frames| first % frames.len())
                    });
                    Sprite {
                        animation: animation.clone(),
                        shown,
                    }
                })
                .collect(),
        }
    }

    /// Emit complete data mip chains only for companions whose selected frame changed.
    /// Single-frame maps remain at their initial upload even while colour interpolates.
    /// Every material frame is written again on the next `animate` (see `SpriteAnimator::forget`).
    pub(super) fn forget(&mut self) {
        for sprite in &mut self.sprites {
            sprite.shown = [None; 2];
        }
    }

    pub(super) fn animate(
        &mut self,
        selected: impl IntoIterator<Item = (u32, usize)>,
        size: u32,
        levels: u32,
        mut upload: impl FnMut(u32, usize, u32, u32, &[u8]),
    ) -> [Vec<u32>; 2] {
        let mut changed = [Vec::new(), Vec::new()];
        for (layer, frame) in selected {
            let Some(sprite) = self.sprites.iter_mut().find(|s| s.animation.layer == layer) else {
                continue;
            };
            for kind in 0..2 {
                let Some(frames) = sprite.animation.material_frames(kind) else {
                    continue;
                };
                let frame = frame % frames.len();
                if sprite.shown[kind] == Some(frame) {
                    continue;
                }
                let pixels = &frames[frame];
                let from = (pixels.len() / 4).isqrt() as u32;
                let pixels: std::borrow::Cow<'_, [u8]> = if from == size {
                    std::borrow::Cow::Borrowed(pixels)
                } else {
                    std::borrow::Cow::Owned(crate::sprites::resample(pixels, from, size))
                };
                // Identical reduction to PreparedTextures::with_pbr, including height alpha.
                let chain = super::box_mip_chain(&pixels, size, 1);
                let mut offset = 0;
                for level in 0..levels {
                    let side = size >> level;
                    let bytes = (side * side * 4) as usize;
                    upload(layer, kind, level, side, &chain[offset..offset + bytes]);
                    offset += bytes;
                }
                sprite.shown[kind] = Some(frame);
                changed[kind].push(layer);
            }
        }
        changed
    }
}
