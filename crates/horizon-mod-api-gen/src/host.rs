//! The host traits: the area files under `horizon-mod-api/src/host/` are the hand-written
//! declarations of what the game implements for mods. This reads them and writes the unions
//! that name them (`Host`, `ServerHost`, `ClientHost` and the hidden backends), the handles'
//! forwarding methods and the mob tick's goal view.
use super::*;
use heck::ToUpperCamelCase;
use std::fmt::Write as _;

/// Reads implemented by the scoped goal context over the active mob and its
/// disjoint live mob loans. Players are read from the game's backend; other
/// entity reads not supported by the scoped context remain typed refusals.
pub(super) const GOAL_VIEW_READS: &[&str] = &[
    "entity_position",
    "entity_block_position",
    "entity_velocity",
    "entity_rotation",
    "entity_head_yaw",
    "entity_bounding_box",
    "entity_eye_height",
    "entity_eye_position",
    "entity_distance_to_sqr",
    "entity_on_ground",
    "entity_kind",
    "living_health",
    "living_max_health",
    "living_alive",
    "merchant_offers",
    "merchant_trading_player",
    "merchant_profession",
    "merchant_level",
    "merchant_villager_type",
    "merchant_xp",
    "entity_dismount_location",
    "entity_root_vehicle",
    "entity_is_passenger_of_same_vehicle",
    "entity_passengers",
    "entity_is_vehicle",
    "entity_indirect_passengers",
    "entity_controlling_passenger",
    "entity_on_block",
    "entity_entities_team_damage_allowed",
    "living_can_use_portal",
    "entity_tags",
    "entity_name",
    "entity_added_to_level",
    "entity_portal_cooldown",
    "entity_invulnerable_to_damage",
    "living_is_climbing",
    "living_last_attacker",
    "living_kill_credit",
    "entity_direction",
    "entity_pose",
    "entity_is_crouching",
    "entity_view_vector",
    "entity_up_vector",
    "entity_level",
    "entity_vehicle",
    "entity_is_passenger",
    "entity_sound_source",
    "entity_leash_holder",
    "entity_pick",
    "living_attribute_value",
    "living_attribute_base",
    "living_attributes",
    "living_attribute_modifiers",
    "living_used_item_hand",
    "mob_can_use_portal",
    "mob_target",
    "mob_raid",
    "entity_custom_name_visible",
    "entity_classification",
    "entity_permanently_invulnerable",
    "entity_invulnerable_time",
    "entity_damage_cooldown_time",
    "entity_invulnerable_to_piercing_weapon",
    "entity_no_physics",
    "entity_spectator",
    "entity_is_shift_key_down",
    "entity_is_sprinting",
    "entity_is_in_water",
    "entity_is_in_lava",
    "entity_fire_immune",
    "entity_fire_ticks",
    "entity_air_supply",
    "entity_max_air_supply",
    "entity_is_silent",
    "entity_glowing",
    "entity_no_gravity",
    "entity_ticks_frozen",
    "entity_tick_count",
    "entity_is_invisible",
    "entity_is_multipart",
    "entity_parts",
    "entity_dimensions",
    "living_effect",
    "living_effects",
    "living_potion_affected",
    "living_gliding_ticks",
    "living_is_gliding",
    "living_is_sleeping",
    "living_sleeping_pos",
    "living_is_jumping",
    "living_baby",
    "living_equipment",
    "living_hand_item",
    "living_is_tamed",
    "living_owner",
    "living_dimensions_for_pose",
    "living_can_freeze",
    "living_can_breathe_underwater",
    "living_has_line_of_sight",
    "living_in_melee_range",
    "living_bed_orientation",
    "mob_goals",
    "mob_target_goals",
    "mob_is_done",
    "mob_navigation_speed",
    "mob_pathfinding_malus",
    "mob_is_aggressive",
    "mob_persistence_required",
    "mob_requires_custom_persistence",
    "mob_no_ai",
    "mob_restrict_center",
    "mob_restrict_radius",
    "mob_has_restriction",
    "mob_is_within_restriction",
    "mob_left_handed",
    "mob_can_pick_up_loot",
    "mob_no_action_time",
];

/// The handle types whose backend reads are entity data (#1077): a read taking
/// one first answers the tick's own view of that entity — the snapshot, the
/// player the tick sees — and is refused where that carries it not. Every
/// other backend read (a level, a chunk, a block entity, a registry key)
/// names data the tick leaves in place, so the view forwards it to the real
/// game backend instead of refusing it.
const GOAL_VIEW_ENTITY_ARGS: &[&str] = &[
    "EntityRef",
    "LivingRef",
    "MobRef",
    "PlayerRef",
    "ItemEntityRef",
    "MerchantRef",
];

/// The `Host` methods that find entities (#1077): the goal view answers these
/// by its own rules (the ticking mob, the player the tick sees, the snapshot,
/// else `None`) instead of forwarding them, so nothing asks the game's taken
/// mob list. Everything else the view forwards to the game host it carries.
pub(super) const GOAL_VIEW_LOOKUPS: &[&str] = &[
    "entity",
    "living",
    "mob",
    "player",
    "item_entity",
    "block_entity",
];

pub(super) const DIRECTORY: &str = "crates/horizon-mod-vocab/src/host";
/// The generated file `horizon-mod-api/src/host.rs` includes: the area modules and the prelude.
pub(super) const AREAS_OUTPUT: &str = "mod_api_host_areas.rs";

/// The traits an area file may hold, each a part of one of the unions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Tier {
    Host,
    ServerHost,
    ClientHost,
    Backend,
    ClientBackend,
    BackendMut,
    ClientBackendMut,
}

impl Tier {
    const ALL: [Tier; 7] = [
        Tier::Host,
        Tier::ServerHost,
        Tier::ClientHost,
        Tier::Backend,
        Tier::ClientBackend,
        Tier::BackendMut,
        Tier::ClientBackendMut,
    ];
    fn suffix(self) -> &'static str {
        match self {
            Tier::Host => "Host",
            Tier::ServerHost => "ServerHost",
            Tier::ClientHost => "ClientHost",
            Tier::Backend => "Backend",
            Tier::ClientBackend => "ClientBackend",
            Tier::BackendMut => "BackendMut",
            Tier::ClientBackendMut => "ClientBackendMut",
        }
    }
    /// The union trait this one is a part of.
    fn union(self) -> &'static str {
        match self {
            Tier::ClientBackend => "Backend",
            tier => tier.suffix(),
        }
    }
    fn mutable(self) -> bool {
        matches!(self, Tier::BackendMut | Tier::ClientBackendMut)
    }
    fn client(self) -> bool {
        matches!(
            self,
            Tier::ClientHost | Tier::ClientBackend | Tier::ClientBackendMut
        )
    }
    fn backend(self) -> bool {
        matches!(
            self,
            Tier::Backend | Tier::ClientBackend | Tier::BackendMut | Tier::ClientBackendMut
        )
    }
}

/// A method of an area trait, as the file declares it.
struct Entry {
    name: String,
    sig: syn::Signature,
    attrs: Vec<syn::Attribute>,
}

