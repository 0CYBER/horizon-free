use super::*;

impl Game<'_> {
    /// A recipe viewer's "+" on a mod menu: the scoped part of the open menu's type may move
    /// `recipe`'s ingredients itself (or, with `dry_run`, say what that would do) and answer in
    /// `result`; None hands the transfer on to the game's own grid transfer.
    #[inline(never)]
    pub(in crate::game) fn native_menu_transfer(
        &mut self,
        call: mod_api::__private::MenuTransferCall<'_>,
    ) {
        self.fire_menu_transfer(call);
    }
}
