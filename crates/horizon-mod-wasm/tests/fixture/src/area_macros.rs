//! Compile the public area macros from a wasm mod, outside the API crate.

pub struct MacroHost;
impl mod_api::Host for MacroHost {}
mod_api::impl_host_areas!(MacroHost);
impl mod_api::ServerHost for MacroHost {}
mod_api::impl_server_host_areas!(MacroHost);
impl mod_api::ClientHost for MacroHost {}
mod_api::impl_client_host_areas!(MacroHost);