/// One area file: its traits by tier.
struct Area {
    stem: String,
    camel: String,
    tiers: BTreeMap<Tier, Vec<Entry>>,
}

impl Area {
    fn trait_name(&self, tier: Tier) -> String {
        format!("{}{}", self.camel, tier.suffix())
    }
    fn trait_path(&self, tier: Tier) -> String {
        format!("crate::host::{}::{}", self.stem, self.trait_name(tier))
    }
}

/// The traits of one area file by tier. Only `<Camel><Tier>` and the explicitly listed draw
/// contexts and services are accepted: an unknown trait would reach neither a union nor the WIT.
/// Imports and file/item attributes are allowed, but modules, macros and other items are refused
/// rather than hiding declarations from this scan.
fn area_tiers(camel: &str, file: syn::File) -> Result<BTreeMap<Tier, Vec<Entry>>, String> {
    let mut tiers = BTreeMap::new();
    for item in file.items {
        let item = match item {
            Item::Use(_) => continue,
            Item::Trait(item) => item,
            other => {
                let kind = match &other {
                    Item::Mod(_) => "module",
                    Item::Macro(_) => "macro",
                    _ => "item",
                };
                return Err(format!(
                    "unsupported {kind} in host area `{camel}`: only imports and declared traits are allowed, found `{}`",
                    other.to_token_stream()
                ));
            }
        };
        let name = item.ident.to_string();
        let Some(tier) = Tier::ALL
            .into_iter()
            .find(|tier| name == format!("{camel}{}", tier.suffix()))
        else {
            if matches!(
                (camel, name.as_str()),
                (
                    "Draw",
                    "DrawContext"
                        | "BlockEntityDrawContext"
                        | "WorldDrawContext"
                        | "CameraDrawContext"
                ) | (
                    "Services",
                    "ClientSession"
                        | "Commands"
                        | "Levels"
                        | "Messages"
                        | "Players"
                        | "RuntimeLevels"
                        | "RenderAssets"
                        | "InstanceGroups"
                )
            ) {
                continue;
            }
            return Err(format!(
                "unrecognised trait `{name}`: expected `{camel}<Host|ServerHost|ClientHost|Backend|ClientBackend|BackendMut|ClientBackendMut>` or an explicitly listed draw context or service"
            ));
        };
        let entries = item
            .items
            .into_iter()
            .filter_map(|member| match member {
                syn::TraitItem::Fn(method) => Some(Entry {
                    name: method.sig.ident.to_string(),
                    sig: method.sig,
                    attrs: method.attrs,
                }),
                _ => None,
            })
            .collect();
        tiers.insert(tier, entries);
    }
    Ok(tiers)
}

/// The area files, `base` first and then by name, as the catalog's are.
fn load(root: &Path) -> Result<Vec<Area>, String> {
    let dir = root.join(DIRECTORY);
    let mut areas = Vec::new();
    for stem in catalog::Catalog::modules(&dir)? {
        let path = dir.join(format!("{stem}.rs"));
        crate::rerun_if_changed(&path.display());
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        let camel = stem.to_upper_camel_case();
        let tiers = area_tiers(&camel, file).map_err(|e| format!("{}: {e}", path.display()))?;
        areas.push(Area { stem, camel, tiers });
    }
    crate::rerun_if_changed(&dir.display());
    Ok(areas)
}

/// The area modules, the prelude that names every trait of them, and one macro for each union of
/// them (`impl_host_areas!`, `impl_server_host_areas!`...) that implements the area traits of that
/// union with their defaults, except the areas a double overrides: what a test double or a startup
/// host writes instead of an `impl` for each area.
pub(super) fn areas_source(root: &Path) -> Result<String, String> {
    let areas = load(root)?;
    let dir = root.join(DIRECTORY);
    let mut modules = String::new();
    let mut prelude = String::new();
    for area in &areas {
        let path =
            fs::canonicalize(dir.join(format!("{}.rs", area.stem))).map_err(|e| e.to_string())?;
        modules.push_str(&format!("#[path = {path:?}] pub mod {};\n", area.stem));
        prelude.push_str(&format!("pub use super::{}::*;\n", area.stem));
    }
    // One helper per area: it expands its item unless the area is in the skip list.
    // Rustc rejects public re-exports of non-exported macro_rules macros. Export
    // hidden __mod_api_* implementations and re-export their caller-facing names
    // unqualified: absolute paths to include!-expanded macro_export definitions
    // otherwise trigger macro_expanded_macro_exports_accessed_by_absolute_paths.
    let mut macros = String::new();
    for area in &areas {
        macros.push_str(&format!(
            "#[doc(hidden)]\n#[macro_export]\nmacro_rules! __mod_api_skip_{stem} {{ ([] $body:item) => {{ $body }}; ([{stem} $($rest:ident)*] $body:item) => {{}}; ([$other:ident $($rest:ident)*] $body:item) => {{ $crate::__skip_{stem}!([$($rest)*] $body); }}; }}\n#[doc(hidden)]\npub use __mod_api_skip_{stem} as __skip_{stem};\n",
            stem = area.stem
        ));
    }
    for (name, tiers, union) in [
        ("impl_host_areas", &[Tier::Host][..], "Host"),
        (
            "impl_server_host_areas",
            &[Tier::ServerHost][..],
            "ServerHost",
        ),
        (
            "impl_client_host_areas",
            &[Tier::ClientHost][..],
            "ClientHost",
        ),
        (
            "impl_backend_areas",
            &[Tier::Backend, Tier::ClientBackend][..],
            "Backend",
        ),
        (
            "impl_backend_mut_areas",
            &[Tier::BackendMut][..],
            "BackendMut",
        ),
        (
            "impl_client_backend_mut_areas",
            &[Tier::ClientBackendMut][..],
            "ClientBackendMut",
        ),
    ] {
        let impls: String = areas
            .iter()
            .flat_map(|area| tiers.iter().map(move |tier| (area, *tier)))
            .filter(|(area, tier)| area.tiers.contains_key(tier))
            .map(|(area, tier)| {
                format!(
                    "$crate::__skip_{}!([$($($skip)*)?] impl $crate::prelude::{} for $host {{}});\n",
                    area.stem,
                    area.trait_name(tier)
                )
            })
            .collect();
        macros.push_str(&format!(
            "/// Implements the area traits of `{union}` for `$host` with their defaults, except the areas listed in `except(...)`, which the caller implements itself. The union trait is left to the caller.\n#[doc(hidden)]\n#[macro_export]\nmacro_rules! __mod_api_{name} {{ ($host:ty $(, except($($skip:ident),*))?) => {{ {impls} }}; }}\n#[doc(hidden)]\npub use __mod_api_{name} as {name};\n"
        ));
    }
    Ok(format!(
        "{modules}/// Every host trait of every area: bring it into scope to implement the game's side or a test double.\npub mod prelude {{\n{prelude}}}\n{macros}"
    ))
}

