use super::*;

impl Game<'_> {
    /// `ModelEvent.BakingCompleted`: the resources' models are baked and published, the
    /// standalone ones included, so `model` resolves them in the new store. Once per successful
    /// model load, after both atlases' `texture_atlas_stitched`: on the first load before
    /// `client_setup`, after a successful reload before `client_resources_reloaded`.
    pub(in crate::game) fn native_model_baking_completed(&mut self) {
        self.fire_model_baking_completed();
    }
}
