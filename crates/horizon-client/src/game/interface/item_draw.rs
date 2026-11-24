//! Items drawn through the resolved item model pipeline
//! (`horizon_client::item_render`): GUI icons (`GuiGraphics.renderItem`),
//! dropped items (`ItemEntityRenderer`) and items posed in the world (hands,
//! frames, armour stands). This binds the resolver's host services
//! (`ItemEnv`) and the special renderers' registries to the game.
use super::api_values::TryIntoApi;
use super::{Dim, Game, entity_culling::EntityCullDecision};
use horizon_assets::item_model::{
    Components, ComponentsView, DisplayContext, FluidAppearance, GlobalPos, ItemEnv, LevelState,
    OwnerState, StackView,
};
use horizon_client::{
    block_entity_render::{diffuse_lights, vertex_light},
    cit::{Hand as CitHand, Kind as CitKind},
    graphics::EntityCullingMode,
    hud::{HudBatch, Rect},
    item_render::{self, ItemRenderState, SpecialHost, WorldSink},
    items::ItemStack,
    mesher::Vertex,
};
use horizon_core::geometry::{Aabb, Vec3};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap};

struct ItemEntityDraw<'a> {
    item: &'a horizon_client::entities::ItemEntity,
    at: [f64; 3],
    partial: f32,
    entity: bool,
}

/// The game's painter for the HUD kit's icons (`hud_kit::icon`): items by their models, the
/// player's face from `face`, their skin's texture.
pub(super) struct IconPainter<'g, 'a> {
    pub(super) game: &'g Game<'a>,
    pub(super) face: Option<&'g str>,
}

impl horizon_client::hud_kit::icon::IconPainter for IconPainter<'_, '_> {
    fn item(&self, batch: &mut HudBatch, id: &str, [x, y, size]: [f32; 3]) -> bool {
        let start = batch.vertices.len();
        self.game
            .draw_item_model(batch, &ItemStack::new(id, 1), x, y);
        let scale = size / horizon_client::hud_kit::icon::ITEM_SIZE;
        batch.scale_since(start, x, y, scale, scale);
        batch.vertices.len() > start
    }
    fn face(&self, batch: &mut HudBatch, [x, y, size]: [f32; 3]) -> bool {
        let Some(skin) = self.face else {
            return false;
        };
        let mut draws = Vec::new();
        // `PlayerFaceExtractor`: the face, then its hat layer.
        horizon_client::tab_list::face(
            &mut draws,
            skin,
            x as i32,
            y as i32,
            size as i32,
            true,
            false,
            u32::MAX,
        );
        horizon_client::hud_layout::paint(batch, &self.game.gui.atlas, &draws);
        true
    }
}

/// The game's answers to what the resolver cannot read from the stack.
pub(super) struct Env<'a, 'g> {
    game: &'a Game<'g>,
}
impl Game<'_> {
    /// Before a frame is drawn: the model draws prepared for the last one are dropped, and so are
    /// the special model renderers' answers it didn't draw.
    pub(super) fn begin_frame_draws(&self) {
        self.prepared_model_draws.borrow_mut().clear();
        super::native_special_renderers::begin_frame(std::ptr::from_ref(self) as usize);
    }
}

