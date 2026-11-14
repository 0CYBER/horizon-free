/// Which installations must carry a package, not the executing host's side.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Side {
    /// Local content, not compared with the server's mods when joining.
    #[default]
    Client,
    /// Both installations; the server tells clients which version it expects.
    Both,
    /// The dedicated server installation, not an integrated server on a client.
    Server,
}
