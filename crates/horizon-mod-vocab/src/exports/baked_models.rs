pub use client::baked_models::{
    BakedModelEditError, BakedModelKey, BakedModelQuad, BakedModelView, BakedModels, BakedQuadChange,
};
mod __private {
    mod built {
        pub use crate::client::baked_models::built::{
            BakedModelFaces, BakedModelsAccess, baked_model_quad,
        };
    }
}