impl ItemEnv for Env<'_, '_> {
    fn item_model_property(&self, property: &str, stack: &StackView) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ItemModelProperty) {
            return false;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return false;
        };
        let mut value = false;
        self.game.native_item_model_property(
            property,
            &snapshot,
            stack.bundle_selected,
            &mut value,
        );
        value
    }
    fn item_model_range_property(&self, property: &str, stack: &StackView) -> f32 {
        if !mod_api::__private::listened(mod_api::HookId::ItemModelRangeProperty) {
            return 0.0;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return 0.0;
        };
        let mut value = 0.0;
        self.game.native_item_model_range_property(
            property,
            &snapshot,
            stack.bundle_selected,
            &mut value,
        );
        value
    }
    fn item_model_select_property(&self, property: &str, stack: &StackView) -> Option<String> {
        if !mod_api::__private::listened(mod_api::HookId::ItemModelSelectProperty) {
            return None;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return None;
        };
        let mut value = None;
        self.game.native_item_model_select_property(
            property,
            &snapshot,
            stack.bundle_selected,
            &mut value,
        );
        value
    }
    fn item_tint_source(
        &self,
        source: &str,
        config: &horizon_core::nbt::Tag,
        stack: &StackView,
    ) -> i32 {
        if !mod_api::__private::listened(mod_api::HookId::ItemTintSource) {
            return -1;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return -1;
        };
        let mut color = u32::MAX;
        self.game.native_item_tint_source(
            source,
            config,
            &snapshot,
            stack.bundle_selected,
            &mut color,
        );
        color as i32
    }
    fn item_model_update(
        &self,
        kind: &str,
        config: &horizon_core::nbt::Tag,
        stack: &StackView,
        ctx: &horizon_assets::item_model::Context,
        models: &mut Vec<String>,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ItemModelUpdate) {
            return;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return;
        };
        let Ok(display) = ctx.display.try_into_api();
        let owner = ctx.owner.and_then(|owner| owner.try_into_api().ok());
        let level = ctx.level.and_then(|level| level.try_into_api().ok());
        self.game
            .native_item_model_update(mod_api::__private::ItemModelUpdateCall {
                kind,
                config,
                stack: &snapshot,
                bundle_selected: stack.bundle_selected,
                display,
                owner: owner.as_ref(),
                level: level.as_ref(),
                seed: ctx.seed,
                models,
            });
    }
    fn special_model_parts(
        &self,
        slot: horizon_assets::item_model::SpecialModelSlot,
        base: &str,
        config: &std::sync::Arc<horizon_core::nbt::Tag>,
        stack: &StackView,
    ) -> Option<horizon_assets::item_model::PreparedParts> {
        super::native_special_renderers::parts(self.game, slot, config, stack, |parts| {
            self.game.pack.item_renderer.prepare_parts(base, parts)
        })
    }
    fn stack_fluid(&self, stack: &StackView) -> Option<&str> {
        // Vanilla's items keep vanilla's closed set; only a mod's own item asks its parts.
        if stack.item.starts_with("minecraft:") {
            return horizon_client::fluid_models::held_fluid(&stack.item);
        }
        if !mod_api::__private::item_listens(mod_api::HookId::ContainedFluid, &stack.item) {
            return None;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return None;
        };
        let mut fluid = None;
        self.game.native_contained_fluid(&snapshot, &mut fluid);
        drawn_contained_fluid(&stack.item, fluid?)
    }
    fn fluid_appearance(&self, fluid: &str) -> Option<FluidAppearance> {
        horizon_client::fluid_models::item_appearance(fluid)
    }
    fn enchantment_in_tag(&self, enchantment: &str, tag: &str) -> bool {
        self.game.stations().enchantments.in_tag(tag, enchantment)
    }
    fn potion_effect_color(&self, contents: &Value) -> Option<i32> {
        // `PotionContents.getColorOr`: the effect mix when any effect is visible.
        let id = contents["potion"].as_str();
        let effects = id
            .and_then(horizon_client::brewing::potion)
            .is_some_and(|p| !p.effects.is_empty());
        effects.then(|| (0xFF00_0000 | horizon_client::brewing::potion_color(id)) as i32)
    }
    fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        self.game
            .pack
            .item_renderer
            .grass_color(temperature, downfall)
    }
    fn bundle_fullness(&self, _stack: &StackView, components: &Components) -> f32 {
        // `BundleContents.weight`: count / max stack size, summed.
        let Some(items) = components
            .get("minecraft:bundle_contents")
            .and_then(Value::as_array)
        else {
            return 0.0;
        };
        let prototypes = &self.game.pack.item_renderer.models.prototypes;
        let mut total = 0.0f64;
        for item in items {
            let id = item["id"].as_str().unwrap_or("");
            let count = item["count"].as_i64().unwrap_or(1) as f64;
            let max = prototypes
                .get(id)
                .and_then(|c| c.get("minecraft:max_stack_size"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as f64;
            total += count / max;
        }
        total as f32
    }
    fn has_shift_down(&self) -> bool {
        self.game.has_shift_down()
    }
    fn local_time(&self, pattern: &str, _locale: &str, _time_zone: Option<&str>) -> String {
        local_time(pattern)
    }
    fn random_float(&self) -> f32 {
        let seed = match &self.game.export {
            Some(export) => export.elapsed().as_nanos() as u64,
            // The same draw for a frame of the same world tick.
            None if self.game.clock.still() => self.game.visual_ticks(),
            None => {
                let nanos = self.game.started.elapsed().subsec_nanos();
                return (nanos % 1_000_000) as f32 / 1_000_000.0;
            }
        };
        let mixed = (seed ^ 0x9e37_79b9).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        (mixed >> 40) as f32 / (1u64 << 24) as f32
    }
}
/// The fluid a mod's `contained_fluid` answer `key` draws in a stack of `item`: none (the stack
/// draws as holding none) for a key no fluid is registered under, logged the first time an item
/// answers so. A registered fluid with no look in an item is drawn, as the missing sprite.
fn drawn_contained_fluid(item: &str, key: mod_api::FluidKey) -> Option<&'static str> {
    let fluid = mod_api::__private::fluid_id(key);
    if fluid.is_none() && first_unregistered_answer(item) {
        log::warn!(
            "contained_fluid for {item} answered a fluid key no fluid is registered under: drawn as holding none"
        );
    }
    fluid
}
/// Whether `item` answers an unregistered fluid key for the first time: only that path reads the
/// list, so a stack that draws pays nothing for it.
fn first_unregistered_answer(item: &str) -> bool {
    static REPORTED: std::sync::Mutex<Vec<Box<str>>> = std::sync::Mutex::new(Vec::new());
    let mut reported = REPORTED.lock().unwrap_or_else(|poison| poison.into_inner());
    if reported.iter().any(|seen| &**seen == item) {
        return false;
    }
    reported.push(item.into());
    true
}
/// The days since 1970-01-01 as a civil (year, month, day) date.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
/// `DateTimeFormatter.ofPattern(pattern).format(now)` for the month, day,
/// year and time letters item models use, in the system time zone
/// (`LocalTime` without a `time_zone`).
pub(super) fn local_time(pattern: &str) -> String {
    let now = chrono::Local::now();
    let secs = now.timestamp() + i64::from(now.offset().local_minus_utc());
    let (y, mo, d) = civil(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    let (h, mi, se) = (s / 3600, s / 60 % 60, s % 60);
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let num = |v: i64| format!("{v:0n$}");
        match c {
            'M' => out += &num(i64::from(mo)),
            'd' => out += &num(i64::from(d)),
            'y' | 'u' => out += &if n == 2 { num(y % 100) } else { num(y) },
            'H' => out += &num(h),
            'm' => out += &num(mi),
            's' => out += &num(se),
            '\'' => {
                // Quoted literal text.
                let end = chars[i + 1..].iter().position(|&x| x == '\'');
                if let Some(e) = end {
                    out.extend(&chars[i + 1..i + 1 + e]);
                    i += e + 2;
                    continue;
                }
                n = 1;
            }
            other => out.extend(std::iter::repeat_n(other, n)),
        }
        i += n;
    }
    out
}

/// Banner and shield pattern textures from the banner pattern registry.
pub(super) struct Host<'a> {
    banners: &'a horizon_client::banner::Banners,
    sprites: &'a HashMap<String, Rect>,
    skins: &'a RefCell<horizon_client::skins::Downloader>,
    profiles: &'a RefCell<horizon_client::skins::Resolver>,
}
impl SpecialHost for Host<'_> {
    fn banner_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.banner_texture(pattern))
    }
    fn shield_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.shield_texture(pattern))
    }
    fn player_skin(&self, profile: &Value) -> Option<String> {
        // SkullBlockRenderer.getPlayerSkin: SkinManager.getInsecureSkinInformation
        // accepts a texture property without a signature, unlike the tab list.
        // A `Dynamic` profile (a name or id only) resolves through the
        // services (`PlayerSkinRenderCache.getOrDefault`).
        Some(horizon_client::skins::head_skin(
            profile,
            &mut self.profiles.borrow_mut(),
            &mut self.skins.borrow_mut(),
            |location| self.sprites.contains_key(location),
        ))
    }
}

