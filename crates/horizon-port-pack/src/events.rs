//! `events <jar or folder>`: the NeoForge events a mod listens to and the
//! capabilities it uses, looked up in the hook catalog. The Rust hook declarations
//! name each hook's NeoForge counterpart; the generator exports them at build time;
//! this reads a jar's class files (`classes`: `@SubscribeEvent` methods,
//! method references and lambdas that take one event, `Capabilities` fields)
//! and finds the hooks for each name. Over a folder of jars it ranks the
//! names by how many mods use them, so a pack shows which missing hooks
//! matter most. It reads names, never code, and translates nothing: it says
//! where a port starts, for a human to check.
use crate::{classes, jar};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufReader, Read, Seek},
    path::{Path, PathBuf},
};

/// The hook catalog this build was made from.
pub const CATALOG: &str = include_str!(concat!(env!("OUT_DIR"), "/hook-counterparts.json"));

/// Names NeoForge changed after 1.21.1, which packs such as ATM10 still use
/// (the javadoc of NeoForge's `1.21.1` and `26.3.x` branches): a name the
/// catalog lacks is looked up again under its new one. Most kept what they do.
/// The capabilities kept their keys and took the 26.1 handler types
/// (`ResourceHandler`, `EnergyHandler`). `BlockEvent.BreakEvent` fired on the
/// server; `BreakBlockEvent` fires on both sides. `ContainerScreenEvent.Render`
/// fired for container screens; `ScreenEvent.Render.Background` and
/// `.Foreground` fire for every screen, and `.Background` is also what
/// `ScreenEvent.BackgroundRendered` became. `RenderHighlightEvent.Block` and
/// `RenderBlockScreenEffectEvent` fired before the outline or the overlay was
/// drawn; their `Extract…` successors fire when its render state is
/// extracted, and cancelling either still drops what vanilla draws.
///
/// `EntityTeleportEvent.ChorusFruit` became `EntityTeleportEvent.ItemConsumption`:
/// it now covers any `teleport_randomly` consume effect, for a living consumer
/// (player or fox), and carries the captured consumed stack.
///
/// The rest changed what they carry, and a port of them is not a rename.
/// `RecipesUpdatedEvent`: 1.21.1's documentation doesn't name it, only that
/// the client has "a full copy of the `RecipeManager`". 26.1's client gets
/// just the recipe types a mod asks for (`OnDatapackSyncEvent#sendRecipes`),
/// in `RecipesReceivedEvent`. `RenderTooltipEvent.Color` set a tooltip's
/// background and border colours; 26.1 draws a tooltip from a style's
/// textures and has only `RenderTooltipEvent.Texture`, which picks them.
/// `AnvilRepairEvent` fired when the player took a repaired result out of the
/// anvil and let a mod change the anvil's break chance; 26.1's
/// `AnvilCraftEvent` fires on that take, `.Pre` before the inputs are used
/// and the anvil is damaged, and has no break chance.
/// `RegisterColorHandlersEvent.Block` registered a `BlockColor` for a set of
/// blocks, where `.BlockTintSources` registers a list of `BlockTintSource`s;
/// `.Item` registered an `ItemColor` for a set of items, where
/// `.ItemTintSources` registers the tint source types item model files name.
/// `CustomizeGuiOverlayEvent.DebugText` could "add or remove text
/// information" on the debug screen each frame; `RegisterDebugEntriesEvent`
/// registers entries of a mod's own, and "existing entries cannot be modified
/// or amended directly". `RegisterRecipeBookCategoriesEvent` registered the
/// categories of a recipe book and a recipe type's category lookup;
/// `RegisterRecipeBookSearchCategoriesEvent` registers search categories
/// only, and the hook label that mentions it says what Horizon has instead.
/// `RegisterShadersEvent` registered shader instances; vanilla replaced those
/// with render pipelines, and `RegisterRenderPipelinesEvent` registers a
/// pipeline: its shaders with the vertex format, blend and depth state it
/// draws with.
const RENAMED: [(&str, &str); 22] = [
    ("Capabilities.ItemHandler.", "Capabilities.Item."),
    ("Capabilities.FluidHandler.", "Capabilities.Fluid."),
    ("Capabilities.EnergyStorage.", "Capabilities.Energy."),
    ("AddReloadListenerEvent", "AddServerReloadListenersEvent"),
    (
        "RegisterClientReloadListenersEvent",
        "AddClientReloadListenersEvent",
    ),
    (
        "ModelEvent.RegisterAdditional",
        "ModelEvent.RegisterStandalone",
    ),
    (
        "ModelEvent.RegisterGeometryLoaders",
        "ModelEvent.RegisterLoaders",
    ),
    ("BlockEvent.BreakEvent", "BreakBlockEvent"),
    ("EnderManAngerEvent", "EndermanAngerEvent"),
    (
        "EntityTeleportEvent.ChorusFruit",
        "EntityTeleportEvent.ItemConsumption",
    ),
    ("ContainerScreenEvent.Render.", "ScreenEvent.Render."),
    (
        "ScreenEvent.BackgroundRendered",
        "ScreenEvent.Render.Background",
    ),
    (
        "RenderHighlightEvent.Block",
        "ExtractBlockOutlineRenderStateEvent",
    ),
    (
        "RenderBlockScreenEffectEvent",
        "ExtractBlockScreenEffectEvent",
    ),
    ("RecipesUpdatedEvent", "RecipesReceivedEvent"),
    ("RenderTooltipEvent.Color", "RenderTooltipEvent.Texture"),
    ("AnvilRepairEvent", "AnvilCraftEvent.Pre"),
    (
        "RegisterColorHandlersEvent.Block",
        "RegisterColorHandlersEvent.BlockTintSources",
    ),
    (
        "RegisterColorHandlersEvent.Item",
        "RegisterColorHandlersEvent.ItemTintSources",
    ),
    (
        "CustomizeGuiOverlayEvent.DebugText",
        "RegisterDebugEntriesEvent",
    ),
    (
        "RegisterRecipeBookCategoriesEvent",
        "RegisterRecipeBookSearchCategoriesEvent",
    ),
    ("RegisterShadersEvent", "RegisterRenderPipelinesEvent"),
];

