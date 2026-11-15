pub use generated::{EntityPart, Hooks};
// hidden: a game-side re-export, not part of the mod surface
#[doc(hidden)]
pub use generated::*;
mod __private {
    pub use crate::generated::calls::*;
}