/// `ItemOwner` of an entity holding or showing an item.
pub(super) fn owner_at(position: [f64; 3], yaw: f32, entity_type: &str) -> OwnerState {
    OwnerState {
        position,
        visual_rotation_y_degrees: yaw,
        entity_type: entity_type.to_owned(),
        ..Default::default()
    }
}
/// `UseDuration` of a stack (`Item.getUseDuration`): 72000 for drawn items
/// and for `blocks_attacks` items (shields), the consume time for food.
fn use_duration(components: ComponentsView<'_>) -> i32 {
    if let Some(c) = components.get("minecraft:consumable") {
        let seconds = c["consume_seconds"].as_f64().unwrap_or(1.6);
        return (seconds * 20.0) as i32;
    }
    if components.get("minecraft:blocks_attacks").is_some()
        || components.get("minecraft:kinetic_weapon").is_some()
    {
        return 72_000;
    }
    let item_model = components
        .get("minecraft:item_model")
        .and_then(Value::as_str)
        .unwrap_or("");
    match item_model {
        "minecraft:bow"
        | "minecraft:crossbow"
        | "minecraft:trident"
        | "minecraft:spyglass"
        | "minecraft:goat_horn" => 72_000,
        // BrushItem.USE_DURATION.
        "minecraft:brush" => 200,
        _ => 0,
    }
}