/// Types nested in an event that listener-shaped code takes without being
/// events. `RegisterEvent.register` takes a `Consumer<RegisterHelper<T>>`, so
/// a mod's lambda for it reads as a listener of the helper. Each is answered
/// as the event it belongs to.
const HELPERS: [(&str, &str); 1] = [("RegisterEvent.RegisterHelper", "RegisterEvent")];

/// Events that are only a parent of the events NeoForge documents under
/// them (its config page: "the `ModConfigEvent.Loading`,
/// `ModConfigEvent.Reloading`, and `ModConfigEvent.Unloading` events"). A
/// mod that listens to the parent hears each of them, so the parent is
/// hooked when every one of them is.
const PARENTS: [(&str, &[&str]); 1] = [("ModConfigEvent", &["Loading", "Reloading", "Unloading"])];

/// Names the mod API answers outside the hook catalog, with what a port uses
/// instead. A registration event NeoForge fires is a `Registrar` method a mod
/// calls from `Hooks::register`; a NeoForge capability is the capability key
/// a mod asks or answers by. The rustdoc of each method and key in
/// `horizon-mod-api` names the NeoForge name it stands under here.
const API: [(&str, &str); 15] = [
    (
        "RegisterKeyMappingsEvent",
        "Registrar::register_key_mapping",
    ),
    (
        "EntityAttributeCreationEvent",
        "Registrar::declare_entity_attributes",
    ),
    (
        "EntityAttributeModificationEvent",
        "Registrar::modify_entity_attributes",
    ),
    (
        "RegisterSpawnPlacementsEvent",
        "Registrar::declare_spawn_placement",
    ),
    (
        "RegisterTicketControllersEvent",
        "Registrar::register_ticket_controller",
    ),
    ("Capabilities.Item.BLOCK", "mod_api::ITEMS"),
    ("Capabilities.Item.ENTITY", "mod_api::ENTITY_ITEMS"),
    (
        "Capabilities.Item.ENTITY_AUTOMATION",
        "mod_api::ENTITY_AUTOMATION",
    ),
    ("Capabilities.Item.ITEM", "mod_api::ITEM_ITEMS"),
    ("Capabilities.Fluid.BLOCK", "mod_api::FLUIDS"),
    ("Capabilities.Fluid.ENTITY", "mod_api::ENTITY_FLUIDS"),
    ("Capabilities.Fluid.ITEM", "mod_api::ITEM_FLUIDS"),
    ("Capabilities.Energy.BLOCK", "mod_api::ENERGY"),
    ("Capabilities.Energy.ENTITY", "mod_api::ENTITY_ENERGY"),
    ("Capabilities.Energy.ITEM", "mod_api::ITEM_ENERGY"),
];

