mod api {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PushReaction {
        PushPull,
        Push,
        Popped,
        Immoveable,
        IgnoreEntity,
    }
}
