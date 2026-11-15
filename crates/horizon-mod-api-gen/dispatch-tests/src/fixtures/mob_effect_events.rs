mod api {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum EffectApplicable {
        Allow,
        Deny,
    }
}
