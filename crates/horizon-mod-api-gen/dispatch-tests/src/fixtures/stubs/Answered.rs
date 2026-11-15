#[derive(Debug, PartialEq)]
pub(super) struct Answered<T> {
    pub(super) answer: T,
    pub(super) mod_id: &'static str,
}