/// Generate every area impl explicitly. Full-path calls plus a hard recursion lint
/// refuse a missing inherent implementation instead of inheriting a trait default.
pub(super) fn game_forwarders(root: &Path) -> Result<String, String> {
    let mut code = String::from(
        "use crate::game::Game;\nuse crate::game::api_values::{call_stack_write, call_write, TryIntoApi, TryIntoEngine};\nuse mod_api::*;\n",
    );
    for area in load(root)? {
        for (tier, entries) in &area.tiers {
            code.push_str(&format!(
                "#[deny(unconditional_recursion)]\nimpl mod_api::prelude::{} for Game<'_> {{\n",
                area.trait_name(*tier)
            ));
            for entry in entries {
                let name = &entry.name;
                let attrs: Vec<_> = entry
                    .attrs
                    .iter()
                    .filter(|a| a.path().is_ident("access"))
                    .collect();
                if attrs.len() != 1 {
                    return Err(format!("{name}: expected exactly one access declaration"));
                }
                let level = attrs[0]
                    .parse_args::<syn::Ident>()
                    .map_err(|e| e.to_string())?;
                if !matches!(
                    level.to_string().as_str(),
                    "WorldOnly" | "Metadata" | "ClientSetup" | "PublishedResources"
                ) {
                    return Err(format!("{name}: unknown access level {level}"));
                }
                let (setup, args, checked_write) = boundary_arguments(&entry.sig, name)?;
                let static_call = entry.attrs.iter().any(|a| a.path().is_ident("static_call"));
                let args = if static_call {
                    args
                } else if args.is_empty() {
                    "self".into()
                } else {
                    format!("self, {args}")
                };
                let sig = entry.sig.to_token_stream();
                let call = if entry.attrs.iter().any(|a| a.path().is_ident("service")) {
                    "self".into()
                } else {
                    format!("Game::{name}({args})")
                };
                let crosses_output = match &entry.sig.output {
                    syn::ReturnType::Type(_, ty) => boundary_value(ty),
                    syn::ReturnType::Default => false,
                };
                let text_output = match &entry.sig.output {
                    syn::ReturnType::Type(_, ty) => text_value(ty),
                    syn::ReturnType::Default => false,
                };
                let text_result = result_error(&entry.sig.output)
                    .is_some_and(|error| outer_type(error, "TextFormError"));
                let call = if text_output {
                    if text_result {
                        format!("{call}.try_into_api().map_err(Into::into)")
                    } else {
                        format!("{call}.try_into_api().unwrap_or_else(|error| Err(error.into()))")
                    }
                } else if text_result {
                    // Writes with converted Text arguments call an infallible engine
                    // body. Read backends return Result so converting API-owned
                    // aggregate fields can refuse without losing a boundary error.
                    let text_input =
                        entry.sig.inputs.iter().any(
                            |arg| matches!(arg, syn::FnArg::Typed(arg) if text_value(&arg.ty)),
                        );
                    let read = entry
                        .sig
                        .receiver()
                        .is_some_and(|receiver| receiver.mutability.is_none());
                    if text_input && !read {
                        format!("Ok({call})")
                    } else {
                        call
                    }
                } else if checked_write {
                    if crosses_output {
                        format!("call_stack_write(|| {call})")
                    } else {
                        format!("call_write(|| {call})")
                    }
                } else if crosses_output {
                    match result_error(&entry.sig.output) {
                        Some(error) if outer_type(error, "LevelReadError") => format!(
                            "{call}.try_into_api().unwrap_or(Err(mod_api::LevelReadError::Unavailable))"
                        ),
                        Some(error) if outer_type(error, "GameRuleReadError") => format!(
                            "{call}.try_into_api().expect(\"game rule values convert infallibly\")"
                        ),
                        Some(_) => format!(
                            "{call}.try_into_api().unwrap_or_else(|error| Err(error.into_write_error().into()))"
                        ),
                        None => format!("{call}.try_into_api().unwrap_or_default()"),
                    }
                } else {
                    call
                };
                let typed_access_error = result_error(&entry.sig.output).filter(|error| {
                    outer_type(error, "LevelReadError") || outer_type(error, "LevelWriteError")
                });
                let access = if let Some(error) = typed_access_error {
                    let error = error.to_token_stream();
                    format!(
                        "if !mod_api::__private::access_allowed({name:?}, mod_api::__private::LoadAccess::{level}) {{ return Err({error}::Unavailable); }}"
                    )
                } else {
                    format!(
                        "mod_api::__private::check_access({name:?}, mod_api::__private::LoadAccess::{level});"
                    )
                };
                code.push_str(&format!("{sig} {{ {access} {setup} {call} }}\n"));
            }
            code.push_str("}\n");
            if matches!(tier, Tier::Host | Tier::Backend | Tier::ClientBackend) {
                let area_trait = area.trait_name(*tier);
                writeln!(code,"#[deny(unconditional_recursion)] impl mod_api::prelude::{area_trait} for crate::game::equip_host::EquipHost<'_, '_> {{").unwrap();
                for entry in entries {
                    let name = &entry.name;
                    let sig = entry.sig.to_token_stream();
                    if entry
                        .sig
                        .receiver()
                        .is_some_and(|receiver| receiver.mutability.is_some())
                    {
                        let unused = entry
                            .sig
                            .inputs
                            .iter()
                            .filter_map(|arg| match arg {
                                FnArg::Typed(arg) => {
                                    Some(format!("let _ = {};", arg.pat.to_token_stream()))
                                }
                                _ => None,
                            })
                            .collect::<String>();
                        writeln!(code,"{sig} {{ {unused} panic!(\"mutable access through an observation-only equipment Host\") }}").unwrap();
                        continue;
                    }
                    let args = signature_args(&entry.sig, name)?;
                    let tail = if args.is_empty() {
                        String::new()
                    } else {
                        format!(",{args}")
                    };
                    let call = format!(
                        "<Game<'_> as mod_api::prelude::{area_trait}>::{name}(self.game{tail})"
                    );
                    let cached = match name.as_str() {
                        "entity_kind" => Some(("entity", "self.row.kind")),
                        "entity_position" => Some(("entity", "self.row.position")),
                        "living" => Some((
                            "entity",
                            "self.row.living.then_some(LivingRef::new(entity))",
                        )),
                        "living_health" => Some(("living.entity()", "self.row.health")),
                        "living_max_health" => Some(("living.entity()", "self.row.max_health")),
                        "living_alive" => Some(("living.entity()", "self.row.alive")),
                        "living_equipment" => Some(("living.entity()", "self.equipment(slot)")),
                        "living_hand_item" => Some((
                            "living.entity()",
                            "self.equipment(match hand { Hand::Main => EquipmentSlot::MainHand, Hand::Off => EquipmentSlot::OffHand })",
                        )),
                        _ => None,
                    };
                    let cached=cached.map(|(entity,value)|format!("if {entity}==self.row.entity {{ mod_api::__private::check_access({name:?},mod_api::__private::LoadAccess::WorldOnly); return {value}; }}")).unwrap_or_default();
                    writeln!(code, "{sig} {{ {cached} {call} }}").unwrap();
                }
                code.push_str("}\n");
            }
        }
    }
    let parsed = syn::parse_file(&code).map_err(|e| e.to_string())?;
    let imports = standard_imports(&parsed);
    let parsed = syn::parse_file(&(imports + &code)).map_err(|e| e.to_string())?;
    Ok(prettyplease::unparse(&parsed))
}

