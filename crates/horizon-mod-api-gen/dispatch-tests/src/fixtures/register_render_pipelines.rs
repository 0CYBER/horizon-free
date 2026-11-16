mod api {
    #[derive(Clone)]
    pub struct RenderPipeline;
}
mod crate_root {
    // Dispatch fixtures isolate ordering; API tests cover pipeline validation.
    mod client {
        pub(crate) mod render_pipeline {
            pub(crate) fn validate_render_pipelines<T>(_: &str, _: &[T], _: &mut Vec<T>) {}
        }
    }
}
