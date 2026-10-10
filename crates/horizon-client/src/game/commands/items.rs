//! `/item` (ItemCommands) and `if items`/`if slots`: addressing the item
//! slots of the player, mobs, armor stands, item frames and containers.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    command::{CommandError, args::ItemInput},
    items::ItemStack,
    scoreboard_cmd::{Holder, Tracker},
    text::Text,
};

/// One addressable slot.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum Slot {
    /// `inventory[0..36]`: hotbar then the main rows.
    Inv(usize),
    /// `armor[0..4]` (head, chest, legs, feet).
    Armor(usize),
    Offhand,
    Ender(usize),
    /// A mob's equipment by name (`mainhand`, `offhand`, `head`, ...).
    Mob(u32, &'static str),
    /// An armor stand's equipment (`ArmorStand::SLOTS` index).
    Stand(u32, usize),
    Frame(u32),
    Block([i32; 3], usize),
}
/// Whose slots a command addresses.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum Owner {
    Entity(Target),
    Block([i32; 3]),
}

/// The player's slot names and the inventory index each addresses.
fn player_slot_names() -> Vec<(String, usize)> {
    let mut v = Vec::new();
    for i in 0..36 {
        v.push((format!("container.{i}"), i));
    }
    for i in 0..9 {
        v.push((format!("hotbar.{i}"), i));
    }
    for i in 0..27 {
        v.push((format!("inventory.{i}"), 9 + i));
    }
    v
}
/// `name` against a group: `container.*` matches every `container.N`.
fn matches_name(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix(".*") {
        Some(prefix) => name.starts_with(&format!("{prefix}.")),
        None => pattern == name,
    }
}