/// API records whose game representation is owned by another crate. Conversions are
/// implemented once by the game; this rule only selects the boundary, never its behaviour.
fn text_value(ty: &Type) -> bool {
    contains_value(ty, &["Text", "GuiDraw"])
}

fn boundary_value(ty: &Type) -> bool {
    contains_value(
        ty,
        &[
            "ItemStack",
            "BlockState",
            "GameRuleValue",
            "ItemResource",
            "ItemCost",
            "MerchantOffer",
            "Ingredient",
            "Recipe",
            "RecipeRule",
            "RecipeTemplate",
            "RecipeTrimPattern",
            "FireworkShapeIngredient",
            "AnvilResult",
            "AnvilCraft",
            "AnvilUpdate",
            "GrindstonePlace",
            "GrindstoneTake",
            "ParticleOptions",
        ],
    )
}

fn contains_value(ty: &Type, names: &[&str]) -> bool {
    struct Values<'a>(bool, &'a [&'a str]);
    impl<'ast> Visit<'ast> for Values<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            self.0 |= path
                .path
                .segments
                .last()
                .is_some_and(|segment| self.1.contains(&segment.ident.to_string().as_str()));
            syn::visit::visit_type_path(self, path);
        }
    }
    let mut values = Values(false, names);
    values.visit_type(ty);
    values.0
}

fn result_error(output: &syn::ReturnType) -> Option<&Type> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    let Type::Path(path) = &**ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(error) = args.args.last()? else {
        return None;
    };
    Some(error)
}

/// A borrowed `&ItemStack`: outside a checked write, the method only reads it.
fn read_only_stack(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if outer_type(&reference.elem, "ItemStack"))
}

fn outer_type(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

/// The conversion code for a host method's boundary arguments, the arguments to pass on, and
/// whether it is a checked write (`&mut self`, a `Result`, a converted value). A checked write
/// refuses an invalid value with its error; a read converts a borrowed stack without the write
/// limits, and any other refused value makes the method return its default.
fn boundary_arguments(
    sig: &syn::Signature,
    method: &str,
) -> Result<(String, String, bool), String> {
    let mutable = sig
        .receiver()
        .is_some_and(|receiver| receiver.mutability.is_some());
    let typed = sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(arg) => Some(arg),
            _ => None,
        })
        .collect::<Vec<_>>();
    let checked_write = mutable
        && result_error(&sig.output).is_some()
        && typed
            .iter()
            .any(|arg| boundary_value(&arg.ty) && !contains_value(&arg.ty, &["ParticleOptions"]));
    let mut setup = String::new();
    let mut args = Vec::new();
    for arg in typed {
        let Pat::Ident(pat) = &*arg.pat else {
            return Err(format!("{method}: unsupported argument"));
        };
        let name = pat.ident.to_string();
        let text = text_value(&arg.ty);
        if !boundary_value(&arg.ty) && !text {
            args.push(name);
            continue;
        }
        let conversion = if text {
            format!("{name}.try_into_engine(&())")
        } else if !checked_write && read_only_stack(&arg.ty) {
            // Only a write imposes resident maxima: a stack vanilla stores (damage above its
            // maximum, say) still answers a read.
            format!("crate::game::api_values::stack_value({name})")
        } else {
            format!("{name}.try_into_engine(self.items())")
        };
        let value = if contains_value(&arg.ty, &["ParticleOptions"]) {
            format!(
                "match {name}.try_into_engine(self.items()) {{ Ok(value) => value, Err(_) => return Default::default() }}"
            )
        } else if text || checked_write {
            format!("{conversion}?")
        } else if result_error(&sig.output).is_some() {
            format!(
                "match {conversion} {{ Ok(value) => value, Err(_) => return Err(mod_api::NoBackend::no_backend()) }}"
            )
        } else {
            format!(
                "match {conversion} {{ Ok(value) => value, Err(_) => return Default::default() }}"
            )
        };
        setup.push_str(&format!("let {name} = {value};\n"));
        args.push(match &*arg.ty {
            Type::Reference(_) => format!("&{name}"),
            Type::Path(path) if outer_type(&arg.ty, "Option") => {
                let segment = path.path.segments.last().unwrap();
                let borrowed = matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args)
                    if args.args.iter().any(|arg| matches!(arg, syn::GenericArgument::Type(Type::Reference(_)))));
                if borrowed { format!("{name}.as_ref()") } else { name }
            }
            _ => name,
        });
    }
    Ok((setup, args.join(", "), checked_write))
}

fn standard_imports(file: &syn::File) -> String {
    struct StdTypes {
        path_buf: bool,
        arc: bool,
    }
    impl<'ast> Visit<'ast> for StdTypes {
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if ty.path.segments.len() == 1 {
                self.path_buf |= ty.path.segments[0].ident == "PathBuf";
                self.arc |= ty.path.segments[0].ident == "Arc";
            }
            syn::visit::visit_type_path(self, ty);
        }
    }
    let mut std_types = StdTypes {
        path_buf: false,
        arc: false,
    };
    std_types.visit_file(file);
    let mut imports = String::new();
    if std_types.path_buf {
        imports.push_str("use std::path::PathBuf;\n");
    }
    if std_types.arc {
        imports.push_str("use std::sync::Arc;\n");
    }
    imports
}

fn signature_args(sig: &syn::Signature, name: &str) -> Result<String, String> {
    Ok(sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            FnArg::Typed(arg) => match &*arg.pat {
                Pat::Ident(pat) => Ok(pat.ident.to_string()),
                _ => Err(format!("{name}: unsupported argument")),
            },
            FnArg::Receiver(_) => Err(format!("{name}: unexpected receiver")),
        })
        .collect::<Result<Vec<_>, String>>()?
        .join(", "))
}

/// How a read answers where there is no backend to ask: its typed refusal, or `None` where only
/// a `String` forwards.
fn refusal(sig: &syn::Signature, name: &str) -> Result<Option<&'static str>, String> {
    let syn::ReturnType::Type(_, ty) = &sig.output else {
        return Err(format!("{name}: a backend read returns a value"));
    };
    let syn::Type::Path(path) = &**ty else {
        return Err(format!(
            "{name}: the goal view cannot refuse its return type"
        ));
    };
    let last = path.path.segments.last().map(|seg| seg.ident.to_string());
    Ok(match last.as_deref() {
        Some("Option") => Some("None"),
        Some("bool") => Some("false"),
        // A read that answers `Result`: its refusal is the error a host without a backend answers.
        Some("Result") => Some("Err(mod_api::NoBackend::no_backend())"),
        Some("u64" | "u32" | "i32" | "u16" | "u8" | "usize") => Some("0"),
        // A registry key's metadata: a read the goal view only forwards.
        Some("String") => None,
        _ => {
            return Err(format!(
                "{name}: the goal view cannot refuse its return type"
            ));
        }
    })
}

