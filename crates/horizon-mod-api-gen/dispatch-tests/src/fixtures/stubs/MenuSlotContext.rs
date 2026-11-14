#[derive(Default)]
pub(super) struct MenuSlotContext {
    pub player: Option<u128>,
    pub pos: Option<[i32; 3]>,
    pub slots: Vec<Vec<Option<super::ItemStack>>>,
}