/// Events NeoForge dropped after 1.21.1 because vanilla made what they built
/// into data, with what a port writes instead (from a native mod's data, or
/// `data_pack_files`): registries, or for fuel an item component. NeoForge's
/// 26.1 primer: "Villager trades has now become a data-generated registry
/// from its original map-based setup", in `trade_set` and `villager_trade`;
/// 26.3's javadoc has neither event. Its `26.3.x` branch has none of the
/// other three either. 26.3 brews from `minecraft:brewing` recipe files
/// (`recipe/brewing/`, in any namespace) and has no `PotionBrewing` for
/// `RegisterBrewingRecipesEvent` to add mixes to. A dimension's sky, fog and
/// clouds are the `minecraft:visual/*` attributes of its `dimension_type`,
/// where `RegisterDimensionSpecialEffectsEvent` registered a
/// `DimensionSpecialEffects` for the type's `effects` id. An item's burn time
/// is its `minecraft:cooking_fuel` component, which `FurnaceFuelBurnTimeEvent`
/// let a mod override for each stack.
const DATA: [(&str, &[&str]); 5] = [
    ("VillagerTradesEvent", &["trade_set", "villager_trade"]),
    ("WandererTradesEvent", &["trade_set", "villager_trade"]),
    ("RegisterBrewingRecipesEvent", &["recipe"]),
    ("RegisterDimensionSpecialEffectsEvent", &["dimension_type"]),
    ("FurnaceFuelBurnTimeEvent", &["minecraft:cooking_fuel"]),
];

/// Mod-loading events Horizon has no hook for by design, and where a port
/// does their work. The modding book's load phases: "There is no separate
/// common-setup, load-complete or intermod event: registration and the first
/// server reload cover common setup, client setup is `client_setup`,
/// dedicated setup is `server_about_to_start`, and dependencies express
/// intermod ordering. Load complete is the edge after initial Config and
/// before `server_about_to_start`, not a hook." Its mod extensions design
/// drops `InterModComms` (sent in `InterModEnqueueEvent`, read in
/// `InterModProcessEvent`): "IMC isn't needed, because dependents call the
/// library directly in dependency order". The book doesn't name
/// `FMLConstructModEvent`, which FML fires once the mods are constructed,
/// ahead of the registry events; Registration, where `Hooks::register` runs,
/// is the first phase the book gives a mod declarations in.
const LOADING: [(&str, &str); 6] = [
    ("FMLConstructModEvent", "`Hooks::register`"),
    (
        "FMLCommonSetupEvent",
        "`Hooks::register`, then the first server reload",
    ),
    (
        "FMLDedicatedServerSetupEvent",
        "`server_about_to_start`, when `dedicated`",
    ),
    (
        "FMLLoadCompleteEvent",
        "after the initial config, before `server_about_to_start`",
    ),
    ("InterModEnqueueEvent", "a direct call, in dependency order"),
    ("InterModProcessEvent", "a direct call, in dependency order"),
];

