//! Optional template saved-data adapter, retained by a registry snapshot.
use super::{Mirror, Rotation};
use horizon_core::nbt::Compound;

/// Higher layers can transform template data without coupling worldgen to their API.
/// Called after state placement, before NBT storage, on live levels and worldgen workers.
pub trait BlockEntityTransform: Send + Sync {
    /// Leave data untouched when no transform applies or the transform refuses.
    fn transform(
        &self,
        position: [i32; 3],
        data: &mut Compound,
        rotation: Rotation,
        mirror: Mirror,
    );
}
