pub(super) struct Loop(pub &'static str);
impl LoopContext for Loop {
    fn side(&self) -> &'static str {
        self.0
    }
}
