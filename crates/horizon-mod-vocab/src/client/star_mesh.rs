//! The CPU-side star geometry a native mod can replace before upload.

/// Maximum number of vertices accepted by [`StarMeshEdit::replace`].
pub const MAX_STAR_VERTICES: usize = 65_535;

/// Why a star mesh replacement was refused. The current mesh is unchanged on refusal.
// no wire: only returned by the native-only star mesh editor
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StarMeshRefusal {
    /// A triangle needs exactly three vertices.
    IncompleteTriangle,
    /// More than [`MAX_STAR_VERTICES`] vertices.
    TooManyVertices,
    /// A position component was non-finite or outside `-4096..=4096` blocks.
    PositionOutOfRange,
}

// no wire: this editor borrows the engine's CPU mesh including its allocation.
/// A native edit of the camera-relative, unrotated star positions.
// exhaustive: its borrowed field is private, so mods cannot build or destructure it
pub struct StarMeshEdit<'a> {
    vertices: &'a mut Vec<[f32; 3]>,
}

impl<'a> StarMeshEdit<'a> {
    // hidden: wraps the renderer's own CPU mesh for a callback, not part of the mod surface
    #[doc(hidden)]
    pub fn new(vertices: &'a mut Vec<[f32; 3]>) -> Self {
        Self { vertices }
    }

    /// The current non-indexed triangle list, including earlier listeners' accepted edits.
    pub fn vertices(&self) -> &[[f32; 3]] {
        self.vertices
    }

    /// Replace the whole list, taking ownership without copying. An empty list removes the stars.
    /// Supply whole triangles, at most [`MAX_STAR_VERTICES`] vertices, and finite position
    /// components within `-4096..=4096` blocks. No truncation, clamping or partial edits.
    pub fn replace(&mut self, vertices: Vec<[f32; 3]>) -> Result<(), StarMeshRefusal> {
        if vertices.len() > MAX_STAR_VERTICES {
            return Err(StarMeshRefusal::TooManyVertices);
        }
        if !vertices.len().is_multiple_of(3) {
            return Err(StarMeshRefusal::IncompleteTriangle);
        }
        if !vertices
            .iter()
            .flatten()
            .all(|v| (-4096.0..=4096.0).contains(v))
        {
            return Err(StarMeshRefusal::PositionOutOfRange);
        }
        *self.vertices = vertices;
        Ok(())
    }
}
