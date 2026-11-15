#[cfg(test)]
pub(crate) struct NoHost;
#[cfg(test)]
impl Host for NoHost {}
#[cfg(test)]
mod_api::impl_host_areas!(NoHost);
#[cfg(test)]
impl ServerHost for NoHost {}
#[cfg(test)]
mod_api::impl_server_host_areas!(NoHost);
#[cfg(test)]
impl ClientHost for NoHost {}
#[cfg(test)]
mod_api::impl_client_host_areas!(NoHost);