impl Game<'_> {
    pub(super) fn item_env(&self) -> Env<'_, '_> {
        Env { game: self }
    }
    pub(super) fn item_host(&self) -> Host<'_> {
        Host {
            banners: &self.stations().banners,
            sprites: &self.gui.atlas.sprites,
            skins: &self.skin_downloads,
            profiles: &self.skin_profiles,
        }
    }
    /// Borrow the cached [`LevelState`] for this frame without cloning its `String` fields.
    fn with_item_level<R>(&self, f: impl FnOnce(&LevelState) -> R) -> R {
        let dimension = self.dim.id();
        let key = (
            self.frames,
            self.world.game_time,
            self.world.day_time,
            dimension,
        );
        if let Ok(cached) = self.item_level_cache.try_borrow()
            && let Some((at, level)) = cached.as_ref()
            && *at == key
        {
            return f(level);
        }
        let level = self.level_state_now(dimension);
        match self.item_level_cache.try_borrow_mut() {
            Ok(mut cached) => {
                *cached = Some((key, level));
                drop(cached);
                let cached = self.item_level_cache.borrow();
                let (_, level) = cached.as_ref().expect("item_level_cache populated");
                f(level)
            }
            Err(_) => f(&level),
        }
    }
    /// HDR `(tier, light)` pair for `item` when `hdr_on` reflects whether HDR output is active.
    pub(super) fn item_hdr(&self, item: &str, hdr_on: bool) -> (u8, u8) {
        let lum = self.luminance();
        let level = lum.of(item);
        if level == 0 {
            return (0, 0);
        }
        let tier = lum.hdr_tier(item);
        let hdr_light = if tier > 0 && hdr_on { level } else { 0 };
        (tier, hdr_light)
    }
    fn level_state_now(&self, dimension: &'static str) -> LevelState {
        let sky = self
            .dim_environment()
            .sample(&horizon_client::environment::Sample {
                day_time: self.world.day_time,
                ..Default::default()
            });
        LevelState {
            dimension: dimension.to_owned(),
            game_time: self.world.game_time,
            sun_angle: sky.sun_angle,
            moon_phase: (self.world.day_time.div_euclid(24_000)).rem_euclid(8) as i32,
            respawn: self.spawn.map(|s| GlobalPos {
                dimension: "minecraft:overworld".to_owned(),
                pos: [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32],
            }),
            runs_normally: true,
        }
    }
    /// The local player as `ItemOwner`; `stack` is the stack drawn, to tell
    /// whether it is the one in use.
    pub(super) fn player_owner(&self, stack: Option<&ItemStack>) -> OwnerState {
        let position = self
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut owner = owner_at(position, self.yaw, "minecraft:player");
        owner.main_arm_left = self.settings.main_hand_left;
        owner.is_local_player = true;
        owner.is_view_entity = true;
        owner.last_death_location = self
            .level
            .last_death
            .clone()
            .map(|(dimension, pos)| GlobalPos { dimension, pos });
        if let (Some((slot, ticks)), Some(stack)) = (self.using, stack)
            && self.stack_in(slot) == Some(stack)
        {
            let view = item_render::stack_view(stack);
            let mut duration = use_duration(self.pack.item_renderer.models.components_view(&view));
            self.native_use_duration(stack, &mut duration);
            owner.using_this_item = true;
            owner.use_duration = duration;
            // getUseItemRemainingTicks: the duration the use started with,
            // as LivingEntityUseItemEvent.Start and .Tick left it (`game/items/item_use.rs`).
            owner.use_item_remaining_ticks = self.use_duration - ticks as i32;
            owner.crossbow_charge_duration = Self::charge_ticks(stack).max(1) as i32;
        }
        owner.fishing_cast_with_this_item = stack
            .zip(self.cast_rod())
            .is_some_and(|(s, rod)| std::ptr::eq(s, rod));
        owner.is_selected =
            stack.is_some() && self.inventory.get(self.selected).and_then(Option::as_ref) == stack;
        owner.is_carried = owner.is_selected;
        owner
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub(super) fn resolve_item(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
    ) -> ItemRenderState {
        self.resolve_item_in_hand(stack, display, owner, seed, CitHand::Main)
    }
    /// `ItemStack.hasFoil`, with the item's default glint.
    fn has_foil(&self, stack: &ItemStack) -> bool {
        let item_glint = self
            .res
            .items()
            .enchantments
            .props
            .get(&*stack.item)
            .and_then(|p| p.glint);
        stack.has_foil(item_glint)
    }
    /// [`Self::resolve_item`] of a stack held in `hand`, which OptiFine CIT rules can test
    /// (GUI items count as the main hand).
    pub(super) fn resolve_item_in_hand(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
        hand: CitHand,
    ) -> ItemRenderState {
        let renderer = &self.pack.item_renderer;
        if let Some(cached) = renderer.resolve_static_stack(stack, display) {
            let mut state = cached.state.clone();
            let hdr_on = self.renderer.as_ref().is_some_and(|r| r.hdr_on());
            let (hdr_tier, hdr_light) = self.item_hdr(&stack.item, hdr_on);
            state.hdr_tier = hdr_tier;
            state.hdr_light = hdr_light;
            return state;
        }
        self.resolve_item_uncached_into(stack, display, owner, seed, hand, Vec::new())
    }
    /// Full uncached item resolution (never touches `static_cache`, safe to call while holding a
    /// [`item_render::StaticDisplayCache`] guard), reusing the allocation of `layers`.
    pub(super) fn resolve_item_uncached_into(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
        hand: CitHand,
        layers: Vec<item_render::LayerRender>,
    ) -> ItemRenderState {
        let renderer = &self.pack.item_renderer;
        let (rule, glint) = renderer.cit.as_ref().map_or((None, None), |cit| {
            let item_max = self
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            let max_damage = item_render::cit::stack_max_damage(stack, item_max);
            (
                cit.rules.select(CitKind::Item, stack, hand, max_damage),
                cit.rules
                    .glint(stack, hand, max_damage, self.has_foil(stack)),
            )
        });
        let stack_view = item_render::stack_view(stack);
        let env = self.item_env();
        let mut state = self.with_item_level(|level| {
            renderer.resolve_with_rule_into(
                &stack_view,
                display,
                owner,
                Some(level),
                seed,
                &env,
                rule,
                layers,
            )
        });
        if let Some(glint) = glint {
            // The pack's enchantment rules replace vanilla's glint unless one keeps it, so far
            // only in the GUI, where their own layers draw: held, dropped and framed items keep
            // vanilla's until theirs do. The pack's `useGlint=false` removes it everywhere.
            let gui = display == DisplayContext::Gui;
            let pack_keeps = renderer
                .cit
                .as_ref()
                .is_none_or(|c| c.rules.global.use_glint);
            if !glint.vanilla && (gui || !pack_keeps) {
                for layer in &mut state.layers {
                    layer.foil = horizon_assets::item_model::FoilType::None;
                }
            }
            if gui && !glint.is_empty() {
                state.cit_glint = Some(glint);
            }
        }
        let hdr_on = self.renderer.as_ref().is_some_and(|r| r.hdr_on());
        let (hdr_tier, hdr_light) = self.item_hdr(&stack.item, hdr_on);
        state.hdr_tier = hdr_tier;
        state.hdr_light = hdr_light;
        state
    }
    /// `GuiGraphicsExtractor.item(player, level, stack, x, y, 0)`.
    pub(super) fn draw_item_model(&self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return;
        }
        let renderer = &self.pack.item_renderer;
        if let Some(cached) = renderer.resolve_static_stack(stack, DisplayContext::Gui) {
            item_render::gui::draw(
                &cached.state,
                renderer,
                &self.gui.atlas.sprites,
                &self.item_host(),
                batch,
                x,
                y,
                self.glint_frame.millis / 1000.0,
                self.glint_frame.speed,
                self.glint_frame.alpha,
            );
            return;
        }
        let owner = self.player_owner(Some(stack));
        let state = self.resolve_item_uncached_into(
            stack,
            DisplayContext::Gui,
            Some(&owner),
            0,
            CitHand::Main,
            Vec::new(),
        );
        item_render::gui::draw(
            &state,
            renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            batch,
            x,
            y,
            self.glint_frame.millis / 1000.0,
            self.glint_frame.speed,
            self.glint_frame.alpha,
        );
    }
    /// The glint clock, options and sprite for this frame: read once per
    /// frame, not per item (`GameRenderer.extractOptions`).
    pub(super) fn capture_glint_frame(&self) -> item_render::GlintFrame {
        item_render::GlintFrame {
            millis: if let Some(export) = &self.export {
                export.elapsed().as_secs_f32() * 1000.0
            } else if self.clock.still() {
                self.visual_millis() as f32
            } else {
                self.started.elapsed().as_secs_f32() * 1000.0
            },
            speed: self.settings.file.double("glintSpeed") as f32,
            alpha: self.settings.file.double("glintStrength") as f32,
            sprite: self
                .gui
                .atlas
                .sprites
                .contains_key("misc/enchanted_glint_item"),
        }
    }
    /// The `ENTITY_GLINT_TEXTURING` inputs of this frame.
    pub(super) fn glint(&self) -> Option<item_render::Glint> {
        self.glint_frame.glint(0.5)
    }
    /// Submit a resolved item under `pose` (camera-relative) at `light`
    /// (packed lightmap coordinates).
    pub(super) fn submit_item_world(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        self.submit_item_world_hdr(state, (state.hdr_tier, state.hdr_light), pose, light, out);
    }
    pub(super) fn submit_item_world_hdr(
        &self,
        state: &ItemRenderState,
        hdr: (u8, u8),
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(self.nether_lighting());
        self.submit_item_lit_hdr(state, hdr, pose, light, lights, out);
    }
    /// [`Self::submit_item_world`] shaded by other diffuse lights (a
    /// `Lighting.Entry` other than the level's), in the pose's output space.
    pub(super) fn submit_item_lit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        out: &mut Vec<Vertex>,
    ) {
        self.submit_item_lit_hdr(
            state,
            (state.hdr_tier, state.hdr_light),
            pose,
            light,
            lights,
            out,
        );
    }
    pub(super) fn submit_item_lit_hdr(
        &self,
        state: &ItemRenderState,
        (hdr_tier, hdr_light): (u8, u8),
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light: item_hdr_light_level(hdr_light, light),
            lights: (hdr_light == 0).then_some(lights),
            glint: self.glint(),
            tier: hdr_tier,
        };
        state.submit(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_world`] without the level's diffuse shading.
    pub(super) fn submit_item_unlit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light: item_hdr_light(state, light),
            lights: None,
            glint: self.glint(),
            tier: state.hdr_tier,
        };
        state.submit(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_lit`] of `state` under each of `poses` on the GPU path: the mark
    /// before its draws and their bounds, or `None` (nothing drawn) when the state cannot
    /// take it.
    pub(super) fn submit_item_skinned(
        &self,
        state: &ItemRenderState,
        poses: &[glam::Mat4],
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
    ) -> Option<((usize, usize, u32), Option<(glam::Vec3, glam::Vec3)>)> {
        self.submit_item_skinned_hdr(
            state,
            (state.hdr_tier, state.hdr_light),
            poses,
            light,
            lights,
        )
    }
    pub(super) fn submit_item_skinned_hdr(
        &self,
        state: &ItemRenderState,
        (hdr_tier, hdr_light): (u8, u8),
        poses: &[glam::Mat4],
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
    ) -> Option<((usize, usize, u32), Option<(glam::Vec3, glam::Vec3)>)> {
        let mut models = self.skin_models.borrow_mut();
        let mut frame = self.skin_frame.borrow_mut();
        let mark = frame.mark();
        let mut skin = horizon_client::mobs::skin::Skin::new(&mut models, &mut frame);
        let sink = (
            item_hdr_light_level(hdr_light, light),
            (hdr_light == 0).then_some(lights),
            hdr_tier,
        );
        if !state.submit_skinned_poses(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            poses,
            sink,
            &mut skin,
        ) {
            return None;
        }
        Some((mark, skin.bounds))
    }
    /// One pose of [`Self::submit_item_skinned`] into an entity's open
    /// `skin` (its bounds grow by the item's): false, with nothing drawn,
    /// when the item cannot take the GPU path.
    pub(super) fn skin_item(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        skin: &mut horizon_client::mobs::skin::Skin,
    ) -> bool {
        let sink = (
            item_hdr_light(state, light),
            (state.hdr_light == 0).then_some(lights),
            state.hdr_tier,
        );
        let renderer = &self.pack.item_renderer;
        state.submit_skinned(renderer, &self.gui.atlas.sprites, pose, sink, skin)
    }
    /// [`Self::submit_item_world`] of `state` under `pose` on the GPU path, its draws counted
    /// with an entity's (`skinned`: its mark and bounds, `cull_skinned`'s); false, with
    /// nothing drawn, when the state cannot take it.
    pub(super) fn skin_item_into(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        skinned: &mut super::creatures::Skinned,
    ) -> bool {
        let lights = diffuse_lights(self.nether_lighting());
        let Some((mark, bounds)) = self.submit_item_skinned(state, &[pose], light, lights) else {
            return false;
        };
        *skinned = Some(match skinned.take() {
            Some((first, before)) => (
                first,
                match (before, bounds) {
                    (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
                    (one, other) => one.or(other),
                },
            ),
            None => (mark, bounds),
        });
        true
    }
    /// `ItemEntityRenderer.submit` for every dropped item.
    pub(super) fn dropped_item_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        if self.dropped.is_empty() {
            return;
        }
        let live = self.dropped.iter().map(|item| {
            let (p, q) = (item.previous, item.position);
            let at = [
                p.x + (q.x - p.x) * alpha,
                p.y + (q.y - p.y) * alpha,
                p.z + (q.z - p.z) * alpha,
            ];
            ItemEntityDraw {
                item,
                at,
                partial: alpha as f32,
                entity: true,
            }
        });
        self.item_entity_geometry(camera, live, out);
    }

    pub(super) fn pickup_item_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        if self.pickups.live.is_empty() {
            return;
        }
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Item(item) => Some(ItemEntityDraw {
                item,
                at: super::pickups::position(p, alpha),
                partial: 1.0,
                entity: false,
            }),
            _ => None,
        });
        self.item_entity_geometry(camera, taken, out);
    }

    fn item_entity_geometry<'a>(
        &self,
        camera: glam::DVec3,
        items: impl Iterator<Item = ItemEntityDraw<'a>>,
        out: &mut Vec<Vertex>,
    ) {
        let mut items = items.peekable();
        if items.peek().is_none() {
            return;
        }
        // Entity.shouldRenderAtSqrDistance for the item's 0.25 box: 16 blocks
        // times the view scale (32 at render distance 16), from its current
        // position. A pickup particle is not an entity and keeps 64 blocks.
        let range =
            super::entity_culling::default_range(glam::DVec3::splat(0.25), self.view_scale());
        let skinning = self.skinning();
        let lights = diffuse_lights(self.dim == Dim::Nether);
        let hdr_on = self.renderer.as_ref().is_some_and(|r| r.hdr_on());
        let mut static_cache = self
            .pack
            .item_renderer
            .static_cache_for(DisplayContext::Ground);
        let mut scratch_layers: Vec<item_render::LayerRender> = Vec::new();
        for ItemEntityDraw {
            item,
            at,
            partial,
            entity,
        } in items
        {
            let entity_start = out.len();
            self.mark_motion(entity_start, (2 << 32) | u64::from(item.id));
            let q = item.position;
            let pos = glam::DVec3::from_array(at) - camera;
            let far = if entity {
                (glam::DVec3::new(q.x, q.y, q.z) - camera).length_squared() >= range * range
            } else {
                pos.length_squared() > 64.0 * 64.0
            };
            if far {
                continue;
            }
            // EntityRenderer.shouldRender: the item's 0.25 box, grown by 0.5,
            // in the view frustum. Checked before its model is resolved.
            if entity && let Some(view) = &self.view_frustum {
                let (lo, hi) = dropped_item_cull_box(glam::DVec3::from_array(at), camera);
                if !view.intersects(lo, hi) {
                    continue;
                }
            }
            // Fast path: plain stack with a statically classified item model — avoids `owner_at`,
            // `LevelState`, `StackView`, `Vec<LayerRender>`, per-item locks, and `bounding_box()`.
            let plain =
                item.stack.count > 0 && item.stack.damage == 0 && item.stack.data().is_none();
            let mut dynamic_state: Option<ItemRenderState> = None;
            let (state, (lo, hi), hdr) = if plain
                && let Some(cached) = static_cache
                    .as_mut()
                    .and_then(|cache| cache.get(&item.stack.item))
            {
                if cached.state.is_empty() {
                    continue;
                }
                (
                    &cached.state,
                    cached.bounding_box,
                    self.item_hdr(&item.stack.item, hdr_on),
                )
            } else {
                // `extractItemGroupRenderState`: GROUND, seeded by the entity id.
                let owner = owner_at([q.x, q.y, q.z], 0.0, "minecraft:item");
                let resolved = self.resolve_item_uncached_into(
                    &item.stack,
                    DisplayContext::Ground,
                    Some(&owner),
                    item.id as i32,
                    CitHand::Main,
                    std::mem::take(&mut scratch_layers),
                );
                if resolved.is_empty() {
                    scratch_layers = resolved.layers;
                    continue;
                }
                let bbox = resolved.bounding_box();
                let hdr = (resolved.hdr_tier, resolved.hdr_light);
                let state_ref = dynamic_state.insert(resolved);
                (&*state_ref, bbox, hdr)
            };
            let age = item.age as f32 + partial;
            let min_offset_y = -lo.y + 0.0625;
            let bob = (age / 10.0 + item.bob_offset).sin() * 0.1 + 0.1;
            let spin = item_render::item_spin(age, item.bob_offset);
            let pose = glam::Mat4::from_translation(pos.as_vec3())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, bob + min_offset_y, 0.0))
                * glam::Mat4::from_rotation_y(spin);
            // `EntityRenderer.getPackedLightCoords`: an item entity is lit from
            // the block its light probe (its eye, 0.2125 up) is in, at 15 block
            // light while it burns. A pickup particle is not an entity and keeps
            // the cell it is drawn in.
            let packed = if entity {
                // The shared extraction already interpolated the drawn position in f64.
                let probe = [
                    at[0],
                    at[1] + horizon_client::entities::ItemEntity::EYE_HEIGHT,
                    at[2],
                ];
                item.render_light(self.world.light(
                    probe[0].floor() as i32,
                    probe[1].floor() as i32,
                    probe[2].floor() as i32,
                ))
            } else {
                vertex_light(self.world.light(
                    q.x.floor() as i32,
                    (q.y + 0.1).floor() as i32,
                    q.z.floor() as i32,
                ))
            };
            let seed = i64::from(self.pack.item_renderer.item_id(&item.stack.item))
                + i64::from(item.stack.damage);
            let (copy_buf, copy_len) = item_render::cluster_poses_stack(
                pose,
                item_render::rendered_amount(item.stack.count),
                seed,
                hi.z - lo.z,
            );
            let copies = &copy_buf[..copy_len];
            // Use the same animated, spun, clustered transforms as the draw.
            // ItemEntity's collision box is only 0.25 blocks and cannot prove
            // that this larger rendered model stays behind an occluder.
            let culling = entity.then(|| {
                // The posed envelope walks every model point in every copy;
                // with culling off the decision is `Bypass` whatever it holds.
                if self.entity_culling_mode() == EntityCullingMode::Off {
                    return EntityCullDecision::Bypass;
                }
                state
                    .posed_bounds(copies)
                    .map(|(lo, hi)| {
                        self.entity_cull_decision_in_bounds(
                            item,
                            Aabb::new(
                                Vec3::new(
                                    camera.x + f64::from(lo.x),
                                    camera.y + f64::from(lo.y),
                                    camera.z + f64::from(lo.z),
                                ),
                                Vec3::new(
                                    camera.x + f64::from(hi.x),
                                    camera.y + f64::from(hi.y),
                                    camera.z + f64::from(hi.z),
                                ),
                            ),
                        )
                    })
                    .unwrap_or_else(|| self.entity_cull_decision(item, f64::from(partial)))
            });
            if culling.is_some_and(|decision| decision.hides_entity()) {
                if let Some(owned) = dynamic_state {
                    scratch_layers = owned.layers;
                }
                continue;
            }
            let outline = self.thing_outline(item.id, "minecraft:item");
            // Posed on the GPU when every layer can be, its silhouette too when it glows.
            let skinned = skinning
                .then(|| self.submit_item_skinned_hdr(state, hdr, copies, packed, lights))
                .flatten();
            if skinned.is_none() {
                for &copy in copies {
                    self.submit_item_world_hdr(state, hdr, copy, packed, out);
                }
            }
            if let Some(rgb) = outline {
                self.mark_outline(&out[entity_start..], rgb);
                if let Some(((first, _, _), _)) = skinned {
                    let color = horizon_client::mobs::render::abgr(rgb);
                    self.skin_frame.borrow_mut().outline_since(first, color);
                }
            } else if let Some(culling) = culling {
                self.finish_entity_occlusion(culling, item, out, entity_start);
            } else {
                self.cull_skinned(out, entity_start, skinned);
            }
            if let Some(owned) = dynamic_state {
                scratch_layers = owned.layers;
            }
        }
    }
}

