//! Pinned living-entity use flags backed by synchronized byte storage.
use crate::{
    player_equipment::InteractionHand,
    synced_values::{DataError, Observer, SyncedValues},
};
pub const FLAGS_ID: u8 = 8;
pub fn using_item(data: &SyncedValues<u8>) -> Result<bool, u8> {
    Ok(data.get(FLAGS_ID)? & 1 != 0)
}
pub fn used_hand(data: &SyncedValues<u8>) -> Result<InteractionHand, u8> {
    Ok(if data.get(FLAGS_ID)? & 2 != 0 {
        InteractionHand::Off
    } else {
        InteractionHand::Main
    })
}
pub fn set_flag<O: Observer<u8>>(
    data: &mut SyncedValues<u8>,
    mask: i32,
    value: bool,
    observer: &mut O,
) -> Result<(), DataError<O::Error>> {
    let old = i32::from(data.get(FLAGS_ID).map_err(DataError::UnknownId)? as i8);
    let next = if value { old | mask } else { old & !mask };
    data.set(FLAGS_ID, next as u8, false, observer)
}