/// Events with no hook because nothing in the game would fire one, and why.
/// `GatherDataEvent` is "fired on the mod-specific event bus, during data
/// generation" (its javadoc): it runs in the mod developer's data generator,
/// and what that writes ships in the jar as the data and assets a port
/// carries. The others are 1.21.1 events NeoForge's `26.3.x` branch no longer
/// has. `IdMappingEvent` was "no longer fired" in 1.21.1 already (its
/// javadoc). `RegisterNamedRenderTypesEvent` named groups of render types.
/// `RenderLevelStageEvent.RegisterStageEvent` added stages of a mod's own;
/// 26.3's stages are the events nested in `RenderLevelStageEvent`
/// (`AfterSky` to `AfterLevel`).
const UNHOOKED: [(&str, &str); 4] = [
    (
        "GatherDataEvent",
        "data generation runs at development time, not in the game",
    ),
    ("IdMappingEvent", "not in NeoForge 26.3"),
    ("RegisterNamedRenderTypesEvent", "not in NeoForge 26.3"),
    (
        "RenderLevelStageEvent.RegisterStageEvent",
        "not in NeoForge 26.3",
    ),
];

/// The catalog's NeoForge names: the hooks that are each name's
/// counterpart, and the hooks whose `none (…)` label mentions it.
#[derive(Debug, Default)]
pub struct Catalog {
    exact: BTreeMap<String, Vec<String>>,
    noted: BTreeMap<String, Vec<String>>,
}

/// How the catalog answers one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The hooks whose counterpart it is (for a parent in `PARENTS`, the
    /// hooks of all its events).
    Hooked(Vec<String>),
    /// The hooks of another phase (`.Post` for `.Pre`), or of the events
    /// nested in it (`ModConfigEvent.Loading` for `ModConfigEvent`).
    Phase(Vec<String>),
    /// The hooks whose label says why they aren't its counterpart.
    Noted(Vec<String>),
    /// No hook is needed: the registries or the component (`DATA`) that
    /// took the event's place.
    Data(Vec<String>),
    /// No hook, but another part of the mod API (`API`): the `Registrar`
    /// method or the capability key a port uses.
    Api(String),
    /// No hook by design: where the load phases (`LOADING`) do the event's
    /// work.
    Loading(String),
    /// No hook, and none to add (`UNHOOKED`): why.
    Unhooked(String),
    Missing,
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Catalog, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("hook catalog: {e}"))?;
        let mut catalog = Catalog::default();
        for hook in value["hooks"].as_array().ok_or("hook catalog: no hooks")? {
            let (Some(name), Some(label)) = (hook["name"].as_str(), hook["neoforge"].as_str())
            else {
                continue;
            };
            let (names, map) = if label.to_ascii_lowercase().starts_with("none") {
                (mentioned(label), &mut catalog.noted)
            } else {
                (counterparts(label), &mut catalog.exact)
            };
            for n in names {
                map.entry(n).or_default().push(name.to_owned());
            }
        }
        Ok(catalog)
    }

    /// The status of `name` (`ServerTickEvent.Post`), and the name it was
    /// found under: its newer one (`RENAMED`), or its event's (`HELPERS`).
    pub fn status(&self, name: &str) -> (Status, Option<String>) {
        let status = self.answer(name);
        if status != Status::Missing {
            return (status, None);
        }
        for (old, new) in RENAMED.iter().chain(&HELPERS) {
            if let Some(rest) = name.strip_prefix(old) {
                let other = format!("{new}{rest}");
                let status = self.answer(&other);
                if status != Status::Missing {
                    return (status, Some(other));
                }
            }
        }
        (Status::Missing, None)
    }

    /// The catalog's answer for `name` as it is written, else the tables'.
    fn answer(&self, name: &str) -> Status {
        let status = self.lookup(name);
        if status != Status::Missing {
            return status;
        }
        if let Some((_, written)) = DATA.iter().find(|(event, _)| *event == name) {
            let written = written.iter().map(|w| (*w).to_owned()).collect();
            return Status::Data(written);
        }
        let said = |table: &[(&str, &str)]| {
            let found = table.iter().find(|(event, _)| *event == name);
            found.map(|(_, said)| (*said).to_owned())
        };
        if let Some(what) = said(&API) {
            return Status::Api(what);
        }
        if let Some(place) = said(&LOADING) {
            return Status::Loading(place);
        }
        said(&UNHOOKED).map_or(Status::Missing, Status::Unhooked)
    }

    fn lookup(&self, name: &str) -> Status {
        if let Some(hooks) = self.exact.get(name) {
            return Status::Hooked(hooks.clone());
        }
        if let Some((_, events)) = PARENTS.iter().find(|(parent, _)| *parent == name) {
            let hooks: Option<Vec<&Vec<String>>> = events
                .iter()
                .map(|event| self.exact.get(&format!("{name}.{event}")))
                .collect();
            if let Some(hooks) = hooks {
                let hooks: BTreeSet<&String> = hooks.into_iter().flatten().collect();
                return Status::Hooked(hooks.into_iter().cloned().collect());
            }
        }
        let phase = |s: &str| s == "Pre" || s == "Post";
        let related = |k: &str| {
            let nested = k
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('.'));
            let sibling = match (name.rsplit_once('.'), k.rsplit_once('.')) {
                (Some((p, a)), Some((q, b))) => p == q && phase(a) && phase(b),
                _ => false,
            };
            nested || sibling
        };
        let hooks: BTreeSet<&String> = self
            .exact
            .iter()
            .filter(|(k, _)| related(k))
            .flat_map(|(_, hooks)| hooks)
            .collect();
        if !hooks.is_empty() {
            return Status::Phase(hooks.into_iter().cloned().collect());
        }
        match self.noted.get(name) {
            Some(hooks) => Status::Noted(hooks.clone()),
            None => Status::Missing,
        }
    }
}

