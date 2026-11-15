#[derive(Default)]
pub(super) struct ModBlockEntity {
    id: String,
    pub(super) data: Vec<i32>,
}
impl ModBlockEntity {
    pub(super) fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            ..Default::default()
        }
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
}
