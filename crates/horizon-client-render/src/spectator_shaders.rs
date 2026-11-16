//! The screen effect spectating an entity puts on (`GameRenderer.checkEntityPostEffect`):
//! vanilla's table and the chains mods register (`Hooks::register_entity_spectator_shaders`),
//! resolved once at start; a lookup builds no name and allocates nothing.
use mod_api::{EntitySpectatorShader, Identifier};
use std::{collections::HashMap, sync::OnceLock};

/// Vanilla's spectated entity types and their chains. `CaveSpider` extends `Spider`.
const VANILLA: [(&str, &str); 4] = [
    ("minecraft:creeper", "minecraft:creeper"),
    ("minecraft:spider", "minecraft:spider"),
    ("minecraft:cave_spider", "minecraft:spider"),
    ("minecraft:enderman", "minecraft:invert"),
];

/// The chains by spectated entity type: vanilla's, then the mods' that registered first.
#[derive(Debug)]
pub struct SpectatorShaders {
    by_entity: HashMap<String, String>,
    /// Every chain once, vanilla's first: what the renderer compiles ahead.
    chains: Vec<String>,
}

static SHADERS: OnceLock<SpectatorShaders> = OnceLock::new();

impl Default for SpectatorShaders {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl SpectatorShaders {
    /// Vanilla's table, then each registration whose ids are namespaced and whose entity type
    /// has no chain yet; the others are logged and skipped.
    pub fn new(registrations: Vec<EntitySpectatorShader>) -> Self {
        let mut result = Self {
            by_entity: HashMap::new(),
            chains: Vec::new(),
        };
        for (entity, chain) in VANILLA {
            result.insert(entity.to_owned(), chain.to_owned());
        }
        for EntitySpectatorShader { entity, shader } in registrations {
            if Identifier::parse_namespaced(&entity).is_none()
                || Identifier::parse_namespaced(&shader).is_none()
            {
                log::warn!("Invalid entity spectator shader registration: {entity} -> {shader}");
                continue;
            }
            if let Some(kept) = result.by_entity.get(&entity) {
                log::warn!(
                    "Entity spectator shader {shader} for {entity} skipped: {kept} was registered first"
                );
                continue;
            }
            result.insert(entity, shader);
        }
        result
    }

    fn insert(&mut self, entity: String, chain: String) {
        if !self.chains.contains(&chain) {
            self.chains.push(chain.clone());
        }
        self.by_entity.insert(entity, chain);
    }

    /// The chain spectating an entity of type `entity` puts on (`minecraft:creeper`, or
    /// `creeper` with the default namespace), if any.
    pub fn chain(&self, entity: &str) -> Option<&str> {
        let found = if entity.contains(':') {
            self.by_entity.get(entity)
        } else {
            self.by_entity
                .iter()
                .find(|(id, _)| id.strip_prefix("minecraft:") == Some(entity))
                .map(|(_, chain)| chain)
        };
        found.map(String::as_str)
    }

    /// Every chain once, vanilla's first.
    pub fn chains(&self) -> impl Iterator<Item = &str> {
        self.chains.iter().map(String::as_str)
    }
}

/// The registered table, or vanilla's until registration publishes one: a lookup never claims
/// the registered slot, so registration at start always lands.
pub(crate) fn table() -> &'static SpectatorShaders {
    static VANILLA_ONLY: OnceLock<SpectatorShaders> = OnceLock::new();
    SHADERS
        .get()
        .unwrap_or_else(|| VANILLA_ONLY.get_or_init(SpectatorShaders::default))
}

/// Publish the client registrations once at startup. Before it, and with no registrations,
/// vanilla's table answers.
pub fn register(registrations: Vec<EntitySpectatorShader>) {
    if SHADERS.set(SpectatorShaders::new(registrations)).is_err() {
        log::warn!("Entity spectator shaders already registered; keeping the first table");
    }
}

/// The chain spectating an entity of type `entity` puts on.
pub fn chain(entity: &str) -> Option<&'static str> {
    table().chain(entity)
}

/// Every spectator chain once, vanilla's first, for the renderer to compile ahead: the first
/// frame through a creeper's eyes used to compile its pipelines, a 0.1-0.9 s freeze on the
/// Mac mini.
pub fn chains() -> impl Iterator<Item = &'static str> {
    table().chains()
}

/// Entities whose registration requires this chain, for load diagnostics only.
impl SpectatorShaders {
    pub(crate) fn entities_for_chain(&self, id: &str) -> Vec<&str> {
        self.by_entity
            .iter()
            .filter_map(|(entity, chain)| (chain == id).then_some(entity.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader(entity: &str, shader: &str) -> EntitySpectatorShader {
        EntitySpectatorShader {
            entity: entity.into(),
            shader: shader.into(),
        }
    }

    #[test]
    fn vanilla_answers_with_no_registrations() {
        let table = SpectatorShaders::default();
        assert_eq!(table.chain("minecraft:creeper"), Some("minecraft:creeper"));
        assert_eq!(table.chain("minecraft:spider"), Some("minecraft:spider"));
        assert_eq!(
            table.chain("minecraft:cave_spider"),
            Some("minecraft:spider")
        );
        assert_eq!(table.chain("minecraft:enderman"), Some("minecraft:invert"));
        assert_eq!(table.chain("enderman"), Some("minecraft:invert"));
        assert_eq!(table.chain("minecraft:cow"), None);
        assert_eq!(
            table.chains().collect::<Vec<_>>(),
            ["minecraft:creeper", "minecraft:spider", "minecraft:invert"]
        );
    }

    #[test]
    fn the_first_registration_wins_vanillas_counting_first() {
        let table = SpectatorShaders::new(vec![
            shader("example:wisp", "example:wisp_vision"),
            // A second mod for the same type, and one for a vanilla mob: skipped.
            shader("example:wisp", "other:glow"),
            shader("minecraft:creeper", "other:pink"),
            // Unnamespaced or malformed ids: skipped.
            shader("ghost", "example:fog"),
            shader("example:ghost", "not an id"),
            shader("example:shade", "example:wisp_vision"),
        ]);
        assert_eq!(table.chain("example:wisp"), Some("example:wisp_vision"));
        assert_eq!(table.chain("example:shade"), Some("example:wisp_vision"));
        assert_eq!(table.chain("minecraft:creeper"), Some("minecraft:creeper"));
        assert_eq!(table.chain("example:ghost"), None);
        assert_eq!(table.chain("ghost"), None);
        // Each chain once, for the renderer to compile ahead.
        assert_eq!(
            table.chains().collect::<Vec<_>>(),
            [
                "minecraft:creeper",
                "minecraft:spider",
                "minecraft:invert",
                "example:wisp_vision"
            ]
        );
    }
}