/// An event (`…Event`, nested or not) or a capability (`Capabilities.X.Y`).
fn is_name(s: &str) -> bool {
    let first = s.split('.').next().unwrap_or_default();
    (first.ends_with("Event") || first == "Capabilities")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// The names a label gives (`ServerTickEvent.Pre / .Post`, or
/// `EntityJoinLevelEvent, FinalizeSpawnEvent`): notes in brackets and a
/// leading package dropped, a name that starts with `.` taken as a sibling
/// of the one before it.
fn counterparts(label: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0u32;
    for c in label.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let mut out: Vec<String> = Vec::new();
    for part in plain
        .split(" / ")
        .flat_map(|part| part.split(','))
        .map(str::trim)
    {
        let full = match (part.strip_prefix('.'), out.last()) {
            (Some(rest), Some(prev)) => match prev.rsplit_once('.') {
                Some((parent, _)) => format!("{parent}.{rest}"),
                None => continue,
            },
            _ => part
                .split('.')
                .skip_while(|s| s.starts_with(|c: char| c.is_ascii_lowercase()))
                .collect::<Vec<_>>()
                .join("."),
        };
        if is_name(&full) {
            out.push(full);
        }
    }
    out
}

/// The names a `none (…)` label mentions.
fn mentioned(label: &str) -> Vec<String> {
    label
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
        .map(|w| w.trim_end_matches('.'))
        .filter(|w| is_name(w))
        .map(str::to_owned)
        .collect()
}

/// The events one jar's classes listen to and the capabilities they use,
/// as the catalog names them (`ServerTickEvent.Post`,
/// `Capabilities.ItemHandler.BLOCK`). Jars inside the jar aren't opened.
pub fn scan<R: Read + Seek>(jar: R) -> Result<BTreeSet<String>, String> {
    let mut zip = crate::zip_intake::open(jar).map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.name().is_ok_and(|n| n.ends_with(".class")) || entry.size() >= 1 << 24 {
            continue;
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        if let Some(class) = classes::parse(&bytes) {
            names.extend(
                class
                    .listens
                    .iter()
                    .map(|c| c.rsplit('/').next().unwrap_or(c).replace('$', ".")),
            );
            names.extend(class.capabilities);
        }
    }
    Ok(names)
}

/// The report for the jar at `path`, or for every jar in the folder at
/// `path`: each name, how many mods use it, its hooks and a few of the mods.
pub fn report(path: &Path, catalog: &str) -> Result<String, String> {
    let catalog = Catalog::parse(catalog)?;
    let jars: Vec<PathBuf> = if path.is_dir() {
        let mut jars: Vec<PathBuf> = fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jar"))
            .collect();
        jars.sort();
        jars
    } else {
        vec![path.to_path_buf()]
    };
    let mut mods: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unread = Vec::new();
    for jar_path in &jars {
        let names = fs::File::open(jar_path)
            .map_err(|e| e.to_string())
            .and_then(|f| scan(BufReader::new(f)));
        let names = match names {
            Ok(names) => names,
            Err(e) if jars.len() == 1 => return Err(format!("{}: {e}", jar_path.display())),
            Err(_) => {
                unread.push(jar_path.display().to_string());
                continue;
            }
        };
        let id = jar::read(jar_path)
            .ok()
            .map(|info| info.mod_id)
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| {
                let stem = jar_path.file_stem().unwrap_or_default();
                stem.to_string_lossy().into_owned()
            });
        mods.entry(id).or_default().extend(names);
    }
    let mut out = table(&mods, &catalog);
    if !unread.is_empty() {
        out += &format!("\nNot read as jars: {}.\n", unread.join(", "));
    }
    Ok(out)
}

