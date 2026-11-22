/// What the tests switch: the boxes taken at the start of a tick off, and how often one answered.
pub(super) mod tests_support {
    use std::cell::Cell;
    thread_local! {
        pub(in crate::game) static NEAR_CARTS_OFF: Cell<bool> = const { Cell::new(false) };
        pub(in crate::game) static NEAR_CARTS_ANSWERS: Cell<u32> = const { Cell::new(0) };
    }
}
