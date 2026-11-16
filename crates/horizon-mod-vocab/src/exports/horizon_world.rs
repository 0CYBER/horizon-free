/// The structure box a piece's `bounding_box` answers, in world blocks.
pub use horizon_core::bounding_box::BoundingBox;
pub use horizon_core::equipment_slot::EquipmentSlot;
pub use horizon_core::identifier::Identifier;
/// A level's key (`LevelDimension::Custom`).
pub use horizon_core::level_key::LevelKey;
pub use horizon_core::nbt::Compound;
pub use horizon_core::nbt::Tag;
pub use horizon_core::use_animation::UseAnimation;
pub use {
    horizon_core::{
        connection_kind::ConnectionKind, dimension::Dimension as LevelDimension,
        neighbour_update::NeighbourUpdate,
    },
    horizon_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentForm, AttachmentPolicy, AttachmentStore,
        AttachmentType, AttachmentValue, AttachmentWrite, Holder,
    },
};
// hidden: the game's maps behind the store, not part of the mod surface
#[doc(hidden)]
pub use horizon_mod_runtime::attachments::{AttachmentAccess, AttachmentHost, Attachments};