/// `entity_position(entity)` on a backend is also `EntityRef::position(self, host)`: the handle
/// a method is of, when its first argument is one and its name starts with the handle's.
fn handle_method(
    entry: &Entry,
    tier: Tier,
    handles: &BTreeMap<String, String>,
) -> Result<Option<(String, String)>, String> {
    let name = entry.name.as_str();
    let mut inputs = entry.sig.inputs.iter();
    let mutable = matches!(inputs.next(), Some(FnArg::Receiver(r)) if r.mutability.is_some());
    let Some(FnArg::Typed(first)) = inputs.next() else {
        return Ok(None);
    };
    let Type::Path(first) = &*first.ty else {
        return Ok(None);
    };
    let Some(handle) = first.path.get_ident().map(|ident| ident.to_string()) else {
        return Ok(None);
    };
    let Some((receiver, _)) = handles.iter().find(|(_, ty)| **ty == handle) else {
        return Ok(None);
    };
    let Some(short) = name
        .strip_prefix(receiver.as_str())
        .and_then(|rest| rest.strip_prefix('_'))
    else {
        return Ok(None);
    };
    if mutable != tier.mutable() {
        return Err(format!("{name}: its receiver and its trait disagree"));
    }
    let mut sig = entry.sig.clone();
    sig.ident =
        syn::parse_str(short).map_err(|_| format!("{name}: {short} is not a method name"))?;
    let host: FnArg = match (mutable, tier.client()) {
        (true, true) => syn::parse_quote!(host: &mut dyn ClientHost),
        (true, false) => syn::parse_quote!(host: &mut dyn ServerHost),
        (false, true) => syn::parse_quote!(host: &dyn ClientHost),
        (false, false) => syn::parse_quote!(host: &dyn Host),
    };
    let mut args = String::new();
    let mut rest = vec![syn::parse_quote!(self), host];
    for arg in entry.sig.inputs.iter().skip(2) {
        let mut arg = arg.clone();
        let FnArg::Typed(typed) = &mut arg else {
            return Err(format!("{name}: unexpected receiver"));
        };
        let Pat::Ident(pat) = &mut *typed.pat else {
            return Err(format!("{name}: unsupported argument"));
        };
        pat.mutability = None;
        args.push_str(&format!(", {}", pat.ident));
        rest.push(arg);
    }
    sig.inputs = rest.into_iter().collect();
    let mut docs = doc_comments(&entry.attrs, name, true)?;
    // The reads a mob's goal tick answers from its view carry the snapshot rule where a mod reads
    // them (#1077).
    if GOAL_VIEW_READS.contains(&name) {
        docs.push_str("///\n/// In a mob's goal tick ([`GoalContext::host`](crate::GoalContext::host)) this reads\n/// the active mob or a disjoint live mob when available; player reads use the\n/// game host. Other unsupported reads retain their typed refusal.\n");
    }
    let backend = match (mutable, tier.client()) {
        (true, true) => "client_backend_mut_of",
        (true, false) => "backend_mut_of",
        _ => "backend_of",
    };
    let returns = |ident: &str| {
        matches!(&sig.output, syn::ReturnType::Type(_, ty) if matches!(&**ty, syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|seg| seg.ident == ident)))
    };
    let call = format!("{name}(self{args})");
    let body = if returns("Option") {
        format!("{backend}(host)?.{call}")
    } else if handle.ends_with("Key") && returns("String") {
        format!("{backend}(host).expect(\"registry key metadata requires a game host\").{call}")
    } else if returns("bool") {
        format!("{backend}(host).map(|backend| backend.{call}).unwrap_or(false)")
    } else if returns("CommandResult") {
        // An unresolved command source has neither a successful run nor feedback.
        format!("{backend}(host).map(|backend| backend.{call}).unwrap_or_default()")
    } else if returns("Result") {
        format!(
            "{backend}(host).map_or_else(|| Err(crate::NoBackend::no_backend()), |backend| backend.{call})"
        )
    } else {
        return Err(format!(
            "{name}: a {receiver} helper returns Option, bool, CommandResult or Result, the value a host without a backend refuses"
        ));
    };
    Ok(Some((
        handle,
        format!("{docs}pub {} {{ {body} }}\n", sig.to_token_stream()),
    )))
}

