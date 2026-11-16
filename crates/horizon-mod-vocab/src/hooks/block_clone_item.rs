//! Upstream prose, retained with fork routing described in the modding book.
pub trait Notes {
    #[notes = "`native_block_clone_item` supplies a block's clone stack from its actual state before ordinary Ctrl+Pick block-entity component copying. First response owns the result; no response retains vanilla. An empty or unregistered response suppresses picking. Bundles need \"hooks\": @API:block_clone_item@."]
    fn block_clone_item();
}
