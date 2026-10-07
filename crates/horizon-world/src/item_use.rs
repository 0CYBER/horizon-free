//! Living blocking-item eligibility retains the component's delay, but rereads
//! use-item storage and remaining ticks around the virtual item-duration query.
pub trait ItemUseWorld {
    type Error;
    type Item: Clone;
    type ItemType: Copy;
    fn using_item(&mut self) -> Result<bool, Self::Error>;
    fn use_item(&self) -> Self::Item;
    fn blocking_delay(&self, item: Self::Item) -> Option<i32>;
    fn item_type(&self, item: Self::Item) -> Self::ItemType;
    fn use_duration(
        &mut self,
        item_type: Self::ItemType,
        item: Self::Item,
    ) -> Result<i32, Self::Error>;
    fn remaining_ticks(&self) -> i32;
    fn remaining_ticks_virtual(&mut self) -> Result<i32, Self::Error>;
    fn query_blocking_item(&mut self) -> Result<Option<Self::Item>, Self::Error>
    where
        Self: Sized,
    {
        blocking_item(self)
    }
    fn query_ticks_using_item(&mut self) -> Result<i32, Self::Error>
    where
        Self: Sized,
    {
        ticks_using_item(self)
    }
}
pub fn blocking_item<W: ItemUseWorld>(w: &mut W) -> Result<Option<W::Item>, W::Error> {
    if !w.using_item()? {
        return Ok(None);
    }
    let Some(delay) = w.blocking_delay(w.use_item()) else {
        return Ok(None);
    };
    let item_type = w.item_type(w.use_item());
    let item = w.use_item();
    let duration = w.use_duration(item_type, item)?;
    let elapsed = duration.wrapping_sub(w.remaining_ticks());
    Ok((elapsed >= delay).then(|| w.use_item()))
}

pub fn is_blocking<W: ItemUseWorld>(w: &mut W) -> Result<bool, W::Error> {
    Ok(w.query_blocking_item()?.is_some())
}
pub fn ticks_using_item<W: ItemUseWorld>(w: &mut W) -> Result<i32, W::Error> {
    if !w.using_item()? {
        return Ok(0);
    }
    let item = w.use_item();
    let item_type = w.item_type(item.clone());
    let duration = w.use_duration(item_type, item)?;
    Ok(duration.wrapping_sub(w.remaining_ticks_virtual()?))
}
pub fn ticks_using_item_partial<W: ItemUseWorld>(w: &mut W, partial: f32) -> Result<f32, W::Error> {
    if !w.using_item()? {
        return Ok(0.0);
    }
    Ok(w.query_ticks_using_item()? as f32 + partial)
}