/// The unions, the handles' methods and the hidden backends' side of the host, and the goal view.
pub(super) fn generate(root: &Path) -> Result<(String, String), String> {
    let areas = load(root)?;
    let api_source = fs::read_to_string(root.join("crates/horizon-mod-vocab/src/lib.rs"))
        .map_err(|e| e.to_string())?;
    let api_source = super::exports::expand(root, &api_source)?;
    let mut types = api_types(&api_source)?;
    // Parse domain files separately: their inner docs and attributes belong to
    // that file, not to the middle of a concatenated API root.
    for entry in
        fs::read_dir(root.join("crates/horizon-mod-vocab/src")).map_err(|e| e.to_string())?
    {
        let path = entry.map_err(|e| e.to_string())?.path().join("types.rs");
        if path.is_file() {
            let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let domain = api_types(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            for (receiver, name) in domain.handles {
                if types.handles.insert(receiver.clone(), name).is_some() {
                    return Err(format!("duplicate handle receiver {receiver}"));
                }
            }
        }
    }
    let supertraits = |union: &str| -> String {
        areas
            .iter()
            .flat_map(|area| {
                Tier::ALL
                    .into_iter()
                    .filter(|tier| tier.union() == union && area.tiers.contains_key(tier))
                    .map(|tier| area.trait_path(tier))
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let mut reads: Vec<(&Area, Tier, &Entry)> = Vec::new();
    let mut host_methods: Vec<(&Area, &Entry)> = Vec::new();
    let mut methods = String::new();
    let mut by_handle = BTreeMap::<String, String>::new();
    for area in &areas {
        for (tier, entries) in &area.tiers {
            for entry in entries {
                if tier.backend()
                    && let Some((handle, code)) = handle_method(entry, *tier, &types.handles)?
                {
                    by_handle.entry(handle).or_default().push_str(&code);
                }
                if matches!(tier, Tier::Backend | Tier::ClientBackend) {
                    reads.push((area, *tier, entry));
                } else if *tier == Tier::Host && area.stem != "services" {
                    host_methods.push((area, entry));
                }
            }
        }
    }
    for (handle, code) in &by_handle {
        methods.push_str(&format!("impl {handle} {{\n{code}}}\n"));
    }
    let imports = standard_imports(&syn::parse_file(&methods).map_err(|e| e.to_string())?);
    let side_enum = |name: &str, server: &str, client: &str| {
        format!(
            "/// The logical side of a hook's host.\n// no wire: borrows the active host; passed as call context.\npub enum {name}<'a> {{\n/// The hook fired on the server.\nServer({server}),\n/// The hook fired on the client.\nClient({client}) }}\n"
        )
    };
    let sides = format!(
        "{}{}",
        side_enum(
            "HostSide",
            "&'a mut dyn ServerHost",
            "&'a mut dyn ClientHost"
        ),
        side_enum("LevelSide", "&'a dyn LevelHost", "&'a dyn LevelHost")
    );
    let (host_supers, server_supers, client_supers) = (
        supertraits("Host"),
        supertraits("ServerHost"),
        supertraits("ClientHost"),
    );
    let (backend_supers, backend_mut_supers, client_backend_mut_supers) = (
        supertraits("Backend"),
        supertraits("BackendMut"),
        supertraits("ClientBackendMut"),
    );
    let host = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\n{imports}/// Services supplied by the game to trait mods.\n/// Each method's default panics with its name, so a test double implements only what its test calls; the game implements every one.\n/// The backend accessors are the exception: they default to None, so a double without a backend answers every handle's typed refusal.\n/// Its methods are declared by area in `host/<area>.rs`; `use mod_api::prelude::*` names every area trait.\npub trait Host: {host_supers} {{\n#[doc(hidden)] fn backend(&self) -> Option<BackendRef<'_>> {{ None }}\n#[doc(hidden)] fn capability_providers(&self) -> &'static dyn CapabilityProviders {{ &InstalledProviders }}\n}}\n/// What only the server's hooks are given: writes to the world.\npub trait ServerHost: Host + {server_supers} {{\n#[doc(hidden)] fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {{ None }}\n}}\n/// What only the client's hooks are given: the client's own services.\npub trait ClientHost: Host + {client_supers} {{\n#[doc(hidden)] fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {{ None }}\n}}\n{sides}#[doc(hidden)] pub trait Backend: {backend_supers} {{}}\n#[doc(hidden)] pub trait BackendMut: {backend_mut_supers} {{}}\n#[doc(hidden)] pub trait ClientBackendMut: {client_backend_mut_supers} {{}}\n{methods}"
    );
    syn::parse_file(&host).map_err(|e| format!("generated Host: {e}"))?;
    let goal_view = goal_view(&areas, &reads, &host_methods)?;
    Ok((host, goal_view))
}

/// The goal view's generated impls (#1077): the backend ones forward the snapshot-backed
/// reads to the engine context's `r_*` methods, refuse their typed refusal for entity data the
/// tick's view does not carry, and forward every other backend read — the level's own: blocks,
/// loaded chunks, time, height, weather, block entities — to the real backend of the game host
/// the view carries; the host ones answer the entity lookups by the tick's own rules, refuse the
/// mutable methods, and forward everything else to the game host the view carries. The engine
/// includes them beside its `GoalCx`.
fn goal_view(
    areas: &[Area],
    reads: &[(&Area, Tier, &Entry)],
    host_methods: &[(&Area, &Entry)],
) -> Result<String, String> {
    let mut impls = Impls::default();
    let mut code = String::from(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse mod_api::{prelude::*, *};\nuse std::path::PathBuf;\nuse std::sync::Arc;\n/// The mob tick's read view (#1077), as the game generates it: the snapshot-backed\n/// reads forward to the engine context's own methods, entity data the view does\n/// not carry answers its typed refusal, and every other backend read forwards\n/// to the real backend of the game host the view carries.\nimpl Backend for super::GoalCx<'_, '_> {}\n",
    );
    for (area, tier, entry) in reads {
        let (name, args) = (
            entry.name.as_str(),
            signature_args(&entry.sig, &entry.name)?,
        );
        let refusal = refusal(&entry.sig, name)?;
        let mut sig = entry.sig.clone();
        sig.ident = syn::parse_str(name).map_err(|e| format!("{name}: {e}"))?;
        // Entity data answers the tick's view; everything else forwards to the real backend the
        // view carries.
        let entity_data = matches!(
            entry.sig.inputs.iter().skip(1).find_map(|arg| match arg {
                FnArg::Typed(arg) => Some(&*arg.ty),
                FnArg::Receiver(_) => None,
            }),
            Some(syn::Type::Path(ty))
                if ty.path.segments.len() == 1
                    && GOAL_VIEW_ENTITY_ARGS
                        .contains(&ty.path.segments[0].ident.to_string().as_str())
        );
        // How a forwarded read answers a host without a backend: the typed refusal, or the empty
        // string of a metadata read.
        let forwarded = match refusal {
            Some("None") => ".flatten()".to_string(),
            // A `Result` read's no-backend answer is its error type's own `NoBackend` value,
            // wrapped back in the `Err`.
            Some(refusal) if refusal.starts_with("Err(") => {
                ".unwrap_or_else(|| Err(mod_api::NoBackend::no_backend()))".to_string()
            }
            Some(refusal) => format!(".unwrap_or({refusal})"),
            None => ".unwrap_or_default()".to_string(),
        };
        let body = if GOAL_VIEW_READS.contains(&name) {
            format!("self.r_{name}({args})")
        } else if entity_data {
            let Some(refusal) = refusal else {
                return Err(format!(
                    "{name}: the goal view cannot refuse its return type"
                ));
            };
            if args.is_empty() {
                refusal.to_string()
            } else if args.contains(',') {
                format!("let _ = ({args}); {refusal}")
            } else {
                format!("let _ = {args}; {refusal}")
            }
        } else {
            // The read names data the tick leaves in place, so the real backend answers it:
            // `backend_read` holds the `None` a host without one answers.
            format!(
                "mod_api::__private::backend_read(self.host, |backend| backend.{name}({args})){forwarded}"
            )
        };
        // Entities that remain in Game can use its ordinary backend. For a
        // multi-entity query all handle arguments must be present there: the
        // Game backend cannot see a mob lent to the current AI tick.
        let game_entities: Vec<_> = entry
            .sig
            .inputs
            .iter()
            .skip(1)
            .filter_map(|arg| {
                let FnArg::Typed(arg) = arg else { return None };
                let Pat::Ident(pat) = &*arg.pat else {
                    return None;
                };
                let Type::Path(ty) = &*arg.ty else {
                    return None;
                };
                let ty = ty.path.get_ident()?.to_string();
                let name = pat.ident.to_string();
                match ty.as_str() {
                    "EntityRef" => Some(name),
                    "LivingRef" | "PlayerRef" | "MobRef" | "ItemEntityRef" | "MerchantRef" => {
                        Some(format!("{name}.entity()"))
                    }
                    _ => None,
                }
            })
            .collect();
        let body = if entity_data && !game_entities.is_empty() {
            let all_available = game_entities
                .iter()
                .map(|entity| format!("self.is_game_entity({entity})"))
                .collect::<Vec<_>>()
                .join(" && ");
            format!(
                "if {all_available} {{ return mod_api::__private::backend_read(self.host, |backend| backend.{name}({args})){forwarded}; }} {body}"
            )
        } else {
            body
        };
        impls.add(
            area.trait_name(*tier),
            format!("{} {{ {body} }}\n", sig.to_token_stream()),
        );
    }
    code.push_str(&impls.take());
    code.push_str(
        "/// The same view as a host: the entity lookups answer the tick's own rules\n/// (the ticking mob, the player it sees, the snapshot, else None), the mutable\n/// methods stay out of reach of the shared host the view holds, and everything\n/// else - the level, its blocks, tags, recipes and settings - forwards to the\n/// game host the view carries.\nimpl Host for super::GoalCx<'_, '_> {\n#[doc(hidden)]\nfn backend(&self) -> Option<BackendRef<'_>> { Some(mod_api::__private::backend_ref(self)) }\n}\n",
    );
    // An area whose host methods all write has nothing to forward, but the view still implements it.
    for area in areas
        .iter()
        .filter(|area| area.tiers.contains_key(&Tier::Host))
    {
        impls.add(area.trait_name(Tier::Host), String::new());
    }
    for (area, entry) in host_methods {
        let name = entry.name.as_str();
        let mut sig = entry.sig.clone();
        sig.ident = syn::parse_str(name).map_err(|e| format!("{name}: {e}"))?;
        // A `&mut self` Host method cannot be called on the `&dyn Host` the context hands out,
        // so the view leaves it to the trait's panicking default: a goal cannot reach it (that
        // is the point).
        if matches!(sig.inputs.first(), Some(FnArg::Receiver(r)) if r.mutability.is_some()) {
            continue;
        }
        let args = signature_args(&sig, name)?;
        let body = if GOAL_VIEW_LOOKUPS.contains(&name) {
            format!("self.lookup_{name}({args})")
        } else {
            format!("self.host.{name}({args})")
        };
        impls.add(
            area.trait_name(Tier::Host),
            format!("{} {{ {body} }}\n", sig.to_token_stream()),
        );
    }
    code.push_str(&impls.take());
    Ok(code)
}

/// The methods of each trait the goal view implements, in the order they were added.
#[derive(Default)]
struct Impls(Vec<(String, String)>);

impl Impls {
    fn add(&mut self, name: String, method: String) {
        match self.0.iter_mut().find(|(other, _)| *other == name) {
            Some((_, methods)) => methods.push_str(&method),
            None => self.0.push((name, method)),
        }
    }
    fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
            .into_iter()
            .map(|(name, methods)| {
                format!("impl {name} for super::GoalCx<'_, '_> {{\n{methods}}}\n")
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "host/state_write_tests.rs"]
mod state_write_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str) -> Entry {
        let method: syn::TraitItemFn = syn::parse_str(source).unwrap();
        Entry {
            name: method.sig.ident.to_string(),
            sig: method.sig,
            attrs: method.attrs,
        }
    }

    fn handles(source: &str) -> BTreeMap<String, String> {
        api_types(source).unwrap().handles
    }

    #[test]
    fn immutable_result_arguments_refuse_conversion_with_a_typed_error() {
        let method = entry(
            "fn read(&self, ingredient: &Ingredient) -> Result<Arc<[ItemKey]>, IngredientKeysError>;",
        );
        let (setup, _, checked_write) = boundary_arguments(&method.sig, "read").unwrap();
        assert!(!checked_write);
        assert!(setup.contains("return Err(mod_api::NoBackend::no_backend())"));
        assert!(!setup.contains("Default::default()"));
    }

    #[test]
    fn borrowed_stacks_are_read_without_write_validation_outside_checked_writes() {
        let read = entry("fn item_resource_damageable(&self, stack: &ItemStack) -> bool;");
        let (setup, args, checked_write) = boundary_arguments(&read.sig, "read").unwrap();
        assert!(!checked_write);
        assert!(setup.contains("crate::game::api_values::stack_value(stack)"));
        assert!(!setup.contains("try_into_engine"));
        assert_eq!(args, "&stack");
        let write = entry(
            "fn living_apply_consume_effect(&mut self, stack: &ItemStack) -> Result<bool, StackWriteError>;",
        );
        let (setup, _, checked_write) = boundary_arguments(&write.sig, "write").unwrap();
        assert!(checked_write);
        assert!(setup.contains("stack.try_into_engine(self.items())?"));
        let owned = entry("fn drop_at(&mut self, stack: ItemStack) -> bool;");
        let (setup, _, _) = boundary_arguments(&owned.sig, "owned").unwrap();
        assert!(setup.contains("stack.try_into_engine(self.items())"));
    }

    #[test]
    fn a_method_is_a_handles_when_it_takes_one_first_and_is_named_for_it() {
        let handles = handles("pub struct EntityRef; pub struct LevelRef; pub struct BlockKey;");
        let read = entry(
            "/// Where it is.\nfn entity_position(&self, entity: EntityRef, scale: f64) -> Option<[f64; 3]>;",
        );
        let (handle, code) = handle_method(&read, Tier::Backend, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "EntityRef");
        assert!(code.starts_with("/// Where it is.\n"));
        assert!(code.contains("pub fn position (self , host : & dyn Host , scale : f64)"));
        assert!(code.contains("backend_of(host)?.entity_position(self, scale)"));
        // A client read borrows the client host, a write the mutable one of its side.
        let (_, client) = handle_method(&read, Tier::ClientBackend, &handles)
            .unwrap()
            .unwrap();
        assert!(client.contains("host : & dyn ClientHost"));
        let write = entry("fn level_set(&mut self, level: LevelRef) -> bool;");
        let (_, code) = handle_method(&write, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains("host : & mut dyn ServerHost"));
        assert!(code.contains(
            "backend_mut_of(host).map(|backend| backend.level_set(self)).unwrap_or(false)"
        ));
        let (_, code) = handle_method(&write, Tier::ClientBackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains("host : & mut dyn ClientHost"));
        assert!(code.contains("client_backend_mut_of(host)"));
        // A command result refuses with an unsuccessful run and no feedback.
        let command = entry(
            "fn entity_run_command_with_output(&mut self, entity: EntityRef, line: &str, level: PermissionLevel) -> CommandResult;",
        );
        let (handle, code) = handle_method(&command, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "EntityRef");
        assert!(code.contains(
            "backend_mut_of(host).map(|backend| backend.entity_run_command_with_output(self, line, level)).unwrap_or_default()"
        ));
        // A `Result` method refuses with its error's `NoBackend` value on a host without a game.
        let tried = entry("fn level_attempt(&mut self, level: LevelRef) -> Result<u8, Refused>;");
        let (_, code) = handle_method(&tried, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains(
            "backend_mut_of(host).map_or_else(|| Err(crate::NoBackend::no_backend()), |backend| backend.level_attempt(self))"
        ));
        // A key's name is read with an owned answer.
        let key = entry("fn block_key_id(&self, key: BlockKey) -> String;");
        let (handle, code) = handle_method(&key, Tier::Backend, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "BlockKey");
        assert!(code.contains("backend_of(host).expect("));
    }

    #[test]
    fn other_methods_are_the_backends_alone_and_a_wrong_shape_is_refused() {
        let handles = handles("pub struct EntityRef; pub struct LevelRef;");
        // Not named for its handle, or not taking one first: a backend method only.
        for source in [
            "fn mob_x(&self, entity: EntityRef) -> Option<()>;",
            "fn entity_x(&self, level: LevelRef) -> Option<()>;",
            "fn entity_x(&self, count: u32) -> Option<()>;",
            "fn entity_x(&self) -> Option<()>;",
        ] {
            assert!(
                handle_method(&entry(source), Tier::Backend, &handles)
                    .unwrap()
                    .is_none(),
                "{source}"
            );
        }
        for (source, tier, error) in [
            (
                "fn entity_type(&self, entity: EntityRef) -> Option<()>;",
                Tier::Backend,
                "not a method name",
            ),
            (
                "fn entity_x(&self, entity: EntityRef);",
                Tier::Backend,
                "returns Option, bool, CommandResult or Result",
            ),
            (
                "fn entity_x(&mut self, entity: EntityRef) -> bool;",
                Tier::Backend,
                "disagree",
            ),
        ] {
            assert!(
                handle_method(&entry(source), tier, &handles)
                    .unwrap_err()
                    .contains(error),
                "{source}"
            );
        }
    }

    #[test]
    fn public_ref_structs_define_receivers_without_registration() {
        let types = api_types(
            "pub struct BlockEntityRef; pub struct HTTPRequestRef;
             struct HiddenRef; pub(crate) struct InternalRef;
             pub enum EnumRef {} pub type AliasRef = BlockEntityRef; pub struct Plain;",
        )
        .unwrap();
        for (receiver, ty) in [
            ("block_entity", "BlockEntityRef"),
            ("http_request", "HTTPRequestRef"),
        ] {
            let name = format!("{receiver}_kind");
            let method = entry(&format!("fn {name}(&self, value: {ty}) -> Option<()>;"));
            let (handle, code) = handle_method(&method, Tier::Backend, &types.handles)
                .unwrap()
                .unwrap();
            assert_eq!(handle, ty);
            assert!(code.contains(&format!("backend_of(host)?.{name}(self)")));
        }
        let hidden = entry("fn hidden_kind(&self, value: HiddenRef) -> Option<()>;");
        assert!(
            handle_method(&hidden, Tier::Backend, &types.handles)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            api_types("pub struct URLRef; pub struct UrlRef;")
                .err()
                .unwrap(),
            "duplicate handle receiver url",
        );
    }

    #[test]
    fn text_arguments_refuse_before_even_checked_stack_writes() {
        let text = entry("fn set_text(&mut self, text: &Text) -> Result<(), TextFormError>;");
        let (setup, args, checked) = boundary_arguments(&text.sig, "set_text").unwrap();
        assert!(!checked);
        assert!(setup.contains("text.try_into_engine(&())?"));
        assert_eq!(args, "&text");
        let mixed = entry(
            "fn update(&mut self, stack: ItemStack, text: Option<&Text>) -> Result<Text, Refused>;",
        );
        let (setup, args, checked) = boundary_arguments(&mixed.sig, "update").unwrap();
        assert!(checked);
        assert!(setup.contains("stack.try_into_engine(self.items())?"));
        assert!(setup.contains("text.try_into_engine(&())?"));
        assert_eq!(args, "stack, text.as_ref()");
        let syn::ReturnType::Type(_, output) = &mixed.sig.output else {
            panic!()
        };
        assert!(text_value(output));
        assert!(!boundary_value(output));
    }

    #[test]
    fn game_provides_every_area_method_explicitly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        use std::collections::BTreeSet;
        let expected: BTreeSet<_> = load(&root)
            .unwrap()
            .into_iter()
            .flat_map(|area| {
                area.tiers.into_iter().flat_map(move |(tier, entries)| {
                    let owner = format!("{}{}", area.camel, tier.suffix());
                    entries
                        .into_iter()
                        .map(move |entry| (owner.clone(), entry.name))
                })
            })
            .collect();
        let source = game_forwarders(&root).unwrap();
        let file = syn::parse_file(&source).unwrap();
        let mut actual = BTreeSet::new();
        for item in file.items {
            if let Item::Impl(item) = item {
                assert!(item.attrs.iter().any(|attr| {
                    attr.path().is_ident("deny")
                        && attr
                            .parse_args::<syn::Ident>()
                            .is_ok_and(|lint| lint == "unconditional_recursion")
                }));
                // The generator also emits a read-only EquipHost implementation.
                // This assertion covers Game's complete backend surface.
                let syn::Type::Path(receiver) = &*item.self_ty else {
                    panic!("expected named host")
                };
                if receiver.path.segments.last().unwrap().ident != "Game" {
                    continue;
                }
                let owner = item
                    .trait_
                    .unwrap()
                    .1
                    .segments
                    .last()
                    .unwrap()
                    .ident
                    .to_string();
                for member in item.items {
                    let syn::ImplItem::Fn(method) = member else {
                        panic!("non-method in Game impl")
                    };
                    let name = method.sig.ident.to_string();
                    assert!(actual.insert((owner.clone(), name)));
                }
            }
        }
        assert_eq!(actual, expected, "Game must not inherit an area default");
        // Other area traits must not be in scope. The current impl's own trait is
        // implicitly in scope; deny(unconditional_recursion) rejects that fallback.
        assert!(!source.contains("prelude::*"));
    }

    #[test]
    fn an_area_file_reads_its_tier_traits() {
        let file = syn::parse_str(
            "pub trait ItemsHost { fn a(&self); } pub trait ItemsBackendMut { fn b(&mut self); }",
        )
        .unwrap();
        let tiers = area_tiers("Items", file).unwrap();
        assert_eq!(
            tiers.keys().copied().collect::<Vec<_>>(),
            [Tier::Host, Tier::BackendMut]
        );
        assert_eq!(tiers[&Tier::Host][0].name, "a");
    }

    #[test]
    fn an_unrecognised_area_trait_is_refused() {
        for wrong in [
            "ItemHost",
            "ItemsReadBackend",
            "OtherServerHost",
            "ReviewProbeHosts",
            "Unrelated",
        ] {
            for recognised in ["", "pub trait ItemsHost { fn known(&self); }"] {
                let file = syn::parse_str(&format!(
                    "{recognised} pub trait {wrong} {{ fn a(&self) -> &std::path::Path; }}"
                ))
                .unwrap();
                let error = area_tiers("Items", file).err().unwrap();
                assert!(error.contains(wrong), "{error}");
            }
        }
    }

    #[test]
    fn declarations_cannot_hide_in_modules_or_macros() {
        for (kind, source) in [
            (
                "module",
                "pub mod inner { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }",
            ),
            (
                "macro",
                "macro_rules! hidden { () => { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }; } hidden!();",
            ),
            (
                "macro",
                "hidden! { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }",
            ),
        ] {
            let file = syn::parse_file(source).unwrap();
            let error = area_tiers("Items", file).err().unwrap();
            assert!(error.contains(&format!("unsupported {kind}")), "{error}");
            assert!(error.contains("Items"), "{error}");
        }
    }

    #[test]
    fn imports_and_attributes_are_allowed() {
        let file = syn::parse_file(
            "#![allow(unused_variables)] use crate::*; /// An area's methods.\n #[doc(hidden)] pub trait ItemsBackend {}",
        )
        .unwrap();
        let tiers = area_tiers("Items", file).unwrap();
        assert!(tiers.contains_key(&Tier::Backend));
    }

    #[test]
    fn non_host_traits_are_allowed_only_in_their_own_file() {
        for (area, name) in [("Draw", "DrawContext"), ("Services", "Commands")] {
            let file = syn::parse_file(&format!("pub trait {name} {{}}")).unwrap();
            assert!(area_tiers(area, file.clone()).unwrap().is_empty());
            assert!(area_tiers("Items", file).is_err());
        }
    }

    #[test]
    fn every_area_file_of_the_checkout_loads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let areas = load(&root).unwrap();
        assert!(areas.iter().any(|area| area.stem == "config"));
        assert!(
            areas
                .iter()
                .filter(|area| area.tiers.is_empty())
                .all(|area| matches!(area.stem.as_str(), "draw" | "services"))
        );
    }
}
