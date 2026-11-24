#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ContainerMouseInput {
    Scroll([f64; 2]),
}
