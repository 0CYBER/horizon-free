//! The block atlases a `shaders/` pack reads, kept while what they are made from stays: the
//! game's block sprites and the resource packs' material maps, never the pack's own files or
//! options. A pack loaded again (its options changed) finds them as they are. Each is as large
//! as every block sprite together, so making them again beside what video memory already held
//! found no room, and the pack was turned off.
//!
//! Where they are made, the old ones are let go first, and video memory running out is not
//! fatal (`frame_targets::made`, #13624): none is kept then, and the next load makes them.

use super::PackAtlas;
use crate::gfx::frame_targets::{Kind, made};

/// What the atlases are made from.
pub(in crate::gfx) struct Sprites<'a> {
    /// The block sprite arrays, `split` layers each.
    pub arrays: &'a [wgpu::Texture],
    pub split: u32,
    /// Each layer's side before the arrays scaled it up ([`PackAtlas::stitched`]).
    pub native: &'a [u32],
    /// The resource packs' normal and specular maps: a pair of arrays per sprite array.
    pub maps: Option<&'a [[wgpu::Texture; 2]]>,
    /// Flat draws: the atlas is stitched as the game's. Other draws need the grid.
    pub flat: bool,
    /// The Mipmap Levels option and the anisotropy bit, which set a stitched atlas's padding.
    pub mip_option: u32,
    pub anisotropy_bit: u32,
}

#[derive(Default)]
pub(in crate::gfx) struct Atlases {
    pub colour: Option<PackAtlas>,
    /// The normal and specular maps, in the colour atlas's layout.
    pub materials: Option<[PackAtlas; 2]>,
    /// The Mipmap Levels option and the anisotropy bit a stitched `colour` was made for.
    padding: [u32; 2],
}

impl Atlases {
    /// Lets the atlases go: no `shaders/` pack runs.
    pub fn release(&mut self) {
        self.colour = None;
        self.materials = None;
    }

    /// Makes the atlases where there are none, or where they were made for other draws or
    /// another padding. `false` where video memory had no room for them: none is kept then.
    /// `queue` is the device's, with nothing waiting to be submitted on another thread.
    pub fn ready(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, sprites: &Sprites) -> bool {
        let padding = [sprites.mip_option, sprites.anisotropy_bit];
        if (self.colour.as_ref())
            .is_some_and(|a| a.is_stitched() && (!sprites.flat || self.padding != padding))
        {
            self.colour = None;
        }
        if self.colour.is_some() {
            return true;
        }
        // Free the old ones before making any: together they are several times the block
        // sprites. The GPU may still be using them, and what the last pack drew into, so wait
        // for it; this is rare.
        self.materials = None;
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        // Submitted inside the scopes too: the copies can find no room as well.
        let Some((colour, materials)) = made(device, Kind::PackAtlas, || {
            let (colour, materials, commands) = Self::make(device, sprites);
            queue.submit([commands]);
            (colour, materials)
        }) else {
            return false;
        };
        if let Some(colour) = &colour {
            let [width, height] = colour.size();
            let maps = if materials.is_some() { 2 } else { 0 };
            log::info!("shader pack block atlas: {width}x{height}, {maps} material maps");
        }
        self.colour = colour;
        self.materials = materials;
        self.padding = padding;
        true
    }

    /// The colour atlas (none where it does not fit the device), its material maps and the
    /// commands that fill them.
    fn make(
        device: &wgpu::Device,
        sprites: &Sprites,
    ) -> (
        Option<PackAtlas>,
        Option<[PackAtlas; 2]>,
        wgpu::CommandBuffer,
    ) {
        let mut encoder = device.create_command_encoder(&Default::default());
        let arrays = sprites.arrays;
        let layers = arrays.iter().map(|t| t.depth_or_array_layers()).sum();
        let sprite = arrays[0].width();
        let stitched = sprites
            .flat
            .then(|| {
                PackAtlas::stitched(
                    device,
                    &mut encoder,
                    arrays,
                    sprites.split,
                    sprite,
                    sprites.native,
                    sprites.mip_option,
                    sprites.anisotropy_bit,
                )
            })
            .flatten();
        if sprites.flat && stitched.is_none() {
            log::warn!(
                "shader pack block atlas: the sprites don't stitch on this device, a grid instead"
            );
        }
        let colour = stitched.or_else(|| {
            PackAtlas::new(device, &mut encoder, arrays, sprites.split, layers, sprite)
        });
        let materials = colour.as_ref().and_then(|colour| {
            let maps = sprites.maps?;
            let make = |kind: usize, encoder: &mut wgpu::CommandEncoder| {
                let arrays: Vec<_> = maps.iter().map(|pair| pair[kind].clone()).collect();
                colour.material_for(device, encoder, &arrays, layers)
            };
            Some([make(0, &mut encoder)?, make(1, &mut encoder)?])
        });
        (colour, materials, encoder.finish())
    }
}

#[cfg(test)]
#[path = "atlases_tests.rs"]
mod tests;
