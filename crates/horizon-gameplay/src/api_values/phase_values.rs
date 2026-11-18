//! Phase vocabulary enters through the same game-local conversion boundary.
//! The two mappings repeat `horizon_client_render::api_context::{load_origin, loop_side}`,
//! which this crate cannot call.
use super::TryIntoApi;
use horizon_world::native_load_context::{DataLoadOrigin, LoopSide};
use std::convert::Infallible;

impl TryIntoApi for DataLoadOrigin {
    type Api = mod_api::DataLoadOrigin;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            DataLoadOrigin::ServerDataLoad => mod_api::DataLoadOrigin::ServerDataLoad,
            DataLoadOrigin::ClientPreload => mod_api::DataLoadOrigin::ClientPreload,
            _ => unreachable!("all engine load origins are mapped"),
        })
    }
}
impl TryIntoApi for LoopSide {
    type Api = mod_api::LoopSide;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            LoopSide::Server => mod_api::LoopSide::Server,
            LoopSide::Client => mod_api::LoopSide::Client,
            _ => unreachable!("all engine loop sides are mapped"),
        })
    }
}
