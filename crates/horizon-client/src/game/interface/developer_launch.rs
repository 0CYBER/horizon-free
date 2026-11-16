//! Developer single-player startup and network-entry policy.
use super::lan::LanSettings;
use super::*;
use horizon_client::lan::{NetConfig, hub::HubConfig};

impl Options {
    pub(super) fn validate_developer_single_player(&self) -> Result<()> {
        if self.developer_single_player
            && (self.server.is_some()
                || self.session_stdin
                || self.session_file.is_some()
                || !self.execution_role.is_client())
        {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        Ok(())
    }
}

impl Game<'_> {
    /// Start the network side and the play-state hub: for the LAN (with the
    /// host's own profile reserved) or for a dedicated server (no host).
    pub(super) fn lan_host(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
    ) -> Result<()> {
        if self.options.developer_single_player && net.listen {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        self.start_lan_hub(settings, net, config)
    }
    /// Join single player's integrated server through memory.
    pub(super) fn connect_local(
        &mut self,
        pipe: horizon_protocol::connection::Pipe,
        name: &str,
    ) -> Result<()> {
        self.join_server("local", name, Some(pipe), None, false, false)
    }
    pub(super) fn join_server(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        if local.is_none() && self.options.developer_single_player {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        self.connect_remote_session(address, name, local, cookies, lan, realm)
    }
}