/// The ranked table over `mods` (mod id to the names it uses).
pub fn table(mods: &BTreeMap<String, BTreeSet<String>>, catalog: &Catalog) -> String {
    let mut users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, names) in mods {
        for n in names {
            users.entry(n).or_default().push(id);
        }
    }
    let mut rows: Vec<(&str, Vec<&str>)> = users.into_iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let code = |hooks: &[String]| {
        let hooks: Vec<String> = hooks.iter().map(|h| format!("`{h}`")).collect();
        hooks.join(", ")
    };
    let mut counts = [0usize; 8];
    let mut body = String::new();
    for (name, ids) in &rows {
        let (status, under) = catalog.status(name);
        let (i, hooks) = match &status {
            Status::Hooked(h) => (0, code(h)),
            Status::Phase(h) => (1, format!("other phase: {}", code(h))),
            Status::Noted(h) => (2, format!("not quite: {}", code(h))),
            Status::Data(r) => (3, format!("data: {}", code(r))),
            Status::Api(what) => (4, format!("mod API: `{what}`")),
            Status::Loading(place) => (5, format!("load phases: {place}")),
            Status::Unhooked(why) => (6, format!("no hook: {why}")),
            Status::Missing => (7, "none".to_owned()),
        };
        counts[i] += 1;
        let under = under.map(|u| format!(" (as `{u}`)")).unwrap_or_default();
        let more = if ids.len() > 3 { ", …" } else { "" };
        let some = ids[..ids.len().min(3)].join(", ");
        body += &format!(
            "| {} | `{name}` | {hooks}{under} | {some}{more} |\n",
            ids.len()
        );
    }
    let listening = mods.values().filter(|names| !names.is_empty()).count();
    format!(
        "{} mods; {listening} listen to NeoForge events or use its capabilities.\n\
         {} names: {} with a hook, {} with another phase's, {} noted as not quite one, {} made data, {} answered elsewhere in the mod API, {} answered by the load phases, {} that need no hook, {} with none.\n\n\
         | Mods | NeoForge | Hooks | Used by |\n|---:|---|---|---|\n{body}",
        mods.len(),
        rows.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        counts[5],
        counts[6],
        counts[7],
    )
}
