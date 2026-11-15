/// How a mod tank misanswers: an extract takes and answers 100 mB more than asked
/// (`GivesMore`), or an insert answers 100 mB more than went in (`TakesMore`).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Lie {
    GivesMore,
    TakesMore,
}
