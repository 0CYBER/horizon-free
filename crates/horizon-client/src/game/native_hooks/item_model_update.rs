use super::*;

impl Game<'_> {
    /// A node of a type a mod registered (`register_item_models`) is resolved for a stack: push
    /// the names of the node's `models` to draw, in order, only for a type your mod owns.
    /// `config` is the node's JSON object. `bundle_selected` is the runtime bundle selection
    /// index; -1 means none. `display`, `owner`, `level` and `seed` are what `ItemModel.update`
    /// reads besides the stack.
    pub(in crate::game) fn native_item_model_update(
        &self,
        call: mod_api::__private::ItemModelUpdateCall<'_>,
    ) {
        self.fire_item_model_update(call);
    }
}
