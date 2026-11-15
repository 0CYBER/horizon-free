//! Layers on living entity renderers, added by mods on the client: `native_register_entity_layers`
//! (NeoForge's `EntityRenderersEvent.AddLayers`) is asked on every resource load, after the entity
//! renderers resolve, and the layers it collects are resolved to the slot each draw reads: a mob's
//! type (its shared `MobType`, which the game's types never replace while it runs) or a player's
//! model. A frame then hands an entity to its own target's layers, after its model and the vanilla
//! layers, each through the guard renderers draw through ([`Game::guarded_entity_draw`]). With no
//! layer at all, a frame checks one flag per entity and builds nothing.
use super::{
    Game,
    entity_type_rows::EntityTypeRows,
    native_entity_renderers::Resolved,
    native_mob_draw::{entity_render_state, mob_pose},
};
use glam::DVec3;
use horizon_client::{
    fast_hash::FxHashMap,
    mesher::Vertex,
    mobs::{Mob, MobType},
};
use mod_api::{
    __private::game::RegistrationContextNew, EntityLayers, EntityRenderLayer, EntityRenderer,
    LayerTarget, PlayerModel, PosedParts, RegistrationContext,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// The player's entity type: its renderers are the player models', so its layers go to those.
const PLAYER: &str = "minecraft:player";

/// One target's layers, in the order they were added.
pub(in crate::game) type Layers = Arc<[Resolved<dyn EntityRenderLayer>]>;

/// Whether the last load resolved any layer: a frame checks only this when none was added.
static ANY: AtomicBool = AtomicBool::new(false);

/// The table the last resource load resolved; none before the first.
static TABLE: RwLock<Option<LayerTable>> = RwLock::new(None);

/// What one `register_entity_layers` round collects: the [`EntityLayers`] mods are handed, with
/// what this load's renderers are, and the layers added to each target.
pub(super) struct LayerRegistration<'a> {
    rows: &'a EntityTypeRows,
    mod_renderers: &'a BTreeMap<String, Arc<dyn EntityRenderer>>,
    /// Whether a living type draws where layers run (as a mob).
    routed: &'a dyn Fn(&str) -> bool,
    entity: BTreeMap<String, Vec<Arc<dyn EntityRenderLayer>>>,
    wide: Vec<Arc<dyn EntityRenderLayer>>,
    slim: Vec<Arc<dyn EntityRenderLayer>>,
}

impl<'a> LayerRegistration<'a> {
    pub(super) fn new(
        rows: &'a EntityTypeRows,
        mod_renderers: &'a BTreeMap<String, Arc<dyn EntityRenderer>>,
        routed: &'a dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            rows,
            mod_renderers,
            routed,
            entity: BTreeMap::new(),
            wide: Vec::new(),
            slim: Vec::new(),
        }
    }

    /// Why `id` can't take a layer; none for a living entity type.
    fn refusal(&self, id: &str) -> Option<&'static str> {
        if id == PLAYER {
            Some("a player takes layers through its model (wide or slim)")
        } else if !self.rows.contains(id) {
            Some("it is no entity type")
        } else if !self.rows.is_living(id) {
            Some("its renderer is not a living entity's, and only those take layers")
        } else if !(self.routed)(id) {
            Some("it draws through a route of its own, which does not run layers yet")
        } else {
            None
        }
    }
}

impl EntityLayers for LayerRegistration<'_> {
    fn entity_types(&self) -> Vec<String> {
        self.rows
            .ids
            .iter()
            .filter(|id| *id != PLAYER)
            .cloned()
            .collect()
    }

    fn is_living(&self, entity_type: &str) -> bool {
        entity_type != PLAYER && self.rows.is_living(entity_type)
    }

    fn player_models(&self) -> Vec<PlayerModel> {
        vec![PlayerModel::Wide, PlayerModel::Slim]
    }

    fn has_mod_renderer(&self, entity_type: &str) -> bool {
        self.rows.contains(entity_type) && self.mod_renderers.contains_key(entity_type)
    }

    fn add(
        &mut self,
        target: LayerTarget,
        layer: Arc<dyn EntityRenderLayer>,
    ) -> Result<(), String> {
        match target {
            LayerTarget::Player(PlayerModel::Wide) => self.wide.push(layer),
            LayerTarget::Player(PlayerModel::Slim) => self.slim.push(layer),
            LayerTarget::Entity(id) => {
                if let Some(why) = self.refusal(&id) {
                    let why = format!("the entity layer for {id} is refused: {why}");
                    log::warn!("Horizon: {why}");
                    return Err(why);
                }
                self.entity.entry(id).or_default().push(layer);
            }
        }
        Ok(())
    }
}

