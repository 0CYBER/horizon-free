pub(super) struct ModBlockEntityView<'a> {
    id: &'a str,
    pub(super) data: &'a mut Vec<i32>,
}
impl<'a> ModBlockEntityView<'a> {
    pub(super) fn new(entity: &'a mut ModBlockEntity) -> Self {
        Self {
            id: &entity.id,
            data: &mut entity.data,
        }
    }
    pub(super) fn id(&self) -> &str {
        self.id
    }
    pub(super) fn __begin_tick_listener(&mut self) {}
    pub(super) fn __finish_tick_listener(&mut self, _: BlockEntityTickReply) {}
}
