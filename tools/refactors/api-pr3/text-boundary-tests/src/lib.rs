//! Isolated compilation of the actual game Text converter, without linking Game.
#![allow(dead_code)]
#[cfg(test)]
mod typed_payloads;
trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}
trait TryIntoEngine<C: ?Sized> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error>;
}
// The boundary now captures through `horizon_client_lib::api_text`, whose sources live beside
// `client_lib.rs`. Compile that module here instead of linking the whole client library, and give
// this crate the name the boundary calls it by (as `client_lib.rs` does with `horizon_client`).
extern crate self as horizon_client_lib;
#[path = "../../../../../crates/horizon-client/src/api_text.rs"]
mod api_text;
#[path = "../../../../../crates/horizon-client/src/game/modding/api_values/text_boundary.rs"]
mod text_boundary;

/// Exercise the real host-built crossing, recursive ownership, and return crossing.
pub fn measure_host_text(engine: horizon_client_core::text::Text) {
    use std::hint::black_box;
    let api = black_box(&engine).try_into_api().unwrap();
    eprintln!("captured");
    drop(black_box(api.clone()));
    eprintln!("cloned and dropped API");
    let restored = black_box(&api).try_into_engine(&()).unwrap();
    eprintln!("emitted engine");
    drop(black_box(restored.clone()));
    drop(restored);
    drop(api);
    drop(engine);
}
