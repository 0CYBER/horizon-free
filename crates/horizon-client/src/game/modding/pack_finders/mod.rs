//! Mod-supplied repository sources. Discovery owns the metadata and retained
//! readers; selection and resource/data reloads use the ordinary pack paths.
use super::{Game, api_values::TryIntoEngine, mods::ModsState};
use horizon_assets::resource_source::{ResourceSource, stays_inside};
use horizon_client::{
    datapack::DataPack,
    pack_screen::{PackInfo, Source as ScreenSource, load_icon},
    packs::{self, Pack, Position, Source},
};
use mod_api::{
    ClientHost, HookId, Host, HostSide, PackFinderError, PackFinders, PackPosition,
    PackRegistration, PackSource, PackType, ServerHost,
};

#[cfg(test)]
mod tests;

struct Registry {
    kind: PackType,
    mods: ModsState,
    packs: Vec<Pack>,
}
impl Registry {
    fn new(kind: PackType, mods: ModsState) -> Self {
        Self {
            kind,
            mods,
            packs: Vec::new(),
        }
    }
}
impl PackFinders for Registry {
    fn pack_type(&self) -> PackType {
        self.kind
    }
    fn add_pack_finders(&mut self, entry: PackRegistration) -> Result<(), PackFinderError> {
        if entry.pack_type != self.kind {
            return Ok(());
        }
        let location = &entry.location;
        if !stays_inside(location.path()) {
            return Err(PackFinderError::InvalidPath);
        }
        let id = format!("mod/{location}");
        if self.packs.iter().any(|pack| pack.id == id) {
            return Err(PackFinderError::Duplicate);
        }
        let package = self
            .mods
            .native
            .packages()
            .iter()
            .map(|package| (package.manifest.id.as_str(), &package.path))
            .chain(
                self.mods
                    .selected
                    .iter()
                    .map(|package| (package.package.manifest.id.as_str(), &package.path)),
            )
            .find(|(id, _)| *id == location.namespace())
            .map(|(_, path)| path)
            .ok_or(PackFinderError::MissingPackage)?;
        let source = if package.is_dir() {
            ResourceSource::directory(package.clone())
        } else {
            ResourceSource::archive(package).map_err(|_| PackFinderError::MissingPackage)?
        };
        let source = source
            .subdirectory(location.path())
            .map_err(|_| PackFinderError::InvalidPath)?;
        let format = match self.kind {
            PackType::ClientResources => horizon_core::version::RESOURCE_PACK_FORMAT,
            PackType::ServerData => horizon_core::version::DATA_PACK_FORMAT,
            _ => return Err(PackFinderError::InvalidMetadata),
        };
        let title = entry.title.try_into_engine(&())?;
        let mut pack = packs::pack_for(id, title, Source::Opened(source), format)
            .ok_or(PackFinderError::InvalidMetadata)?;
        pack.selection = Some(PackInfo {
            id: pack.id.clone(),
            title: pack.title.clone(),
            description: pack.raw_description.clone(),
            description_style: pack.description_style.clone(),
            source: match entry.source {
                PackSource::BuiltIn => ScreenSource::BuiltIn,
                PackSource::Feature => ScreenSource::Feature,
                _ => return Err(PackFinderError::InvalidMetadata),
            },
            compat: pack.compatibility,
            icon: pack.icon_png().as_deref().and_then(load_icon),
            required: entry.always_active,
            position: match entry.position {
                PackPosition::Top => Position::Top,
                PackPosition::Bottom => Position::Bottom,
                _ => return Err(PackFinderError::InvalidMetadata),
            },
            fixed: false,
            hidden: entry.hidden,
        });
        self.packs.push(pack);
        Ok(())
    }
}

/// A repository may be discovered before Game exists. As with startup reload
/// listeners, registration has no world reads; only the supplied builder is live.
struct DiscoveryHost;
impl Host for DiscoveryHost {}
mod_api::impl_host_areas!(DiscoveryHost);
impl ClientHost for DiscoveryHost {}
mod_api::impl_client_host_areas!(DiscoveryHost);
impl ServerHost for DiscoveryHost {}
mod_api::impl_server_host_areas!(DiscoveryHost);

pub(super) fn discover(kind: PackType, mods: &ModsState) -> Vec<Pack> {
    if !mod_api::__private::listened(HookId::AddPackFinders) {
        return Vec::new();
    }
    let mut registry = Registry::new(kind, mods.clone());
    let mut host = DiscoveryHost;
    let host = match kind {
        PackType::ClientResources => HostSide::Client(&mut host),
        PackType::ServerData => HostSide::Server(&mut host),
        _ => return Vec::new(),
    };
    mod_api::dispatch_add_pack_finders(host, &mut registry);
    registry.packs
}

pub(super) fn data_packs(mods: &ModsState) -> Vec<std::sync::Arc<DataPack>> {
    discover(PackType::ServerData, mods)
        .into_iter()
        .map(|pack| {
            let mut data =
                DataPack::from_source(pack.id, pack.source.open().expect("retained source"));
            data.selection = pack.selection;
            std::sync::Arc::new(data)
        })
        .collect()
}

/// The same repository is used by commands, content loading and world generation.
pub(super) fn data_repository(
    world: &std::path::Path,
    resources: &std::path::Path,
    additional: &[std::sync::Arc<DataPack>],
) -> Vec<DataPack> {
    let mut packs = horizon_client::datapack::discover_with_features(world, resources);
    packs.extend(additional.iter().map(|pack| (**pack).clone()));
    packs
}

impl Game<'_> {
    pub(super) fn discover_mod_packs(&mut self, kind: PackType) -> Vec<Pack> {
        if !mod_api::__private::listened(HookId::AddPackFinders) {
            return Vec::new();
        }
        let mut registry = Registry::new(kind, self.mods.clone());
        self.native_add_pack_finders(&mut registry);
        registry.packs
    }
}
