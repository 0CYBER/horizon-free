//! The player's advancements in the game: criteria triggered from game
//! events (`CriteriaTriggers`), awarding with rewards, the toast and the
//! chat announcement, and `players/advancements/<uuid>.json`.
use super::{Dim, Game};
use horizon_client::{
    advancements::{self as adv, EntityView, Frame, ItemView, Progress, Tree},
    hud::HudBatch,
    items::ItemStack,
    text::{self, Style, Text},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Instant,
};

pub(super) struct Toast {
    icon: String,
    frame: Frame,
    title: Text,
    shown: Option<Instant>,
}
#[derive(Default)]
pub(super) struct AdvState {
    pub(super) tree: Arc<Tree>,
    pub(super) progress: Progress,
    loaded: bool,
    pub(super) recipes: BTreeSet<String>,
    toasts: VecDeque<Toast>,
    /// Awards skip the toast (`/advancement grant ... everything`).
    pub(super) quiet: bool,
    /// The inventory last seen by `inventory_changed`.
    last_inventory: u64,
}
/// The game's world for predicates.
struct View<'a, 'g>(&'a Game<'g>);
impl adv::World for View<'_, '_> {
    fn dimension(&self) -> &str {
        self.0.dim.id()
    }
    fn biome(&self, p: [i32; 3]) -> String {
        self.0
            .world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map(|c| {
                self.0
                    .res
                    .biomes
                    .get(c.biome(p[0], p[1], p[2]))
                    .name
                    .clone()
            })
            .unwrap_or_default()
    }
    fn structures(&self, p: [i32; 3]) -> Vec<String> {
        self.0.structures_at(p)
    }
    fn block(&self, p: [i32; 3]) -> (String, BTreeMap<String, String>) {
        let state = self.0.world.block(p[0], p[1], p[2]);
        let name = self.0.res.facts.get(state).name.to_string();
        let props = self
            .0
            .res
            .assets
            .registry
            .state(state)
            .map(|s| {
                s.properties
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (name, props)
    }
    fn fluid(&self, p: [i32; 3]) -> Option<String> {
        match self
            .0
            .res
            .facts
            .get(self.0.world.block(p[0], p[1], p[2]))
            .fluid
        {
            horizon_client::blocks::Fluid::Water => Some("minecraft:water".into()),
            horizon_client::blocks::Fluid::Lava => Some("minecraft:lava".into()),
            _ => None,
        }
    }
    fn can_see_sky(&self, p: [i32; 3]) -> bool {
        (p[1] + 1..p[1] + 200).all(|y| {
            !self
                .0
                .res
                .facts
                .get(self.0.world.block(p[0], y, p[2]))
                .full_collision
        })
    }
    fn light(&self, p: [i32; 3]) -> i32 {
        let l = self.0.world.light(p[0], p[1], p[2]);
        i32::from((l >> 4).max(l & 15))
    }
    fn smokey(&self, p: [i32; 3]) -> bool {
        (1..=5).any(|d| {
            self.0
                .res
                .facts
                .get(self.0.world.block(p[0], p[1] - d, p[2]))
                .name
                .ends_with("campfire")
        })
    }
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        let items = &self.0.res.items;
        match registry {
            "item" => items.item_tags.get(tag).is_some_and(|t| t.contains(id)),
            "block" => items.block_tags.get(tag).is_some_and(|t| t.contains(id)),
            "entity_type" => self.0.res.mob_data.entity_has(tag, id),
            "worldgen/biome" => self.0.res.mob_data.biome_has(tag, id),
            _ => false,
        }
    }
}
fn now_text() -> String {
    // "yyyy-MM-dd HH:mm:ss Z" in UTC.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
pub(super) fn item_view(s: &ItemStack) -> ItemView {
    ItemView {
        id: s.item.to_string(),
        count: s.count,
        damage: s.damage,
        enchantments: s
            .enchantments()
            .iter()
            .map(|(id, l)| (id.to_string(), *l))
            .collect(),
        potion: s.potion().map(str::to_owned),
        custom_name: s.custom_name().is_some(),
    }
}
impl Game<'_> {
    fn adv_path(&self) -> Option<std::path::PathBuf> {
        let uuid = self.level.player_uuid?;
        let hex: String = uuid.iter().map(|v| format!("{:08x}", *v as u32)).collect();
        let text = format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );
        Some(
            self.world_root
                .join("players/advancements")
                .join(format!("{text}.json")),
        )
    }
    pub(super) fn load_advancements(&mut self) {
        if self.adv.loaded || self.remote.is_some() {
            return;
        }
        self.adv.loaded = true;
        if self.adv.tree.list.is_empty() {
            self.adv.tree = Arc::new(Tree::load(&self.options.resources.join("data")));
        }
        let progress = self
            .adv_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Progress::from_json(&v))
            .unwrap_or_default();
        // Unlocked recipes come from the done recipe advancements' rewards.
        let tree = self.adv.tree.clone();
        for a in &tree.list {
            if progress.done(a) {
                self.adv.recipes.extend(a.rewards.recipes.iter().cloned());
            }
        }
        self.adv.progress = progress;
        self.ui.registries.advancements = tree.list.iter().map(|a| a.id.clone()).collect();
    }
    pub(super) fn save_advancements(&self) {
        if !self.adv.loaded || self.remote.is_some() {
            return;
        }
        let Some(path) = self.adv_path() else {
            return;
        };
        let json = self
            .adv
            .progress
            .to_json(&self.adv.tree, horizon_world::DATA_VERSION);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
    }
    /// The player as predicates see it.
    /// The predicate view of a command target.
    pub(super) fn entity_view_of(&self, t: super::commands::Target) -> EntityView {
        use super::commands::Target;
        match t {
            Target::Player => self.player_entity(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m))
                .unwrap_or_default(),
            other => EntityView {
                kind: self.target_type(other),
                position: self.target_position(other).unwrap_or_default(),
                ..EntityView::default()
            },
        }
    }
    /// A `ContextAwarePredicate` for `this` at `origin`.
    pub(super) fn conditions_hold(
        &self,
        v: &Value,
        this: Option<&EntityView>,
        origin: [f64; 3],
    ) -> bool {
        adv::conditions(
            v,
            &adv::Entities {
                this,
                origin,
                tool: None,
                block_pos: None,
            },
            &View(self),
        )
    }
    pub(super) fn player_entity(&self) -> EntityView {
        let Some(p) = self.player.as_ref() else {
            return EntityView::default();
        };
        let mut eq: [Option<ItemView>; 7] = Default::default();
        eq[0] = self.held().map(item_view);
        eq[1] = self.offhand.as_ref().map(item_view);
        for (i, a) in self.armor.iter().enumerate() {
            eq[2 + i] = a.as_ref().map(item_view);
        }
        let vehicle = self.riding.and_then(|id| {
            self.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| Box::new(self.mob_entity(m)))
        });
        EntityView {
            kind: "minecraft:player".into(),
            position: [p.position.x, p.position.y, p.position.z],
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
            swimming: p.pose == horizon_client::pose::Pose::Swimming,
            flying: p.flying,
            on_ground: p.ground.on_ground,
            equipment: eq,
            vehicle,
            effects: self
                .potions
                .effects
                .list
                .iter()
                .map(|e| (format!("minecraft:{}", e.id), e.amplifier, e.duration))
                .collect(),
            gamemode: Some(self.mode.name().to_owned()),
            ..Default::default()
        }
    }
    pub(super) fn mob_entity(&self, m: &horizon_client::mobs::Mob) -> EntityView {
        EntityView {
            kind: m.kind.id.clone(),
            position: [m.position.x, m.position.y, m.position.z],
            on_fire: m.fire_ticks > 0,
            baby: m.is_baby(),
            on_ground: m.on_ground,
            ..Default::default()
        }
    }
    /// `SimpleCriterionTrigger.trigger`: test each open criterion of the
    /// trigger and award what it completes.
    pub(super) fn trigger(&mut self, name: &str, test: impl Fn(&Value, &dyn adv::World) -> bool) {
        if self.remote.is_some() {
            return;
        }
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let Some(list) = tree.by_trigger.get(name) else {
            return;
        };
        let mut hits = Vec::new();
        {
            let view = View(self);
            for &(ai, ci) in list {
                let a = &tree.list[ai];
                let c = &a.criteria[ci];
                if self.adv.progress.has(&a.id, &c.name) || self.adv.progress.done(a) {
                    continue;
                }
                if test(&c.conditions, &view) {
                    hits.push((ai, ci));
                }
            }
        }
        let date = now_text();
        for (ai, ci) in hits {
            let a = &tree.list[ai];
            let was = self.adv.progress.done(a);
            self.adv.progress.grant(&a.id, &a.criteria[ci].name, &date);
            if !was && self.adv.progress.done(a) {
                self.award(ai);
            }
        }
    }
    /// `PlayerAdvancements.award` once done: rewards, toast, announcement.
    fn award(&mut self, index: usize) {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        // AdvancementRewards.grant
        if a.rewards.experience > 0 {
            self.experience.give(a.rewards.experience);
        }
        let mut unlocked = Vec::new();
        for r in &a.rewards.recipes {
            if self.adv.recipes.insert(r.clone()) {
                unlocked.push(r.clone());
            }
        }
        for table in &a.rewards.loot {
            let items = self
                .res
                .items
                .loot_table(table, false, None, &mut self.world.random);
            for s in items {
                if let Some(rest) = self.give(s)
                    && let Some(eye) = self.eye_position()
                {
                    self.spawn_item(rest, eye, [0.0; 3], 0);
                }
            }
        }
        if let Some(d) = &a.display {
            if d.show_toast && !self.adv.quiet {
                self.adv.toasts.push_back(Toast {
                    icon: d.icon.clone(),
                    frame: d.frame,
                    title: text_of(&d.title),
                    shown: None,
                });
            }
            if d.announce && self.ui.rules.bool("show_advancement_messages") {
                let key = match d.frame {
                    Frame::Task => "chat.type.advancement.task",
                    Frame::Goal => "chat.type.advancement.goal",
                    Frame::Challenge => "chat.type.advancement.challenge",
                };
                let name = adv_name(a);
                self.system_message(Text::translate(
                    key,
                    &[Text::literal(super::chat::PLAYER_NAME), name],
                ));
            }
        }
        for r in unlocked {
            self.trigger("minecraft:recipe_unlocked", |c, _| {
                c["recipes"].as_str() == Some(r.as_str())
            });
        }
    }
    /// `InventoryChangeTrigger`, checked when the inventory changes.
    pub(super) fn check_inventory_trigger(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
        {
            s.as_ref().map(|s| (&s.item, s.count)).hash(&mut h);
        }
        let key = h.finish();
        if key == self.adv.last_inventory {
            return;
        }
        self.adv.last_inventory = key;
        let stacks: Vec<ItemView> = self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
            .flatten()
            .map(item_view)
            .collect();
        // InventoryChangeTrigger slots: occupied, full and empty counts.
        let all: Vec<&Option<ItemStack>> = self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
            .collect();
        let occupied = all.iter().filter(|s| s.is_some()).count() as f64;
        let empty = all.len() as f64 - occupied;
        let full = all
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|s| {
                self.res
                    .items
                    .get(&s.item)
                    .is_some_and(|i| s.count >= i.max_stack)
            })
            .count() as f64;
        self.trigger("minecraft:inventory_changed", move |c, w| {
            let slots = &c["slots"];
            let range = |k: &str, v: f64| {
                let r = &slots[k];
                r.is_null()
                    || r.as_f64().is_some_and(|n| (n - v).abs() < 0.5)
                    || r.is_object()
                        && r["min"].as_f64().is_none_or(|m| v >= m)
                        && r["max"].as_f64().is_none_or(|m| v <= m)
            };
            if !(range("occupied", occupied) && range("full", full) && range("empty", empty)) {
                return false;
            }
            let mut wanted: Vec<&Value> = c["items"]
                .as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            for s in &stacks {
                wanted.retain(|p| !adv::item(p, s, w));
            }
            wanted.is_empty()
        });
    }
    /// `PlayerTrigger.LOCATION` every second, and the other per-tick checks.
    pub(super) fn tick_advancements(&mut self) {
        if self.player.is_none() || self.remote.is_some() {
            return;
        }
        self.check_inventory_trigger();
        if self.ticks.is_multiple_of(20) {
            let me = self.player_entity();
            self.trigger("minecraft:location", |c, w| {
                let ctx = adv::Entities {
                    this: Some(&me),
                    origin: me.position,
                    tool: None,
                    block_pos: None,
                };
                adv::conditions(&c["player"], &ctx, w)
            });
        }
    }
    /// A player condition plus one entity (`player`, `entity` keys).
    pub(super) fn trigger_entity(&mut self, name: &str, key: &str, target: &EntityView) {
        let me = self.player_entity();
        let target = target.clone();
        let key = key.to_owned();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let ectx = adv::Entities {
                this: Some(&target),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::conditions(&c[&key], &ectx, w)
        });
    }
    /// A player condition plus one item (`item` key).
    pub(super) fn trigger_item(&mut self, name: &str, stack: &ItemStack) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::item(&c["item"], &it, w)
        });
    }
    /// `ItemUsedOnLocationTrigger` (placed_block, item_used_on_block).
    pub(super) fn trigger_block(&mut self, name: &str, pos: [i32; 3], stack: &ItemStack) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger(name, move |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: Some(&it),
                block_pos: Some(pos),
            };
            adv::conditions(&c["location"], &ctx, w)
        });
    }
    /// A trigger with only the player condition (slept_in_bed, started_riding).
    pub(super) fn trigger_player(&mut self, name: &str) {
        let me = self.player_entity();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `BredAnimalsTrigger`.
    pub(super) fn trigger_bred(
        &mut self,
        parent: &EntityView,
        partner: &EntityView,
        child: Option<&EntityView>,
    ) {
        let me = self.player_entity();
        let (parent, partner, child) = (parent.clone(), partner.clone(), child.cloned());
        self.trigger("minecraft:bred_animals", move |c, w| {
            let ctx = |e: &EntityView| {
                adv::Entities {
                    this: Some(e),
                    origin: me.position,
                    tool: None,
                    block_pos: None,
                }
                .this
                .map(|_| ())
            };
            let _ = ctx;
            let test = |key: &str, e: Option<&EntityView>| {
                c[key].is_null()
                    || e.is_some_and(|e| {
                        adv::conditions(
                            &c[key],
                            &adv::Entities {
                                this: Some(e),
                                origin: me.position,
                                tool: None,
                                block_pos: None,
                            },
                            w,
                        )
                    })
            };
            // matches(parent, partner, child) or (partner, parent, child).
            (test("parent", Some(&parent)) && test("partner", Some(&partner))
                || test("parent", Some(&partner)) && test("partner", Some(&parent)))
                && test("child", child.as_ref())
        });
    }
    /// `ChangeDimensionTrigger`.
    pub(super) fn trigger_dimension(&mut self, from: Dim, to: Dim) {
        let (from, to) = (from.id().to_owned(), to.id().to_owned());
        self.trigger("minecraft:changed_dimension", move |c, _| {
            c["from"].as_str().is_none_or(|f| f == from) && c["to"].as_str().is_none_or(|t| t == to)
        });
    }
    /// `ToastManager` with the `AdvancementToast`.
    pub(super) fn draw_toasts(&mut self, batch: &mut HudBatch) {
        let Some(t) = self.adv.toasts.front_mut() else {
            return;
        };
        let shown = *t.shown.get_or_insert_with(Instant::now);
        let ms = shown.elapsed().as_millis() as f32;
        // AdvancementToast.DISPLAY_TIME: 5 s, sliding 600 ms in and out.
        if ms > 5600.0 {
            self.adv.toasts.pop_front();
            return;
        }
        let slide = if ms < 600.0 {
            ms / 600.0
        } else if ms > 5000.0 {
            1.0 - (ms - 5000.0) / 600.0
        } else {
            1.0
        };
        let (title, frame, icon) = (t.title.clone(), t.frame, t.icon.clone());
        if frame == Frame::Challenge && ms < 50.0 {
            self.audio
                .play("minecraft:ui.toast.challenge_complete", None, 1.0, 1.0);
        }
        let atlas = &self.res.atlas;
        let x = batch.width - 160.0 * slide;
        batch.sprite(atlas, "gui/sprites/toast/advancement", x, 0.0);
        let (key, color) = match frame {
            Frame::Task => ("advancements.toast.task", 0xFF00_FFFF),
            Frame::Goal => ("advancements.toast.goal", 0xFF00_FFFF),
            Frame::Challenge => ("advancements.toast.challenge", 0xFFFF_88FF),
        };
        batch.text_styled(
            atlas,
            &Text::translate(key, &[]),
            x + 30.0,
            7.0,
            color,
            false,
        );
        batch.text_styled(atlas, &title, x + 30.0, 18.0, u32::MAX, false);
        self.draw_item(batch, &ItemStack::new(&icon, 1), x + 8.0, 8.0);
    }
}
/// `Advancement.name`: the bracketed title in the frame's colour, with the
/// description on hover; the id when there is no display.
pub(super) fn adv_name(a: &adv::Advancement) -> Text {
    let Some(d) = &a.display else {
        return Text::literal(a.id.clone());
    };
    let color = match d.frame {
        Frame::Challenge => text::DARK_PURPLE,
        _ => text::GREEN,
    };
    let tooltip = text_of(&d.title)
        .with_color(color)
        .append("\n")
        .append(text_of(&d.description));
    let title = text_of(&d.title).with_style(&Style {
        hover: Some(Box::new(tooltip)),
        ..Style::default()
    });
    Text::literal("[")
        .append(title)
        .append("]")
        .with_color(color)
}
impl Game<'_> {
    /// `AdvancementCommands.Action.GRANT` on one advancement (or one of
    /// its criteria): false when nothing changed.
    pub(super) fn grant_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        if criterion.is_none() && self.adv.progress.done(a) {
            return false;
        }
        let names: Vec<&str> = match criterion {
            Some(c) => vec![c],
            None => a.criteria.iter().map(|c| c.name.as_str()).collect(),
        };
        let date = now_text();
        let mut changed = false;
        for c in names {
            let was = self.adv.progress.done(a);
            if self.adv.progress.grant(&a.id, c, &date) {
                changed = true;
                if !was && self.adv.progress.done(a) {
                    self.award(index);
                }
            }
        }
        changed || criterion.is_none()
    }
    /// `AdvancementCommands.Action.REVOKE`.
    pub(super) fn revoke_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        match criterion {
            Some(c) => self.adv.progress.revoke(&a.id, c),
            None => {
                let mut any = false;
                for c in &a.criteria {
                    any |= self.adv.progress.revoke(&a.id, &c.name);
                }
                any
            }
        }
    }
}
/// A text component from advancement JSON (`translate` or `text`).
pub(super) fn text_of(v: &Value) -> Text {
    if let Some(key) = v["translate"].as_str() {
        Text::translate(key, &[])
    } else if let Some(s) = v["text"].as_str() {
        Text::literal(s)
    } else if let Some(s) = v.as_str() {
        Text::literal(s)
    } else {
        Text::empty()
    }
}
