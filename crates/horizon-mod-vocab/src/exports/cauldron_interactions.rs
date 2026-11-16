pub use block::cauldron_interactions::{
    CauldronDecision, CauldronFire, CauldronInteractions, CauldronPlan, CauldronRegistrationError,
    CauldronUse, Interaction,
};

mod __private {
    pub use crate::block::cauldron_interactions::cauldron_owner;
}
