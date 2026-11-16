pub(super) struct HurtSource<'a> {
    pub(super) damage_type: &'a str,
    pub(super) position: Option<[f64; 3]>,
}