impl Game<'_> {
    fn owner_of(&mut self, ctx: &Context, prefix: &str, many: bool) -> R<Vec<Owner>> {
        let kind = ctx
            .literals
            .iter()
            .filter(|l| matches!(l.as_str(), "entity" | "block"))
            .nth(usize::from(prefix == "source" && ctx.has("from")))
            .cloned()
            .unwrap_or_default();
        if kind == "block" {
            let p = self.block_pos_arg(ctx, &format!("{prefix}Pos"))?;
            return Ok(vec![Owner::Block(p)]);
        }
        let targets = self.entities(ctx, prefix, false)?;
        let _ = many;
        Ok(targets.into_iter().map(Owner::Entity).collect())
    }
    /// The slots a name (or wildcard group) addresses on an owner.
    pub(in crate::game) fn slots_named(&mut self, owner: &Owner, name: &str) -> Vec<Slot> {
        let mut out = Vec::new();
        let mut add = |s: Slot| out.push(s);
        match owner {
            Owner::Block(p) => {
                let size = match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(horizon_client::containers::BlockEntity::Chest(items)) => items.len(),
                    _ => 0,
                };
                for i in 0..size {
                    if matches_name(name, &format!("container.{i}")) {
                        add(Slot::Block(*p, i));
                    }
                }
            }
            Owner::Entity(Target::Player) => {
                for (n, i) in player_slot_names() {
                    if matches_name(name, &n) {
                        add(Slot::Inv(i));
                    }
                }
                for (i, n) in ["armor.head", "armor.chest", "armor.legs", "armor.feet"]
                    .into_iter()
                    .enumerate()
                {
                    if matches_name(name, n) {
                        add(Slot::Armor(i));
                    }
                }
                if matches_name(name, "weapon") || matches_name(name, "weapon.mainhand") {
                    add(Slot::Inv(self.selected));
                }
                if matches_name(name, "weapon.offhand") {
                    add(Slot::Offhand);
                }
                for i in 0..27 {
                    if matches_name(name, &format!("enderchest.{i}")) {
                        add(Slot::Ender(i));
                    }
                }
            }
            Owner::Entity(Target::Mob(id)) => {
                for (n, key) in [
                    ("weapon", "mainhand"),
                    ("weapon.mainhand", "mainhand"),
                    ("weapon.offhand", "offhand"),
                    ("armor.head", "head"),
                    ("armor.chest", "chest"),
                    ("armor.legs", "legs"),
                    ("armor.feet", "feet"),
                    ("armor.body", "body"),
                    ("saddle", "saddle"),
                ] {
                    if matches_name(name, n) {
                        add(Slot::Mob(*id, key));
                    }
                }
            }
            Owner::Entity(Target::Stand(id)) => {
                for (i, key) in horizon_client::entities::ArmorStand::SLOTS
                    .iter()
                    .enumerate()
                {
                    let aliases: &[&str] = match *key {
                        "mainhand" => &["weapon", "weapon.mainhand"],
                        "offhand" => &["weapon.offhand"],
                        "feet" => &["armor.feet"],
                        "legs" => &["armor.legs"],
                        "chest" => &["armor.chest"],
                        _ => &["armor.head"],
                    };
                    if aliases.iter().any(|a| matches_name(name, a)) {
                        add(Slot::Stand(*id, i));
                    }
                }
            }
            Owner::Entity(Target::Frame(id)) => {
                if matches_name(name, "container.0") || matches_name(name, "contents") {
                    add(Slot::Frame(*id));
                }
            }
            Owner::Entity(_) => {}
        }
        out
    }
    pub(in crate::game) fn slot_get(&self, s: &Slot) -> Option<ItemStack> {
        match s {
            Slot::Inv(i) => self.inventory.get(*i).cloned().flatten(),
            Slot::Armor(i) => self.armor.get(*i).cloned().flatten(),
            Slot::Offhand => self.offhand.clone(),
            Slot::Ender(i) => self.ender_items.get(*i).cloned().flatten(),
            Slot::Mob(id, key) => {
                let m = self.mobs.iter().find(|m| m.id == *id)?;
                let item = match *key {
                    "mainhand" => &m.extra.main_hand,
                    "offhand" => &m.extra.off_hand,
                    "head" | "body" => &m.extra.head,
                    "chest" => &m.extra.chest,
                    "legs" => &m.extra.legs,
                    "feet" => &m.extra.feet,
                    _ => return None,
                };
                item.as_ref().map(|i| ItemStack {
                    item: i.clone(),
                    count: 1,
                    damage: 0,
                    extra: None,
                })
            }
            Slot::Stand(id, i) => self
                .armor_stands
                .iter()
                .find(|a| a.id == *id)?
                .equipment
                .get(*i)
                .cloned()
                .flatten(),
            Slot::Frame(id) => self.item_frames.iter().find(|f| f.id == *id)?.item.clone(),
            Slot::Block(p, i) => match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                Some(horizon_client::containers::BlockEntity::Chest(items)) => {
                    items.get(*i).cloned().flatten()
                }
                _ => None,
            },
        }
    }
    /// Put a stack (or nothing) in a slot; false when the slot cannot take it.
    pub(in crate::game) fn slot_set(&mut self, s: &Slot, stack: Option<ItemStack>) -> bool {
        match s {
            Slot::Inv(i) => match self.inventory.get_mut(*i) {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Armor(i) => match self.armor.get_mut(*i) {
                Some(slot) => {
                    *slot = stack;
                    self.apply_equipment();
                    true
                }
                None => false,
            },
            Slot::Offhand => {
                self.offhand = stack;
                true
            }
            Slot::Ender(i) => match self.ender_items.get_mut(*i) {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Mob(id, key) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == *id) else {
                    return false;
                };
                let item = stack.map(|s| s.item);
                match *key {
                    "mainhand" => m.extra.main_hand = item,
                    "offhand" => m.extra.off_hand = item,
                    "head" | "body" => m.extra.head = item,
                    "chest" => m.extra.chest = item,
                    "legs" => m.extra.legs = item,
                    "feet" => m.extra.feet = item,
                    _ => return false,
                }
                true
            }
            Slot::Stand(id, i) => match self
                .armor_stands
                .iter_mut()
                .find(|a| a.id == *id)
                .and_then(|a| a.equipment.get_mut(*i))
            {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Frame(id) => match self.item_frames.iter_mut().find(|f| f.id == *id) {
                Some(f) => {
                    f.item = stack;
                    true
                }
                None => false,
            },
            Slot::Block(p, i) => match self.world.block_entities.get_mut(&(p[0], p[1], p[2])) {
                Some(horizon_client::containers::BlockEntity::Chest(items)) => {
                    match items.get_mut(*i) {
                        Some(slot) => {
                            *slot = stack;
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            },
        }
    }
    fn owner_holder(&self, o: &Owner) -> Holder {
        match o {
            Owner::Entity(t) => Holder {
                name: String::new(),
                text: self.target_name(*t),
                display: None,
            },
            Owner::Block(p) => Holder {
                name: String::new(),
                text: Text::literal(format!("{}, {}, {}", p[0], p[1], p[2])),
                display: None,
            },
        }
    }
    fn stack_name(&self, s: &ItemStack) -> Text {
        self.item_display_name(&s.item)
    }
    fn item_from_input(&self, ctx: &Context) -> R<Option<ItemStack>> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(None);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(horizon_client::item_data::command_components)
        {
            Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let count = Self::int(ctx, "count").unwrap_or(1).max(1) as u32;
        Ok(Some(ItemStack {
            item: id.as_str().into(),
            count,
            damage,
            extra,
        }))
    }
    /// `/item replace|fill|override <target> <slots> (with <item> [count] | from <source> <slots>)`.
    pub(super) fn cmd_item(&mut self, ctx: &Context) -> R<i32> {
        let mode = ctx
            .literals
            .iter()
            .find(|l| matches!(l.as_str(), "replace" | "fill" | "override"))
            .cloned()
            .unwrap_or_default();
        let targets = self.owner_of(ctx, "target", true)?;
        let Some(Arg::Str(slot_name)) = ctx.get("slots").cloned() else {
            return Ok(0);
        };
        // The items to place.
        let (items, known): (Vec<Option<ItemStack>>, Option<ItemStack>) = if ctx.has("with") {
            let stack = self.item_from_input(ctx)?;
            (vec![stack.clone()], stack)
        } else {
            let sources = self.owner_of(ctx, "source", true)?;
            let Some(Arg::Str(source_slots)) = ctx.get("sourceSlots").cloned() else {
                return Ok(0);
            };
            let mut found = Vec::new();
            for o in &sources {
                for s in self.slots_named(o, &source_slots) {
                    found.push(self.slot_get(&s));
                }
            }
            if found.is_empty() {
                return Err(match &sources[..] {
                    [Owner::Block(p)] => err(
                        "commands.item.source.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ),
                    _ => err("commands.item.source.no_such_slot", &[&source_slots]),
                });
            }
            // A source with nothing in the slot contributes air.
            (found, None)
        };
        let mut tracker = Tracker::default();
        let mut selected = 0;
        for o in &targets {
            let slots = self.slots_named(o, &slot_name);
            if slots.is_empty() {
                if let Owner::Block(p) = o
                    && !self.world.block_entities.contains_key(&(p[0], p[1], p[2]))
                {
                    return Err(err(
                        "commands.item.target.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ));
                }
                continue;
            }
            selected += slots.len();
            let mut changed = 0;
            for (i, slot) in slots.iter().enumerate() {
                // `fill` cycles through the items; `replace` stops when they
                // run out; `override` empties the rest.
                let item = match mode.as_str() {
                    "fill" => items.get(i % items.len()).cloned().flatten(),
                    "override" => items.get(i).cloned().flatten(),
                    _ => match items.get(i) {
                        Some(x) => x.clone(),
                        None => break,
                    },
                };
                if self.slot_get(slot) != item && self.slot_set(slot, item) {
                    changed += 1;
                }
            }
            let h = self.owner_holder(o);
            tracker.track(&h, changed);
        }
        if selected == 0 {
            return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
        }
        let block = matches!(targets.first(), Some(Owner::Block(_)));
        let pos = match targets.first() {
            Some(Owner::Block(p)) => *p,
            _ => [0; 3],
        };
        let zero = match &known {
            Some(k) => err(
                "commands.item.target.failed.known_item",
                &[&k.item.to_string()],
            ),
            None => err("commands.item.target.failed", &[]),
        };
        let name = known.as_ref().map(|k| self.stack_name(k));
        let (message, total) = tracker.message(
            false,
            Some(zero),
            |h, count| {
                let n = Text::literal(count.to_string());
                match (&name, block) {
                    (Some(item), true) => Text::translate(
                        "commands.item.block.replace.success.known_item",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                            item.clone(),
                        ],
                    ),
                    (None, true) => Text::translate(
                        "commands.item.block.replace.success",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                        ],
                    ),
                    (Some(item), false) => Text::translate(
                        "commands.item.entity.replace.success.single.known_item",
                        &[n, h.text.clone(), item.clone()],
                    ),
                    (None, false) => Text::translate(
                        "commands.item.entity.replace.success.single",
                        &[n, h.text.clone()],
                    ),
                }
            },
            |c, _| match &name {
                Some(item) => Text::translate(
                    "commands.item.entity.replace.success.multiple.known_item",
                    &[Text::literal(c.to_string()), item.clone()],
                ),
                None => Text::translate(
                    "commands.item.entity.replace.success.multiple",
                    &[Text::literal(c.to_string())],
                ),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `if items` and `if slots`: how many items match / how many slots.
    pub(super) fn condition_items(&mut self, c: &Context) -> R<(bool, i32)> {
        let owners = self.owner_of(c, "source", true)?;
        let Some(Arg::Str(slot_name)) = c.get("slots").cloned() else {
            return Ok((false, 0));
        };
        let predicate = match c.get("item_predicate") {
            Some(Arg::Item(p)) => Some(p.clone()),
            _ => None,
        };
        let mut count = 0;
        for o in owners {
            let slots = self.slots_named(&o, &slot_name);
            if predicate.is_none() {
                count += slots.len() as i32;
                continue;
            }
            for s in slots {
                if let Some(stack) = self.slot_get(&s)
                    && predicate
                        .as_ref()
                        .is_some_and(|p| self.item_matches(p, &stack.item))
                {
                    count += stack.count as i32;
                }
            }
        }
        Ok((count > 0, count))
    }
}
