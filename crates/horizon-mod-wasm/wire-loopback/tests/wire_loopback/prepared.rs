//! The production prepared-payload authority, exposed through ordinary fixture discovery.
//! Both generated host decisions and the prepared-equipment transport use this one instance.
#[path = "../../../src/prepared.rs"]
mod authority;
pub(super) use authority::*;
