//! Typed identities and declarations of block entity types.
use crate::BlockEntityTypeKey;
use std::{fmt, sync::Arc};

/// The mirror followed by rotation applied during structure placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct StructureTransform {
    /// Rotation after mirroring.
    pub rotation: crate::StructureRotation,
    /// Mirror before rotation.
    pub mirror: crate::StructureMirror,
}

impl StructureTransform {
    /// Apply `mirror` first and then `rotation`, as template placement does.
    pub fn new(rotation: crate::StructureRotation, mirror: crate::StructureMirror) -> Self {
        Self { rotation, mirror }
    }
}

/// A deterministic transform declined the saved data. Placement keeps the original.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct TransformRefusal {
    /// Diagnostic explaining why the original data must be kept.
    pub reason: String,
}

impl TransformRefusal {
    /// Decline a transform with a diagnostic.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}
impl fmt::Display for TransformRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}
impl std::error::Error for TransformRefusal {}
impl From<(crate::Identifier, String)> for TransformRefusal {
    fn from((id, reason): (crate::Identifier, String)) -> Self {
        Self::new(format!("{id}: {reason}"))
    }
}

/// Saved-data transform, shared by native and wasm declarations.
/// Runs on the server during structure placement, possibly on worldgen worker threads.
/// Its only inputs are saved data, rotation and mirror. Determinism is the mod's
/// obligation, not enforced: any matching `Send + Sync` closure is accepted.
/// The engine preserves the original `id` and any original `x`, `y`, `z` fields.
/// Return a refusal to keep the original data. Output must fit standard NBT limits
/// (64 MiB, one million nodes, depth 512); invalid output keeps the original.
/// Do not panic: native unwinds are caught, and wasm traps refuse the call.
pub trait BlockEntityTransform: Send + Sync + 'static {
    /// Transform saved data; no Host or world handle is supplied.
    fn transform(
        &self,
        data: &crate::Compound,
        transform: StructureTransform,
    ) -> Result<crate::Compound, TransformRefusal>;
}
impl<F> BlockEntityTransform for F
where
    F: Fn(&crate::Compound, StructureTransform) -> Result<crate::Compound, TransformRefusal>
        + Send
        + Sync
        + 'static,
{
    fn transform(
        &self,
        data: &crate::Compound,
        transform: StructureTransform,
    ) -> Result<crate::Compound, TransformRefusal> {
        self(data, transform)
    }
}

/// A native Part cannot be attached to this type during registration.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockEntityPartError {
    /// Vanilla entities do not expose a native Part's payload.
    VanillaType {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
    },
    /// This Part type is already attached to the entity type.
    Duplicate {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
        /// The Part type that is already attached.
        part: &'static str,
    },
}

impl fmt::Display for BlockEntityPartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VanillaType { kind } => write!(
                f,
                "cannot attach a native block entity Part to vanilla type {}",
                crate::content::registry_keys::block_entity_type_display_name(*kind)
            ),
            Self::Duplicate { kind, part } => {
                write!(
                    f,
                    "Part {part} is attached to type {} twice",
                    crate::content::registry_keys::block_entity_type_display_name(*kind)
                )
            }
        }
    }
}

impl std::error::Error for BlockEntityPartError {}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// Builds one mod block entity type during registration on native and wasm.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityTypeBuilder<'a> {
    pub(crate) registrar: &'a mut dyn crate::Registrar,
    pub(crate) id: String,
    pub(crate) blocks: Vec<String>,
    pub(crate) transform: Option<Arc<dyn BlockEntityTransform>>,
}

impl BlockEntityTypeBuilder<'_> {
    /// The blocks which may contain this type. A bare name is this mod's own, as the type's
    /// id is, so `.blocks(&["press"])` names `<mod id>:press`; the block may be declared later.
    pub fn blocks(mut self, blocks: &[&str]) -> Self {
        self.blocks = blocks.iter().map(|block| (*block).to_owned()).collect();
        self
    }

    /// Set the optional saved-data transform; the mod must ensure determinism.
    pub fn transform(mut self, step: impl BlockEntityTransform) -> Self {
        self.transform = Some(Arc::new(step));
        self
    }

    /// Register the type and return its typed key.
    pub fn build(self) -> BlockEntityTypeKey {
        let key = self
            .registrar
            .declare_block_entity_type(&self.id, self.blocks);
        if let Some(step) = self.transform {
            self.registrar.set_block_entity_transform(key, step);
        }
        key
    }
}

impl crate::BlockEntityTypeAddBlocksEvent<'_> {
    /// Add blocks to one registered type, leaving its factory unchanged.
    pub fn modify(&mut self, kind: BlockEntityTypeKey, blocks: &[crate::BlockKey]) {
        self.additions
            .extend(blocks.iter().map(|&block| (kind, block)));
    }

    /// Add blocks to every registered type accepted by `predicate`.
    pub fn modify_matching(
        &mut self,
        mut predicate: impl FnMut(BlockEntityTypeKey) -> bool,
        blocks: &[crate::BlockKey],
    ) {
        for &kind in self.types {
            if predicate(kind) {
                self.additions
                    .extend(blocks.iter().map(|&block| (kind, block)));
            }
        }
    }
}
