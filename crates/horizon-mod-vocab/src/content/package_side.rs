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
    /// As a package's side: only the server needs it. It installs and runs everywhere, in the
    /// integrated server too, but is never compared at join, so joining clients need not have
    /// it; it is refused at load by name when it carries what a joining client must know
    /// (registry content, a client-side hook or part, a required clientbound payload).
    /// As a dependency's side: the dedicated server installation.
    Server,
}