/// The layers by the slot each draw reads: rebuilt on every resource load.
#[derive(Default)]
pub(super) struct LayerTable {
    /// By the address of the mob type the game shares with every mob of the type.
    mobs: FxHashMap<usize, Layers>,
    wide: Option<Layers>,
    slim: Option<Layers>,
}

impl LayerTable {
    /// The table for what `registration` collected, with `mob_types` the types the game draws as
    /// mobs (the only living types `add` accepts).
    pub(super) fn resolve(
        registration: LayerRegistration<'_>,
        mob_types: &HashMap<String, Arc<MobType>>,
    ) -> Self {
        let resolved = |layers: Vec<Arc<dyn EntityRenderLayer>>| -> Option<Layers> {
            (!layers.is_empty()).then(|| layers.into_iter().map(Resolved::new).collect())
        };
        let mobs = registration
            .entity
            .into_iter()
            .filter_map(|(id, layers)| {
                let kind = mob_types.get(&id)?;
                Some((Arc::as_ptr(kind) as usize, resolved(layers)?))
            })
            .collect();
        Self {
            mobs,
            wide: resolved(registration.wide),
            slim: resolved(registration.slim),
        }
    }

    fn is_empty(&self) -> bool {
        self.mobs.is_empty() && self.wide.is_none() && self.slim.is_none()
    }

    /// The layers of `mob`: its player model's for a player, else its type's.
    pub(super) fn of(&self, mob: &Mob) -> Option<&Layers> {
        match (&*mob.kind.id == PLAYER, mob.slim) {
            (true, true) => self.slim.as_ref(),
            (true, false) => self.wide.as_ref(),
            (false, _) => self.mobs.get(&(Arc::as_ptr(&mob.kind) as usize)),
        }
    }
}

impl Game<'_> {
    /// The layers the last load resolved for `mob` (a player's model's for a player), looked up
    /// once per mob a frame: the answer both decides whether the draw keeps the mob's posed parts
    /// and is what `mod_layer_geometry` draws. With no layer added at all it is none after one
    /// flag check, with no lock and no lookup.
    pub(in crate::game) fn mod_layers(&self, mob: &Mob) -> Option<Layers> {
        if !ANY.load(Ordering::Relaxed) {
            return None;
        }
        let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
        table.as_ref()?.of(mob).cloned()
    }

    /// Asks the mods for their layers on the renderers this load built (`rows` its entity types,
    /// `mod_renderers` the registered renderers) and resolves them.
    pub(super) fn resolve_entity_layers(
        &self,
        rows: &EntityTypeRows,
        mod_renderers: &BTreeMap<String, Arc<dyn EntityRenderer>>,
    ) {
        let mob_types = self.res.mob_types();
        let routed = |id: &str| mob_types.contains_key(id);
        let mut registration = LayerRegistration::new(rows, mod_renderers, &routed);
        Self::native_register_entity_layers(&RegistrationContext::new(), &mut registration);
        let table = LayerTable::resolve(registration, mob_types);
        let any = !table.is_empty();
        *TABLE.write().unwrap_or_else(PoisonError::into_inner) = Some(table);
        ANY.store(any, Ordering::Relaxed);
    }

    /// Draws `layers` over `mob`, told as the entity `id`, each from its frame (at the feet,
    /// turned with the body), in the order they were added; `parts` are its humanoid parts as the
    /// vanilla draw posed them.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn registered_layer_geometry(
        &self,
        mob: &Mob,
        id: u32,
        layers: &Layers,
        parts: Option<&PosedParts>,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let entity = entity_render_state(mob, id, partial);
        let feet = mob.previous.lerp(mob.position, f64::from(partial));
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let pose = mob_pose(
            feet - camera,
            entity.living.expect("mob living state").body_rot,
        );
        let block = [feet.x, feet.y, feet.z].map(|c| c.floor() as i32);
        for layer in layers.iter() {
            self.guarded_entity_draw(&layer.failed, partial, block, pose, light, out, |draw| {
                layer.inner.submit(&entity, parts, draw)
            });
        }
    }
}
