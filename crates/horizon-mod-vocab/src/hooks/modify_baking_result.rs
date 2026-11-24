//! The `modify_baking_result` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Inspect or edit the candidate block and standalone geometry after baking and before
    /// publication, once per client resource load that reaches this stage. The first listener
    /// sees the fresh bake; later listeners see accepted edits. Client-native, on the resource
    /// loader, without a live client host. A candidate may still fail to publish.
    ///
    /// Not on wasm: the model catalog is a short-lived borrow of native renderer data.
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        target = "horizon_client_render::models::catalog::baking_result::native_modify_baking_result",
        neoforge = "ModelEvent.ModifyBakingResult (block and standalone bake-time subset)"
    )]
    fn modify_baking_result();
}
