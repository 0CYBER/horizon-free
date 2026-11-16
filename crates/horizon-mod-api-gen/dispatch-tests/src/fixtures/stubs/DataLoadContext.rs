pub(super) trait DataLoadContext {
    fn origin(&self) -> &'static str;
}
impl DataLoadContext for Load {
    fn origin(&self) -> &'static str {
        self.0
    }
}
