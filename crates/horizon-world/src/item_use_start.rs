//! Begin using a held stack, retaining native callback and flag ordering.
use crate::item_use_lifecycle::{UseLifecycleWorld, UseState};
pub trait StartUseWorld: UseLifecycleWorld {
    fn duration(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
    ) -> Result<i32, Self::Error>;
    fn synchronize_equipment(
        &mut self,
        s: &mut UseState<Self::Item>,
        level: Self::Level,
        hand: Self::Hand,
        item: Self::Item,
    ) -> Result<(), Self::Error>;
    fn is_off_hand(&self, hand: Self::Hand) -> bool;
    fn set_off_hand_flag(
        &mut self,
        s: &mut UseState<Self::Item>,
        off: bool,
    ) -> Result<(), Self::Error>;
    fn start_vibration(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
    ) -> Result<(), Self::Error>;
    fn has_kinetic_weapon(&self, item: Self::Item) -> bool;
    fn reset_recent_kinetic_enemies(&mut self, s: &mut UseState<Self::Item>);
}
pub fn start<W: StartUseWorld>(
    w: &mut W,
    s: &mut UseState<W::Item>,
    hand: W::Hand,
) -> Result<(), W::Error> {
    let item = w.hand_item(s, hand)?;
    if w.is_empty(item.clone()) || w.using_item(s)? {
        return Ok(());
    }
    s.item = item.clone();
    s.remaining = w.duration(s, item)?;
    let level = w.level(s)?;
    if !w.is_client(s, level)? {
        let level = w.level(s)?;
        w.synchronize_equipment(s, level, hand, s.item.clone())?;
        w.set_using_flag(s, true)?;
        let off = w.is_off_hand(hand);
        w.set_off_hand_flag(s, off)?;
        w.start_vibration(s, s.item.clone())?;
        if w.has_kinetic_weapon(s.item.clone()) {
            w.reset_recent_kinetic_enemies(s);
        }
    }
    Ok(())
}
