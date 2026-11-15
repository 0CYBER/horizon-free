/// The block at `pos` in [`here`], through `side`.
pub(crate) fn on(pos: [i32; 3], side: Option<Direction>) -> BlockSide {
    mod_vocab::__private::parts_installation::block_side(here(), pos, side)
}
