//! Exports of `crate::parts` items defined in `parts/items.rs`.
pub use parts::{ItemCapabilityProvider, ItemProvider};
mod __private {
    pub use crate::parts::{ItemKeyError, TooltipAppenderPart, item_listens, tooltip_appenders};
}