/// `EntityRenderer.shouldRender`'s box for a dropped item, in camera space:
/// `EntityRenderer.getBoundingBoxForCulling(entity, partialTick)` is
/// `Entity.getInterpolatedBoundingBox(partialTick)` - the item's 0.25 box at
/// the position the frame draws, `ItemEntity` overriding neither - grown by 0.5
/// all round before the frustum test. `position` is the already-extracted f64
/// frame position, shared with drawing and lighting.
fn dropped_item_cull_box(position: glam::DVec3, camera: glam::DVec3) -> (glam::Vec3, glam::Vec3) {
    let feet = (position - camera).as_vec3();
    let half = glam::Vec3::new(0.125 + 0.5, 0.5, 0.125 + 0.5);
    (
        feet - half,
        feet + glam::Vec3::new(half.x, 0.25 + 0.5, half.z),
    )
}

/// The packed light of an item (sky low, block above it: `vertex_light`, entity.wgsl), its block
/// light raised to what it glows with in HDR output.
fn item_hdr_light(state: &ItemRenderState, light: u32) -> u32 {
    item_hdr_light_level(state.hdr_light, light)
}

fn item_hdr_light_level(hdr_light: u8, light: u32) -> u32 {
    let block = ((light >> 8) & 0xff).max(u32::from(hdr_light) * 16);
    (light & !0xff00) | block << 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ClientHost;
    use serde_json::json;

    /// The resolver skips a mod that leaves `item_model_property` at its default;
    /// a mod that implements it is asked on every resolve.
    #[test]
    #[ignore = "daily"]
    fn item_model_property_is_asked_only_while_a_mod_answers() {
        use mod_api::{self, HookId, Hooks, ItemModelPropertyEvent, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct Quiet;
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {}
        struct Charged(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Charged {
            fn item_model_property(
                &self,
                _: &dyn ClientHost,
                event: &mut ItemModelPropertyEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::Relaxed);
                event.value = event.property == "example:charged";
            }
        }
        let stack = StackView::new("minecraft:stick", 1);

        let mods: mod_api::Mods = vec![ModEntry::new("quiet", Box::new(Quiet))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                assert!(!mod_api::__private::listened(HookId::ItemModelProperty));
                assert!(!env.item_model_property("example:charged", &stack));
                assert!(!mod_api::__private::listened(HookId::ItemModelProperty));
                assert!(!env.item_model_property("example:charged", &stack));
            });
        });

        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods =
            vec![ModEntry::new("charged", Box::new(Charged(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                for _ in 0..3 {
                    assert!(env.item_model_property("example:charged", &stack));
                }
                assert!(mod_api::__private::listened(HookId::ItemModelProperty));
            });
        });
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    struct Fill;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Fill {
        fn register_range_select_item_model_property(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRangeSelectItemModelPropertyEvent<'_>,
        ) {
            event.properties.push("example:fill".into());
        }
        fn item_model_range_property(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::ItemModelRangePropertyEvent<'_>,
        ) {
            if event.property == "example:fill" {
                event.value = event.stack.count() as f32 / 64.0;
            }
        }
    }

    /// A mod registers its numeric property once at start.
    #[test]
    fn a_mod_registers_its_range_select_item_model_property() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("fill", Box::new(Fill))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            Game::native_register_range_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            assert_eq!(properties, ["example:fill"]);
        });
    }

    /// The resolver reads a mod's number for its property, 0 for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_model_range_property_reads_the_mods_number() {
        let stack = StackView::new("minecraft:stick", 16);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("fill", Box::new(Fill))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                assert_eq!(env.item_model_range_property("example:fill", &stack), 0.25);
                assert_eq!(env.item_model_range_property("other:fill", &stack), 0.0);
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                assert_eq!(env.item_model_range_property("example:fill", &stack), 0.0);
            });
        });
    }

    struct Mode;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Mode {
        fn register_select_item_model_property(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterSelectItemModelPropertyEvent<'_>,
        ) {
            event.properties.push("example:mode".into());
        }
        fn item_model_select_property(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::ItemModelSelectPropertyEvent<'_>,
        ) {
            if event.property == "example:mode" {
                *event.value = Some(
                    if event.stack.count() > 1 {
                        "build"
                    } else {
                        "dig"
                    }
                    .into(),
                );
            }
        }
    }

    /// A mod registers its categorical property once at start.
    #[test]
    fn a_mod_registers_its_select_item_model_property() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mode", Box::new(Mode))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            Game::native_register_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            assert_eq!(properties, ["example:mode"]);
        });
    }

    /// The resolver reads a mod's value for its property, none for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_model_select_property_reads_the_mods_value() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mode", Box::new(Mode))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                let one = StackView::new("minecraft:stick", 1);
                let two = StackView::new("minecraft:stick", 2);
                assert_eq!(
                    env.item_model_select_property("example:mode", &one)
                        .as_deref(),
                    Some("dig")
                );
                assert_eq!(
                    env.item_model_select_property("example:mode", &two)
                        .as_deref(),
                    Some("build")
                );
                assert_eq!(env.item_model_select_property("other:mode", &one), None);
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                let one = StackView::new("minecraft:stick", 1);
                assert_eq!(env.item_model_select_property("example:mode", &one), None);
            });
        });
    }

    /// `EntityRenderer.shouldRender` culls with `getBoundingBoxForCulling(entity,
    /// partialTick)`, which is `Entity.getInterpolatedBoundingBox(partialTick)`:
    /// an item's 0.25 box at the position the frame draws, grown by 0.5. The box
    /// was built from the tick's own position, so an item crossing the frustum
    /// edge mid-tick disappeared for the frames before it got there.
    #[test]
    fn a_dropped_item_is_culled_at_the_frame_s_position() {
        let camera = glam::DVec3::ZERO;
        let previous = glam::DVec3::new(0.0, 64.0, 0.0);
        let position = glam::DVec3::new(1.0, 64.0, 0.0);
        let bounds = |alpha: f64| {
            let (lo, hi) = dropped_item_cull_box(previous.lerp(position, alpha), camera);
            (lo.to_array(), hi.to_array())
        };
        // The frame's start: the box stands at the item's previous position.
        assert_eq!(bounds(0.0), ([-0.625, 63.5, -0.625], [0.625, 64.75, 0.625]));
        // Halfway through the tick the box has travelled half the block.
        assert_eq!(bounds(0.5), ([-0.125, 63.5, -0.625], [1.125, 64.75, 0.625]));
        // And at its end it is where the old code put it on every frame.
        assert_eq!(bounds(1.0), ([0.375, 63.5, -0.625], [1.625, 64.75, 0.625]));
        // A falling item moves in y the same way: halfway down a 66 -> 65 drop
        // the box's floor is at 65.0, where the old code had the tick's 64.5.
        let (lo, hi) = dropped_item_cull_box(
            glam::DVec3::new(0.0, 66.0, 0.0).lerp(glam::DVec3::new(0.0, 65.0, 0.0), 0.5),
            camera,
        );
        assert!((lo.y - 65.0).abs() < 1e-6);
        assert!((hi.y - 66.25).abs() < 1e-6);
    }

    struct Charge;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Charge {
        fn register_item_tint_sources(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterItemTintSourcesEvent<'_>,
        ) {
            event.sources.push("example:charge".into());
        }
        fn item_tint_source(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::ItemTintSourceEvent<'_>,
        ) {
            if event.source == "example:charge" {
                event.color = match event.stack.count() {
                    1 => {
                        (match event.config.get("low") {
                            Some(horizon_core::nbt::Tag::Long(v)) => *v,
                            _ => 0,
                        }) as u32
                    }
                    _ => 0xFF11_2233,
                };
            }
        }
    }

    /// A mod's item model type: a cell that draws `gui` in a screen and `ground` elsewhere, `full`
    /// or `empty` by the stack, and reports the seed, the owner's type and the level it was told,
    /// as names the test reads back.
    struct Cell;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cell {
        fn register_item_models(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterItemModelsEvent<'_>,
        ) {
            event.models.push("cellmod:cell".into());
        }
        fn item_model_update(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::ItemModelUpdateEvent<'_>,
        ) {
            if event.kind != "cellmod:cell" {
                return;
            }
            let place = match event.display {
                mod_api::ItemDisplayContext::Gui => "gui",
                _ => "ground",
            };
            let fill = if event.stack.count() > 1 {
                "full"
            } else {
                "empty"
            };
            event.models.push(format!("{place}/{fill}"));
            event.models.push(format!("seed {}", event.seed));
            if let Some(owner) = event.owner {
                event.models.push(format!("owner {}", owner.entity_type));
            }
            if let Some(level) = event.level {
                event.models.push(format!("level {}", level.dimension));
            }
        }
    }

    /// A mod registers its item model type once at start, and the game's resolver gives it what
    /// `ItemModel.update` reads: the stack, where the item is drawn, its owner and level when it
    /// has them, and the seed. Another type, or no mod, draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_mods_item_model_type_answers_with_the_draws_context() {
        use horizon_assets::item_model::{Context, DisplayContext, LevelState, OwnerState};
        use mod_api::__private::game::*;
        let config =
            horizon_core::json_nbt::json_to_tag(&serde_json::json!({"type": "cellmod:cell"}))
                .unwrap();
        let owner = OwnerState {
            entity_type: "minecraft:zombie".into(),
            ..OwnerState::default()
        };
        let level = LevelState {
            dimension: "minecraft:the_nether".into(),
            ..LevelState::default()
        };
        let in_screen = Context {
            display: DisplayContext::Gui,
            owner: None,
            level: None,
            seed: 7,
        };
        let on_ground = Context {
            display: DisplayContext::Ground,
            owner: Some(&owner),
            level: Some(&level),
            seed: 42,
        };
        let drawn = |env: &Env<'_, '_>, kind: &str, count: i32, ctx: &Context<'_>| {
            let mut models = Vec::new();
            env.item_model_update(
                kind,
                &config,
                &StackView::new("minecraft:stick", count),
                ctx,
                &mut models,
            );
            models
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("cellmod", Box::new(Cell))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            Game::native_register_item_models(&mod_api::RegistrationContext::new(), &mut types);
            assert_eq!(types, ["cellmod:cell"]);
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                assert_eq!(
                    drawn(&env, "cellmod:cell", 1, &in_screen),
                    ["gui/empty", "seed 7"]
                );
                assert_eq!(
                    drawn(&env, "cellmod:cell", 2, &on_ground),
                    [
                        "ground/full",
                        "seed 42",
                        "owner minecraft:zombie",
                        "level minecraft:the_nether"
                    ]
                );
                assert!(drawn(&env, "othermod:cell", 2, &on_ground).is_empty());
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            super::super::native_entity_refs::tests::with_game(|game| {
                assert!(drawn(&Env { game }, "cellmod:cell", 2, &on_ground).is_empty());
            });
        });
    }

    /// A mod registers its item tint source once at start.
    #[test]
    fn a_mod_registers_its_item_tint_source() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("charge", Box::new(Charge))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut sources = Vec::new();
            Game::native_register_item_tint_sources(
                &mod_api::RegistrationContext::new(),
                &mut sources,
            );
            assert_eq!(sources, ["example:charge"]);
        });
    }

    /// The resolver reads a mod's colour per stack, white for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_tint_source_reads_the_mods_colour() {
        let config = horizon_core::json_nbt::json_to_tag(
            &serde_json::json!({"type": "example:charge", "low": 0xFFAA_BBCC_u32}),
        )
        .unwrap();
        let one = StackView::new("minecraft:stick", 1);
        let two = StackView::new("minecraft:stick", 2);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("charge", Box::new(Charge))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                let color = |source, stack| env.item_tint_source(source, &config, stack) as u32;
                assert_eq!(color("example:charge", &one), 0xFFAA_BBCC);
                assert_eq!(color("example:charge", &two), 0xFF11_2233);
                assert_eq!(color("other:charge", &one), 0xFFFF_FFFF);
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let env = Env { game };
                assert_eq!(env.item_tint_source("example:charge", &config, &one), -1);
            });
        });
    }

    #[test]
    fn civil_dates_follow_the_gregorian_calendar() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_716), (2023, 12, 25));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_shield_is_used_for_the_vanilla_s_time() {
        // Item.getUseDuration: an item with `blocks_attacks` (a shield) is
        // used for 72000 ticks, so its use animation (the arm held up)
        // runs instead of ending at once (arismoko/horizon#2289).
        let mut shield = Components::new();
        shield.insert("minecraft:blocks_attacks".into(), json!({}));
        assert_eq!(use_duration(ComponentsView::of(&shield)), 72_000);
        // Food is used for as long as it takes to eat it.
        let mut food = Components::new();
        food.insert(
            "minecraft:consumable".into(),
            json!({"consume_seconds": 1.6}),
        );
        assert_eq!(use_duration(ComponentsView::of(&food)), 32);
        // The bow keeps its own use time.
        let mut bow = Components::new();
        bow.insert("minecraft:item_model".into(), json!("minecraft:bow"));
        assert_eq!(use_duration(ComponentsView::of(&bow)), 72_000);
        // Anything else is not used at all.
        assert_eq!(use_duration(ComponentsView::of(&Components::new())), 0);
    }
}
