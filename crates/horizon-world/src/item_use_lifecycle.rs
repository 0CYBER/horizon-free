//! Living item-use lifecycle ordering over stable item-stack handles.
#[derive(Clone, Copy, Debug)]
pub struct UseState<I> {
    pub item: I,
    pub remaining: i32,
    pub has_recent_kinetic_enemies: bool,
}
pub trait UseLifecycleWorld: Sized {
    type Error;
    type Item: Clone + Eq;
    type Hand: Copy;
    type Level: Copy;
    fn empty_item(&self) -> Self::Item;
    fn is_empty(&self, item: Self::Item) -> bool;
    fn same_item_type(&self, a: Self::Item, b: Self::Item) -> bool;
    fn using_item(&mut self, s: &mut UseState<Self::Item>) -> Result<bool, Self::Error>;
    fn used_hand(&mut self, s: &mut UseState<Self::Item>) -> Result<Self::Hand, Self::Error>;
    fn hand_item(
        &mut self,
        s: &mut UseState<Self::Item>,
        hand: Self::Hand,
    ) -> Result<Self::Item, Self::Error>;
    fn set_hand_item(
        &mut self,
        s: &mut UseState<Self::Item>,
        hand: Self::Hand,
        item: Self::Item,
    ) -> Result<(), Self::Error>;
    fn level(&mut self, s: &mut UseState<Self::Item>) -> Result<Self::Level, Self::Error>;
    fn is_client(
        &mut self,
        s: &mut UseState<Self::Item>,
        level: Self::Level,
    ) -> Result<bool, Self::Error>;
    fn remaining_ticks(&mut self, s: &mut UseState<Self::Item>) -> Result<i32, Self::Error>;
    fn item_tick(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error>;
    fn use_on_release(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
    ) -> Result<bool, Self::Error>;
    fn release_item(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error>;
    fn finish_item(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
        level: Self::Level,
    ) -> Result<Self::Item, Self::Error>;
    fn set_using_flag(
        &mut self,
        s: &mut UseState<Self::Item>,
        using: bool,
    ) -> Result<(), Self::Error>;
    fn finish_vibration(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
    ) -> Result<(), Self::Error>;
    fn update_using_item(
        &mut self,
        s: &mut UseState<Self::Item>,
        item: Self::Item,
    ) -> Result<(), Self::Error> {
        update(self, s, item)
    }
    fn complete_using_item(&mut self, s: &mut UseState<Self::Item>) -> Result<(), Self::Error> {
        complete(self, s)
    }
    fn release_using_item(&mut self, s: &mut UseState<Self::Item>) -> Result<(), Self::Error> {
        release(self, s)
    }
    fn stop_using_item(&mut self, s: &mut UseState<Self::Item>) -> Result<(), Self::Error> {
        stop(self, s)
    }
}
pub fn tick<W: UseLifecycleWorld>(w: &mut W, s: &mut UseState<W::Item>) -> Result<(), W::Error> {
    if w.using_item(s)? {
        let hand = w.used_hand(s)?;
        let held = w.hand_item(s, hand)?;
        if w.same_item_type(held.clone(), s.item.clone()) {
            let hand = w.used_hand(s)?;
            s.item = w.hand_item(s, hand)?;
            w.update_using_item(s, s.item.clone())?;
        } else {
            w.stop_using_item(s)?;
        }
    }
    Ok(())
}
pub fn update<W: UseLifecycleWorld>(
    w: &mut W,
    s: &mut UseState<W::Item>,
    item: W::Item,
) -> Result<(), W::Error> {
    let level = w.level(s)?;
    let remaining = w.remaining_ticks(s)?;
    w.item_tick(s, item.clone(), level, remaining)?;
    s.remaining = s.remaining.wrapping_sub(1);
    if s.remaining == 0 {
        let level = w.level(s)?;
        if !w.is_client(s, level)? && !w.use_on_release(s, item)? {
            w.complete_using_item(s)?;
        }
    }
    Ok(())
}
pub fn complete<W: UseLifecycleWorld>(
    w: &mut W,
    s: &mut UseState<W::Item>,
) -> Result<(), W::Error> {
    let level = w.level(s)?;
    if w.is_client(s, level)? && !w.using_item(s)? {
        return Ok(());
    }
    let hand = w.used_hand(s)?;
    let original = s.item.clone();
    if original != w.hand_item(s, hand)? {
        w.release_using_item(s)?;
        return Ok(());
    }
    if !w.is_empty(s.item.clone()) && w.using_item(s)? {
        let item = s.item.clone();
        let level = w.level(s)?;
        let result = w.finish_item(s, item, level)?;
        if result != s.item {
            w.set_hand_item(s, hand, result)?;
        }
        w.stop_using_item(s)?;
    }
    Ok(())
}
pub fn release<W: UseLifecycleWorld>(w: &mut W, s: &mut UseState<W::Item>) -> Result<(), W::Error> {
    let hand = w.used_hand(s)?;
    let held = w.hand_item(s, hand)?;
    if !w.is_empty(s.item.clone()) && w.same_item_type(held.clone(), s.item.clone()) {
        s.item = held;
        let item = s.item.clone();
        let level = w.level(s)?;
        let remaining = w.remaining_ticks(s)?;
        w.release_item(s, item, level, remaining)?;
        if w.use_on_release(s, s.item.clone())? {
            tick(w, s)?;
        }
    }
    w.stop_using_item(s)
}
pub fn stop<W: UseLifecycleWorld>(w: &mut W, s: &mut UseState<W::Item>) -> Result<(), W::Error> {
    let level = w.level(s)?;
    if !w.is_client(s, level)? {
        let using = w.using_item(s)?;
        s.has_recent_kinetic_enemies = false;
        w.set_using_flag(s, false)?;
        if using {
            w.finish_vibration(s, s.item.clone())?;
        }
    }
    s.item = w.empty_item();
    s.remaining = 0;
    Ok(())
}
