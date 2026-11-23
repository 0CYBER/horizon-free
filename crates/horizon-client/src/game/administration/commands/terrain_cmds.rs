//! `/forceload`, `/fillbiome`, `/worldborder` and `/loot`.
use super::{Arg, Context, Game, R, Target, err, items::Owner};
use horizon_client::{
    command::CommandError,
    items::ItemStack,
    scoreboard_cmd::{Holder, Tracker},
    text::Text,
};
#[cfg(test)]
use horizon_core::position::ChunkPos;
use horizon_core::random::RandomSource;

/// `LocationCheck.test` / `BlockPredicate`: the measured loaded block-state subset.
fn loot_location_block(
    condition: &serde_json::Value,
    origin: [i32; 3],
    chunks: &horizon_client::chunk_map::ChunkMap,
    registry: &horizon_world::registry::BlockRegistry,
    items: &horizon_client::items::Items,
) -> Option<bool> {
    let condition = condition.as_object()?;
    if condition.keys().any(|k| {
        !matches!(
            k.as_str(),
            "type" | "predicate" | "offsetX" | "offsetY" | "offsetZ"
        )
    }) {
        return None;
    }
    let predicate = condition.get("predicate")?.as_object()?;
    if predicate.len() != 1 || !predicate.contains_key("block") {
        return None;
    }
    let block = predicate.get("block")?.as_object()?;
    if block
        .keys()
        .any(|k| !matches!(k.as_str(), "blocks" | "state"))
        || block.get("blocks")?.as_str().is_none()
    {
        return None;
    }
    if let Some(state) = block.get("state")
        && state.as_object()?.values().any(|v| !v.is_string())
    {
        return None;
    }
    let mut at = origin;
    for (axis, key) in ["offsetX", "offsetY", "offsetZ"].iter().enumerate() {
        let offset = match condition.get(*key) {
            None => 0,
            Some(value) => i32::try_from(value.as_i64()?).ok()?,
        };
        // floor(integer block origin + .5 + integer offset), also for negatives.
        at[axis] = origin[axis].checked_add(offset)?;
    }
    // Unloaded positions are unsupported: never load or invent a block here.
    // `LocationPredicate.matches`: a block check fails where `Level.isLoaded` is false.
    let Some(chunk) = chunks.get(&(at[0] >> 4, at[2] >> 4)) else {
        return Some(false);
    };
    let state = registry.state(chunk.block(at[0] & 15, at[1], at[2] & 15))?;
    Some(items.matches_block_state(&predicate["block"], &state.name, &state.properties))
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}

/// Whose hand `LootCommand.getSourceHandItem` reads a `mainhand` or `offhand` tool from.
#[derive(Debug, PartialEq)]
enum HandSource {
    /// A living entity: its hand (`LivingEntity.getItemBySlot`).
    Holder(Target),
    /// A source without an entity (a command block, a function run as the server):
    /// `getEntityOrException`'s `permissions.requires.entity`.
    NoEntity,
    /// Any other entity: `commands.drop.no_held_items`.
    CannotHold(Target),
}

/// `getSourceHandItem` for a command whose source is `source` (None: no source at all, the player in place's own
/// command; Some(None): a source without an entity). `kind` is the type of a source kept as its saved data
/// (`Target::Other`): the ones known not to be living entities hold nothing, and any other such kind is read as
/// before, so no command that worked starts failing for a type Horizon doesn't simulate.
fn hand_source(source: Option<Option<Target>>, kind: &str) -> HandSource {
    let living = |t: Target| match t {
        Target::Player
        | Target::Guest(_)
        | Target::Mob(_)
        | Target::Stand(_)
        | Target::Dragon(_) => true,
        Target::Other(_) => !matches!(
            kind,
            "minecraft:marker"
                | "minecraft:block_display"
                | "minecraft:item_display"
                | "minecraft:text_display"
                | "minecraft:interaction"
        ),
        _ => false,
    };
    match source {
        None => HandSource::Holder(Target::Player),
        Some(None) => HandSource::NoEntity,
        Some(Some(t)) if living(t) => HandSource::Holder(t),
        Some(Some(t)) => HandSource::CannotHold(t),
    }
}

impl Game<'_> {
    // ---- /forceload ----------------------------------------------------------

    pub(super) fn column_arg(&self, ctx: &Context, name: &str) -> R<[i32; 2]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos2d.incomplete", &[]))?;
        let p = self.resolve_coords(&c);
        Ok([p[0].floor() as i32, p[2].floor() as i32])
    }
    /// `ForceLoadCommand.changeForceLoad`: update and load each newly forced column in order.
    pub(super) fn cmd_forceload(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::forceload::forceload(self, id, ctx)
    }

    // ---- /fillbiome ----------------------------------------------------------------

    pub(super) fn cmd_fillbiome(&mut self, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::fillbiome::fillbiome(self, ctx)
    }

    /// The tail of a biome change: the columns in `touched` are saved and re-meshed, and the
    /// players who see `columns` are sent their new biomes (`ChunkMap.resendBiomesForChunks`), in
    /// the order the columns were gathered (z, then x). Shared by `/fillbiome` and
    /// `level_set_biome`.
    pub(in crate::game) fn commit_biome_columns(
        &mut self,
        touched: &std::collections::HashSet<(i32, i32)>,
        columns: &[(i32, i32)],
    ) {
        for key in touched {
            self.world.modified.insert(*key);
            for sy in 0..self.world.layout().sections() as i32 {
                self.world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.resend_biomes(&self.world, columns);
        }
    }
    /// Set the biome cell holding `pos` (a 4x4x4 cell, `QuartPos`) to the biome named `biome`
    /// (`minecraft:plains`), as `/fillbiome` does for one cell: the chunk is saved, re-meshed and
    /// resent. False for an unknown biome, an unloaded or out-of-height position, or a cell that
    /// already has it.
    /// Tentative reads see the new cells inside a transaction. Abort restores the cells
    /// without publishing. Saving, remeshing and resending wait for the outermost commit,
    /// once per column. Outside a transaction they run immediately.
    pub(in crate::game) fn set_biome_at(&mut self, pos: [i32; 3], biome: &str) -> bool {
        let Some(target) = world_biomes!(self)
            .list
            .iter()
            .position(|b| b.name == biome)
        else {
            return false;
        };
        let key = (pos[0] >> 4, pos[2] >> 4);
        let Some(chunk) = self.world.chunks.get(&key) else {
            return false;
        };
        if !self.world.shape.contains_y(pos[1])
            || usize::from(chunk.biome(pos[0], pos[1], pos[2])) == target
        {
            return false;
        }
        let old = chunk.biome(pos[0], pos[1], pos[2]);
        self.journal_biome(pos, old);
        let qy =
            ((pos[1] - self.world.bottom_y()) >> 2).clamp(0, self.world.layout().height() / 4 - 1);
        self.world.chunks.mutate_chunk(key, |chunk| {
            let chunk = std::sync::Arc::make_mut(chunk);
            chunk.set_biome_cell((pos[0] & 15) >> 2, qy, (pos[2] & 15) >> 2, target as u16);
        });
        self.after_commit(super::super::transactions::After::Biome(key));
        true
    }

    // ---- /worldborder -----------------------------------------------------------------------

    /// Every tick: the size change, the wall the player cannot pass and
    /// damage outside the border.
    /// `BorderChangeListener.DelegateBorderChangeListener`'s packets: what
    /// changed on the border since the guests were last told of it.
    pub(in crate::game) fn lan_sync_border(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let now = horizon_client::lan::border::Sent::of(&self.border);
        if let Some(sent) = host.border_sent.replace(now) {
            for packet in sent.changes(&self.border) {
                // PlayerList$1: only the players of the border's own level.
                host.hub.broadcast_level(&packet);
            }
        }
    }
    pub(in crate::game) fn tick_worldborder(&mut self) {
        let remaining = self.border.remaining_ticks();
        self.border.tick();
        if remaining > 0
            && self.border.remaining_ticks() == 0
            && (self.serving.is_some() || self.options.dedicated.is_some())
        {
            self.native_world_border_bounds_change_finish();
        }
        self.player_border();
        self.border_damage();
    }
    /// The player's movement collides with the border as it is now
    /// (`CollisionGetter.borderCollision`): from inside, its walls stop it.
    pub(in crate::game) fn player_border(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.border = Some(self.border.geometry().clone());
        }
    }
    /// `LivingEntity.baseTick`: a player outside the border, further than
    /// its safe zone, is hurt by the blocks beyond it. The game's player, or
    /// a guest's in their turn (`as_guest`).
    pub(in crate::game) fn border_damage(&mut self) {
        let Some((px, pz, half)) = self.player.as_ref().map(|p| {
            (
                p.position.x,
                p.position.z,
                f64::from(p.dimensions.width) / 2.0,
            )
        }) else {
            return;
        };
        let inside = self.border.contains(px - half, pz - half)
            && self.border.contains(px + half, pz + half);
        if inside {
            return;
        }
        let dist = self.border.distance(px, pz) + self.border.safe_zone;
        if dist < 0.0 && self.border.damage_per_block > 0.0 && self.ticks.is_multiple_of(10) {
            let amount = (-dist * self.border.damage_per_block).floor().max(1.0) as f32;
            self.hurt_player_by("minecraft:outside_border", amount);
        }
    }
    pub(super) fn cmd_worldborder(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::worldborder::worldborder(self, id, ctx)
    }

    // ---- /loot ---------------------------------------------------------------------------------

    /// `LootCommand`'s tool for `mine` and `fish`: the item the command gives, the source's hand for `mainhand`
    /// or `offhand` (`getSourceHandItem`), and none (`ItemStack.EMPTY`) when it names neither.
    fn tool_stack(&mut self, ctx: &Context) -> R<Option<ItemStack>> {
        if let Some(Arg::Item(p)) = ctx.get("tool") {
            let (damage, extra) = match p
                .components
                .as_deref()
                .map(horizon_client::item_data::command_components)
            {
                Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
                Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
                None => (0, None),
            };
            return Ok(Some(ItemStack {
                item: p.id.as_str().into(),
                count: 1,
                damage,
                extra,
            }));
        }
        let slot = if ctx.has("offhand") {
            "weapon.offhand"
        } else if ctx.has("mainhand") {
            "weapon.mainhand"
        } else {
            return Ok(None);
        };
        let source = self.command_source.map(|s| s.entity);
        let kind = match source {
            Some(Some(t @ Target::Other(_))) => self.target_type(t),
            _ => String::new(),
        };
        let entity = match hand_source(source, &kind) {
            HandSource::Holder(t) => t,
            HandSource::NoEntity => return Err(err("permissions.requires.entity", &[])),
            HandSource::CannotHold(t) => {
                let name = self.target_name(t).plain();
                return Err(err("commands.drop.no_held_items", &[&name]));
            }
        };
        Ok(self
            .slots_named(&Owner::Entity(entity), slot)
            .first()
            .and_then(|s| self.slot_get(s)))
    }
    /// `LootCommand.dropChestLoot`: resolve a named table from the world's
    /// reloadable loot registry. `dropBlockLoot` keeps the mined block's components.
    pub(super) fn cmd_loot(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let keep_empty = ctx.literals.iter().any(|literal| literal == "replace");
        // The drops.
        let (drops, table): (Vec<ItemStack>, Option<String>) = match id {
            "loot.loot" | "loot.fish" => {
                let Some(Arg::Resource(table)) = ctx.get("loot_table") else {
                    return Ok(0);
                };
                // An inline table (ResourceOrIdArgument) keeps its random sequence.
                if table.starts_with('{') {
                    let (tag, _) = horizon_core::nbt::parse_snbt(table)
                        .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
                    let json = horizon_client::scoreboard::tag_to_json(&tag);
                    // LootCommand's omitted fishing tool is EMPTY, not mainhand.
                    let held = if id == "loot.fish"
                        && (ctx.get("tool").is_some() || ctx.has("mainhand") || ctx.has("offhand"))
                    {
                        self.tool_stack(ctx)?
                    } else {
                        None
                    };
                    let drops = match json["random_sequence"].as_str() {
                        Some(sequence) => {
                            let random = self.rand_sequences.get(sequence, self.level.seed);
                            world_items!(self).inline_loot_table_for_command(
                                &json,
                                held.as_ref(),
                                random,
                                keep_empty,
                            )
                        }
                        None => world_items!(self).inline_loot_table_for_command(
                            &json,
                            held.as_ref(),
                            &mut self.world.random,
                            keep_empty,
                        ),
                    };
                    (drops, None)
                } else {
                    if !world_items!(self).has_loot_table(table) {
                        return Err(err("argument.resource_or_id.failed_to_parse", &[table]));
                    }
                    let held = if id == "loot.fish" {
                        self.tool_stack(ctx)?
                    } else {
                        None
                    };
                    let items = world_items!(self);
                    let drops = match items.loot_sequence(table) {
                        Some(seq) => {
                            let random = self.rand_sequences.get(&seq, self.level.seed);
                            items.loot_table_for_command(
                                table,
                                false,
                                held.as_ref(),
                                random,
                                keep_empty,
                            )
                        }
                        None => items.loot_table_for_command(
                            table,
                            false,
                            held.as_ref(),
                            &mut self.world.random,
                            keep_empty,
                        ),
                    };
                    (drops, None)
                }
            }
            "loot.kill" => {
                let targets = self.entities(ctx, "target", false)?;
                let Some(t) = targets.first().copied() else {
                    return Ok(0);
                };
                let ty = self.target_type(t);
                // `EntityType.getDefaultLootTable`, in the type's namespace.
                let (namespace, path) = ty.split_once(':').unwrap_or(("minecraft", ty.as_str()));
                let table = format!("{namespace}:entities/{path}");
                if t == Target::Player || !world_items!(self).has_loot_table(&table) {
                    return Err(err(
                        "commands.drop.no_loot_table.entity",
                        &[&self.target_name(t).plain()],
                    ));
                }
                // LootCommand.dropKillLoot: the source's entity is the killer.
                // A connected player is the last damage player, and its own
                // stack counts; it is in place only to read that stack.
                let guest = match self.command_source.map(|s| s.entity) {
                    Some(Some(Target::Guest(conn))) => self.for_player(Target::Guest(conn), |g| {
                        g.slot_get(&super::items::Slot::Inv(g.selected))
                    }),
                    _ => None,
                };
                let (by_player, held) = match guest {
                    Some(held) => (true, held),
                    None => (
                        matches!(
                            self.command_source.map(|s| s.entity),
                            None | Some(Some(Target::Player))
                        ),
                        self.slot_get(&super::items::Slot::Inv(self.selected)),
                    ),
                };
                let drops = self.roll_kill_loot(crate::game::loot_rolls::KillRoll {
                    kind: &ty,
                    table: &table,
                    target: t,
                    killer: self
                        .command_source
                        .map(|s| s.entity)
                        .unwrap_or(Some(Target::Player)),
                    by_player,
                    held: held.as_ref(),
                    keep_empty,
                });
                (drops, Some(table))
            }
            _ => {
                // loot mine
                let p = self.block_pos_arg(ctx, "pos")?;
                // getSourceHandItem runs before dropBlockLoot looks for the block's table.
                let tool = self.tool_stack(ctx)?;
                let state = self.world.block(p[0], p[1], p[2]);
                let name = self.data_block_name(p);
                let Some(table) =
                    world_items!(self).block_loot_table(self.pack.assets.registry, state)
                else {
                    return Err(err("commands.drop.no_loot_table.block", &[&name]));
                };
                let mut pending = crate::game::spill::Pending::default();
                let key = (p[0], p[1], p[2]);
                let has_block_entity = if let Some(entity) = self.world.block_entities.get(&key) {
                    pending.record(
                        p,
                        entity,
                        &name,
                        self.world.block_names.get(&key).cloned(),
                        self.world.block_lock(key),
                    );
                    true
                } else if let Some(saved) = self.block_entity_nbt(p) {
                    pending.record_saved(p, &saved);
                    true
                } else {
                    false
                };
                // LootCommand: the source's entity is `this`, if it has one.
                let entity = match self.command_source.map(|s| s.entity) {
                    None | Some(Some(Target::Player)) => Some(self.native_clicker().1 as u32),
                    Some(Some(Target::Guest(conn))) => self
                        .player_entity_id_by_connection(Some(conn))
                        .map(|id| id as u32),
                    Some(Some(target)) => crate::game::native_entities::target_id(target),
                    Some(None) => None,
                };
                // The hook may borrow the live level mutably. Keep a read-only
                // view of the location for this roll rather than borrow Game.
                let mut chunks = horizon_client::chunk_map::ChunkMap::default();
                for (&key, chunk) in self.world.chunks.iter() {
                    chunks.insert(key, chunk.clone());
                }
                let items = self
                    .server_items
                    .clone()
                    .unwrap_or_else(|| std::sync::Arc::new(world_items!(self).clone()));
                let registry = self.pack.assets.registry;
                let location = |condition: &serde_json::Value| {
                    loot_location_block(condition, p, &chunks, registry, &items)
                };
                let roll = horizon_client::items::BlockLootRoll {
                    held: tool.as_ref(),
                    keep_empty,
                    location: Some(&location),
                    breaker: entity.is_some(),
                    ..Default::default()
                };
                let mut drops = self.roll_block_loot(p, state, entity, roll);
                if has_block_entity {
                    for drop in &mut drops {
                        pending.attach(p, drop);
                    }
                }
                (items.split_stacks(drops), Some(table))
            }
        };
        let drops: Vec<ItemStack> = drops
            .into_iter()
            .filter(|d| keep_empty || (d.count > 0 && &*d.item != "minecraft:air"))
            .collect();
        // Where they go.
        let mut used: Vec<ItemStack> = Vec::new();
        let literals = &ctx.literals;
        if literals.iter().any(|l| l == "give") {
            for d in &drops {
                for &t in &self.entities(ctx, "players", true)? {
                    // LootCommand.playerGive gives to every target player: a
                    // guest gets it with their state in place, as with /give.
                    // `Inventory.add` keeps what fits and throws nothing out. It is true
                    // only when all of it went in, or always for a player with infinite
                    // materials (creative).
                    let added = self.for_player(t, |g| {
                        g.add_stack_to_inventory(d.clone()).is_none() || g.creative
                    });
                    if added == Some(true) {
                        used.push(d.clone());
                    }
                }
            }
        } else if literals.iter().any(|l| l == "spawn") {
            let at = self
                .position_arg(ctx, "targetPos")
                .unwrap_or_else(|| self.source_position());
            for d in &drops {
                let r = &mut self.world.random;
                let v = [
                    r.next_double() * 0.2 - 0.1,
                    0.2,
                    r.next_double() * 0.2 - 0.1,
                ];
                self.spawn_item(d.clone(), at, v, 10);
                used.push(d.clone());
            }
        } else if literals.iter().any(|l| l == "insert") {
            let p = self.block_pos_arg(ctx, "targetPos")?;
            let owner = Owner::Block(p);
            let slots = self.slots_named(&owner, "container.*");
            if slots.is_empty() {
                return Err(err(
                    "commands.item.target.not_a_container",
                    &[&p[0], &p[1], &p[2]],
                ));
            }
            for d in &drops {
                let mut left = d.clone();
                let mut placed = false;
                for s in &slots {
                    if !self.slot_accepts(s, Some(&left)) {
                        continue;
                    }
                    match self.slot_get(s) {
                        None => {
                            self.slot_set(s, Some(left.clone()));
                            placed = true;
                            left.count = 0;
                            break;
                        }
                        Some(mut cur)
                            if cur.item == left.item
                                && cur.damage == left.damage
                                && cur.extra == left.extra =>
                        {
                            let max = self.items().get(&cur.item).map_or(64, |i| i.max_stack);
                            let take = left.count.min(max.saturating_sub(cur.count));
                            if take > 0 {
                                cur.count += take;
                                left.count -= take;
                                self.slot_set(s, Some(cur));
                                placed = true;
                            }
                            if left.count == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if placed {
                    used.push(d.clone());
                }
            }
        } else {
            // replace entity|block <slot> [count]
            let block = literals.iter().any(|l| l == "block");
            let owners: Vec<Owner> = if block {
                vec![Owner::Block(self.block_pos_arg(ctx, "targetPos")?)]
            } else {
                self.entities(ctx, "entities", false)?
                    .into_iter()
                    .map(Owner::Entity)
                    .collect()
            };
            let Some(Arg::Str(slot_name)) = ctx.get("slot").cloned() else {
                return Ok(0);
            };
            let count = match ctx.get("count") {
                Some(Arg::Int(c)) => *c as usize,
                _ => drops.len(),
            };
            for o in owners {
                // `prefix.N` continues as `prefix.N+1`, ...
                let (prefix, start) = match slot_name.rsplit_once('.') {
                    Some((p, n)) if n.parse::<usize>().is_ok() => {
                        (Some(p.to_owned()), n.parse::<usize>().unwrap_or(0))
                    }
                    _ => (None, 0),
                };
                let first = self.slots_named(&o, &slot_name);
                if first.is_empty() {
                    if let Owner::Block(p) = &o {
                        return Err(err(
                            "commands.item.target.not_a_container",
                            &[&p[0], &p[1], &p[2]],
                        ));
                    }
                    return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
                }
                for i in 0..count {
                    let slot = match &prefix {
                        Some(p) => self
                            .slots_named(&o, &format!("{p}.{}", start + i))
                            .first()
                            .cloned(),
                        None => first.first().cloned().filter(|_| i == 0),
                    };
                    let Some(slot) = slot else { continue };
                    let item = loot_replace_slot_item(&drops, i);
                    if self.slot_accepts(&slot, item.as_ref())
                        && self.slot_set_by_command(&slot, item.clone())
                    {
                        used.push(item.unwrap_or(ItemStack {
                            item: "minecraft:air".into(),
                            count: 0,
                            damage: 0,
                            extra: None,
                        }));
                    }
                }
            }
        }
        let mut tracker = Tracker::default();
        for u in &used {
            // LootCommand's feedback names each drop by `getDisplayName`.
            let name = self.stack_display_name(u);
            // CommandResponseTracker.track(drop): one per stack, so the
            // command returns how many stacks it used, not how many items.
            tracker.track(&holder(name), 1);
        }
        let counts: Vec<u32> = used.iter().map(|u| u.count).collect();
        let (message, total) = tracker.message(
            false,
            None,
            |h, _| {
                let n = counts.iter().copied().find(|c| *c > 0).unwrap_or(0);
                match &table {
                    Some(t) => Text::translate(
                        "commands.drop.success.single_with_table",
                        &[num(n), h.text.clone(), Text::literal(t.clone())],
                    ),
                    None => {
                        Text::translate("commands.drop.success.single", &[num(n), h.text.clone()])
                    }
                }
            },
            |c, _| match &table {
                Some(t) => Text::translate(
                    "commands.drop.success.multiple_with_table",
                    &[num(c), Text::literal(t.clone())],
                ),
                None => Text::translate("commands.drop.success.multiple", &[num(c)]),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
}

fn loot_replace_slot_item(drops: &[ItemStack], index: usize) -> Option<ItemStack> {
    drops
        .get(index)
        .filter(|drop| drop.count > 0 && &*drop.item != "minecraft:air")
        .cloned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn loot_replace_normalises_air_and_zero_positions_while_preserving_later_items() {
        let drops = [
            super::ItemStack::new("minecraft:air", 1),
            super::ItemStack::new("minecraft:iron_ingot", 0),
            super::ItemStack::new("minecraft:diamond", 2),
        ];
        assert_eq!(super::loot_replace_slot_item(&drops, 0), None);
        assert_eq!(super::loot_replace_slot_item(&drops, 1), None);
        assert_eq!(
            super::loot_replace_slot_item(&drops, 2),
            Some(super::ItemStack::new("minecraft:diamond", 2))
        );
        assert_eq!(super::loot_replace_slot_item(&drops, 3), None);
    }
    /// `LootCommand.getSourceHandItem`: a living source's hand; `permissions.requires.entity` for a source
    /// without an entity; `commands.drop.no_held_items` for any other entity. The player in place's own command
    /// (no source) reads its own hand.
    #[test]
    fn only_a_living_source_has_a_hand_to_mine_or_fish_with() {
        use super::{HandSource, Target, hand_source};
        assert_eq!(hand_source(None, ""), HandSource::Holder(Target::Player));
        assert_eq!(hand_source(Some(None), ""), HandSource::NoEntity);
        for living in [
            Target::Player,
            Target::Guest(3),
            Target::Mob(4),
            Target::Stand(5),
            Target::Dragon(6),
        ] {
            assert_eq!(
                hand_source(Some(Some(living)), ""),
                HandSource::Holder(living)
            );
        }
        for thing in [
            Target::Item(7),
            Target::Frame(8),
            Target::Cart(9),
            Target::Boat(10),
            Target::Projectile(11),
            Target::Cushion(12),
        ] {
            assert_eq!(
                hand_source(Some(Some(thing)), ""),
                HandSource::CannotHold(thing)
            );
        }
        let other = Target::Other(13);
        for kind in [
            "minecraft:marker",
            "minecraft:text_display",
            "minecraft:interaction",
        ] {
            assert_eq!(
                hand_source(Some(Some(other)), kind),
                HandSource::CannotHold(other)
            );
        }
        // A type Horizon doesn't simulate and that isn't known to be lifeless keeps its hand read.
        assert_eq!(
            hand_source(Some(Some(other)), "example:golem"),
            HandSource::Holder(other)
        );
    }
}

#[cfg(test)]
mod loot_mine_component_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, containers::BlockEntity, save};
    use horizon_core::nbt::{Compound, Tag};
    use std::sync::Arc;

    fn slot(game: &Game<'_>, index: usize) -> ItemStack {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(12, 200, 4)) else {
            panic!("actual command-created chest required");
        };
        items[index]
            .clone()
            .expect("actual inventory slot must be occupied")
    }

    fn decorations(stack: &ItemStack) -> Option<Tag> {
        save::item_to_nbt(stack)
            .get("components")?
            .get("minecraft:pot_decorations")
            .cloned()
    }

    #[test]
    #[ignore = "daily"]
    fn loot_mine_copies_actual_empty_and_right_pot_components_into_the_chest() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(game.run_command(
            "setblock 12 200 4 chest[facing=north,type=single,waterlogged=false] strict"
        ));
        assert!(
            game.run_command("item replace block 12 200 4 container.26 with minecraft:diamond 7")
        );
        assert!(game.run_command("item replace block 12 200 4 container.2 with minecraft:decorated_pot[minecraft:pot_decorations={}] 1"));
        let empty = Tag::Compound(Compound::new());
        assert_eq!(
            (&*slot(&game, 26).item, slot(&game, 26).count),
            ("minecraft:diamond", 7)
        );
        assert_eq!(
            decorations(&slot(&game, 2)),
            Some(empty.clone()),
            "direct-item parser/encoder positive control"
        );
        assert!(
            slot(&game, 2)
                .extra
                .as_ref()
                .unwrap()
                .pot_decorations
                .is_some()
        );
        let start = game.world.game_time;
        let right = "minecraft:angler_pottery_sherd";
        let mut failures = Vec::new();
        for (x, nbt, decorated) in [
            (4, "", false),
            (
                7,
                r#"{sherds:{right:{id:"minecraft:angler_pottery_sherd"}}}"#,
                true,
            ),
        ] {
            assert!(game.run_command(&format!(
                "setblock {x} 200 4 decorated_pot[facing=north,cracked=false,waterlogged=false]{nbt} strict"
            )));
            let pos = [x, 200, 4];
            let state = game.world.block(x, 200, 4);
            let actual = game.pack.assets.registry.state(state).unwrap();
            assert_eq!(&*actual.name, "minecraft:decorated_pot");
            assert_eq!(
                actual.properties.get("cracked").map(String::as_str),
                Some("false")
            );
            assert_eq!(
                game.world.block(x, 199, 4),
                game.pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap()
            );
            let before = game.pot_data(pos).expect("actual source block entity read");
            assert_eq!(
                before.get("id"),
                Some(&Tag::String("minecraft:decorated_pot".into()))
            );
            for (key, value) in [("x", x), ("y", 200), ("z", 4)] {
                assert_eq!(before.get(key), Some(&Tag::Int(value)));
            }
            if decorated {
                assert_eq!(
                    before
                        .get("sherds")
                        .and_then(|t| t.get("right"))
                        .and_then(|t| t.get("id")),
                    Some(&Tag::String(right.into())),
                    "actual admitted source sherd"
                );
            } else {
                assert!(before.get("sherds").is_none());
            }
            assert!(before.get("item").is_none() && before.get("LootTable").is_none());
            assert!(
                game.run_command("item replace block 12 200 4 container.0 with minecraft:stone 3")
            );
            let initial = slot(&game, 0);
            assert_eq!((&*initial.item, initial.count), ("minecraft:stone", 3));
            assert!(game.run_command(&format!(
                "loot replace block 12 200 4 container.0 1 mine {x} 200 4"
            )));
            let out = slot(&game, 0);
            assert_eq!(
                (&*out.item, out.count),
                ("minecraft:decorated_pot", 1),
                "inventory is the count authority"
            );
            let expected = if decorated {
                Tag::compound([("right", Tag::compound([("id", Tag::String(right.into()))]))])
            } else {
                empty.clone()
            };
            if decorations(&out) != Some(expected) {
                failures.push(format!(
                    "source {x}: actual saved output pot_decorations missing/wrong"
                ));
            }
            let modeled = out.extra.as_ref().and_then(|d| d.pot_decorations.as_ref());
            if modeled.is_none_or(|sides| {
                sides.iter().enumerate().any(|(i, item)| {
                    if decorated && i == 2 {
                        item.as_ref()
                            .is_none_or(|s| &*s.item != right || s.count != 1)
                    } else {
                        item.is_some()
                    }
                })
            }) {
                failures.push(format!(
                    "source {x}: actual output component state missing/wrong"
                ));
            }
            assert_eq!(
                game.pot_data(pos),
                Some(before),
                "mine leaves the actual source unchanged"
            );
            assert_eq!(game.world.block(x, 200, 4), state);
            assert_eq!(
                (&*slot(&game, 26).item, slot(&game, 26).count),
                ("minecraft:diamond", 7)
            );
            assert_eq!(decorations(&slot(&game, 2)), Some(empty.clone()));
            assert_eq!(game.world.game_time, start);
        }
        assert!(game.run_command("setblock 10 200 4 dirt strict"));
        assert!(game.run_command("loot replace block 12 200 4 container.0 1 mine 10 200 4"));
        let plain = slot(&game, 0);
        assert_eq!((&*plain.item, plain.count), ("minecraft:dirt", 1));
        assert!(decorations(&plain).is_none());
        assert_eq!(game.world.game_time, start);
        assert!(
            failures.is_empty(),
            "actual loot-mine component-copy failures: {failures:#?}"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn loot_mine_uses_the_actual_stand_or_server_executor_presence() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));

        game.command_source = Some(crate::game::commands::execute::Source {
            silent: false,
            ..game.server_source()
        });
        assert!(game.source_entity().is_none());
        assert!(game.run_command("execute unless entity @s"));
        assert!(game.run_command("setblock 12 200 4 chest strict"));
        assert!(game.run_command("setblock 2 200 4 snow[layers=1] strict"));
        assert!(game.run_command("setblock 4 200 4 dirt strict"));
        assert!(game.run_command("item replace block 12 200 4 container.26 with diamond 7"));
        assert!(game.run_command("item replace block 12 200 4 container.2 with diamond_shovel"));
        assert_eq!(&*slot(&game, 2).item, "minecraft:diamond_shovel");
        assert!(game.run_command("execute if items block 12 200 4 container.2 #minecraft:shovels"));
        assert!(game.run_command(
            "summon armor_stand 8.5 200 8.5 {UUID:[I;60,0,0,1],Health:20f,NoGravity:1b}"
        ));
        let stand = "0000003c-0000-0000-0000-000000000001";
        assert!(game.run_command(&format!("execute as {stand} if entity @s[type=armor_stand,nbt={{UUID:[I;60,0,0,1],Health:20f,NoGravity:1b,Pos:[8.5d,200d,8.5d]}}]")));
        let id = game
            .armor_stands
            .iter()
            .find(|s| game.persist.uuids.get(&s.id) == Some(&[60, 0, 0, 1]))
            .unwrap()
            .id;
        let before_stand = game.command_target_nbt(Target::Stand(id)).unwrap();
        assert_eq!(
            before_stand.get("UUID"),
            Some(&Tag::IntArray(vec![60, 0, 0, 1]))
        );
        let clock = game.world.game_time;
        // Unconditional dirt proves both sources really execute the same path.
        for (as_stand, x, expected) in [
            (false, 4, "minecraft:dirt"),
            (true, 4, "minecraft:dirt"),
            (false, 2, "minecraft:emerald"),
            (true, 2, "minecraft:snowball"),
        ] {
            assert!(game.source_entity().is_none());
            assert!(game.run_command("item replace block 12 200 4 container.0 with emerald 4"));
            let before = slot(&game, 0);
            let state = game.world.block(x, 200, 4);
            let prefix = if as_stand {
                format!("execute as {stand} run ")
            } else {
                String::new()
            };
            assert!(game.run_command(&format!(
                "{prefix}loot replace block 12 200 4 container.0 mine {x} 200 4 diamond_shovel"
            )));
            let actual = slot(&game, 0);
            assert_eq!(
                &*actual.item, expected,
                "executor stand={as_stand}, block x={x}"
            );
            if !as_stand && x == 2 {
                assert_eq!(actual, before);
            } else {
                assert_eq!(actual.count, 1);
            }
            assert_eq!(
                (&*slot(&game, 26).item, slot(&game, 26).count),
                ("minecraft:diamond", 7)
            );
            assert_eq!(&*slot(&game, 2).item, "minecraft:diamond_shovel");
            assert_eq!(game.world.block(x, 200, 4), state);
            assert_eq!(
                game.command_target_nbt(Target::Stand(id)).unwrap(),
                before_stand
            );
            assert_eq!(game.world.game_time, clock);
        }
    }
}

#[cfg(test)]
mod loot_position_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

    #[test]
    #[ignore = "daily"]
    fn actual_loot_replacement_retains_the_emitted_zero_slot() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();

        game.set_allow_commands(true);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 4 200 4 chest"));
        let slot = |game: &mut Game<'_>, i| {
            game.slot_get(&crate::game::commands::items::Slot::Block([4, 200, 4], i))
        };
        let reset = |game: &mut Game<'_>| {
            assert!(game.run_command("item replace block 4 200 4 container.0 with diamond 3"));
            assert!(game.run_command("item replace block 4 200 4 container.1 with emerald 4"));
            assert!(game.run_command("item replace block 4 200 4 container.2 with gold_ingot 5"));
            assert_eq!(
                slot(game, 0).map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 3))
            );
            assert_eq!(
                slot(game, 1).map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:emerald".into(), 4))
            );
        };
        let table = |counts: &[i32]| {
            serde_json::json!({"pools":counts.iter().map(|count| serde_json::json!({
            "rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot",
                "modifier":{"type":"minecraft:set_count","count":count}}]
        })).collect::<Vec<_>>()})
            .to_string()
        };
        let time = game.world.game_time;
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 loot {}",
            table(&[1])
        )));
        assert_eq!(
            slot(&mut game, 0).map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:iron_ingot".into(), 1))
        );
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 loot {}",
            table(&[0, 1])
        )));
        assert!(
            slot(&mut game, 0).is_none(),
            "the emitted zero occupies and clears slot0"
        );
        assert_eq!(
            slot(&mut game, 1).map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:iron_ingot".into(), 1))
        );
        assert_eq!(slot(&mut game, 2).unwrap().count, 5);
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 2 loot {}",
            table(&[])
        )));
        assert!(slot(&mut game, 0).is_none());
        assert!(slot(&mut game, 1).is_none());
        assert_eq!(slot(&mut game, 2).unwrap().count, 5);
        assert_eq!(game.world.game_time, time);
    }
}

#[cfg(test)]
mod loot_kill_target_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, containers::BlockEntity};
    use std::sync::Arc;
    #[test]
    #[ignore = "daily"]
    fn loot_kill_uses_actual_sheep_state_without_killing_the_target() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h036-game"),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        let clock = game.world.game_time;
        let mut differences = Vec::new();
        for (n, color, sheared, wool) in [
            (1, 0, true, "minecraft:white_wool"),
            (2, 14, true, "minecraft:red_wool"),
            (3, 0, false, "minecraft:white_wool"),
            (4, 14, false, "minecraft:red_wool"),
        ] {
            let uuid = format!("00000024-0000-0000-0000-{n:012x}");
            assert!(game.run_command(&format!(
                "summon sheep 4.5 200 4.5 {{UUID:[I;36,0,0,{n}],Health:8f,NoAI:1b,NoGravity:1b,Age:0,Color:{color}b,Sheared:{}b,Motion:[0d,0d,0d]}}", i32::from(sheared)
            )));
            let mob = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([36, 0, 0, n]))
                .unwrap();
            assert_eq!(mob.kind.id, "minecraft:sheep");
            assert_eq!(mob.health, 8.0);
            assert!(mob.no_ai && mob.no_gravity);
            assert_eq!(mob.extra.color, color);
            assert_eq!(mob.extra.sheared, sheared);
            let before = (mob.id, mob.position, mob.velocity, mob.health);
            let x = 2 + n * 3;
            assert!(game.run_command(&format!(
                "setblock {x} 200 8 chest[facing=north,type=single,waterlogged=false] strict"
            )));
            let Some(BlockEntity::Chest(initial)) = game.world.block_entities.get(&(x, 200, 8))
            else {
                panic!("real chest required")
            };
            assert!(initial.iter().all(Option::is_none));
            assert!(game.run_command(&format!("loot insert {x} 200 8 kill {uuid}")));
            let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(x, 200, 8))
            else {
                panic!("real output chest required")
            };
            let output: Vec<_> = items.iter().flatten().collect();
            assert!(
                output
                    .iter()
                    .any(|s| s.item.as_ref() == "minecraft:mutton" && (1..=2).contains(&s.count)),
                "nonempty meat proves the command actually rolled"
            );
            let actual: u32 = output
                .iter()
                .filter(|s| s.item.as_ref() == wool)
                .map(|s| s.count)
                .sum();
            if actual != u32::from(!sheared) {
                differences.push((n, sheared, wool, actual));
            }
            let mob = game.mobs.iter().find(|m| m.id == before.0).unwrap();
            assert_eq!((mob.id, mob.position, mob.velocity, mob.health), before);
            assert_eq!(game.persist.uuid_of(mob.id), Some([36, 0, 0, n]));
            assert_eq!((mob.extra.color, mob.extra.sheared), (color, sheared));
        }
        assert_eq!(game.world.game_time, clock);
        assert!(
            differences.is_empty(),
            "actual command wool-state differences: {differences:?}"
        );
    }
}

#[cfg(test)]
mod loot_mine_location_tests {
    use super::*;
    use horizon_client::{chunk::Chunk, containers::BlockEntity};
    use std::sync::Arc;
    const TABLE: &str = r###"{"type":"minecraft:block","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald","modifier":{"type":"minecraft:set_count","count":2}}]},{"rolls":1,"condition":{"type":"minecraft:location_check","offsetX":1,"predicate":{"block":{"blocks":"minecraft:oak_log","state":{"axis":"x"}}}},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"###;
    #[test]
    #[ignore = "daily"]
    fn loot_mine_location_reads_actual_offset_block_state() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                    .join("h079-game"),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();

            game.world
                .chunks
                .insert((-1, -1), Arc::new(Chunk::empty(-1, -1)));
            game.world
                .chunks
                .insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
            game.world
                .chunks
                .insert((0, -1), Arc::new(Chunk::empty(0, -1)));
            for (x, z) in [(-1, -1), (-1, 0), (0, -1)] {
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(x, z))
                    .unwrap();
            }
            game.world_root = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join("h079-world");
            let pack = game.world_root.join("datapacks/h079");
            std::fs::create_dir_all(pack.join("data/minecraft/loot_table/blocks")).unwrap();
            std::fs::create_dir_all(pack.join("data/h079/function")).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"H079 fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/minecraft/loot_table/blocks/stone.json"),
                TABLE,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/h079/function/admit.mcfunction"),
                "scoreboard players set pack h079 42\n",
            )
            .unwrap();
            std::fs::write(
                game.world_root.join("datapacks.json"),
                r#"{"enabled":["vanilla","file/h079"],"disabled":[]}"#,
            )
            .unwrap();
            let stack = game.reload_datapacks(false, &[]).unwrap();
            game.world_tables = game
                .stage_world_tables(&stack, false, &game.options.resources.clone())
                .unwrap()
                .0;
            assert!(game.world_tables.is_some());
            game.ensure_datapacks();
            assert!(game.run_command("scoreboard objectives add h079 dummy"));
            game.run_command("function h079:admit");
            assert!(
                game.run_command("execute if score pack h079 matches 42"),
                "real pack function admission"
            );
            assert!(game.run_command("forceload add -16 -16 15 15"));
            assert!(game.run_command("fill -16 199 -16 15 199 15 stone"));
            let clock = game.world.game_time;
            let mut differences = Vec::new();
            for (label, x, z, counterpart, expected) in [
                ("correct", 4, 4, Some("oak_log[axis=x]"), 1),
                ("wrong-state", 8, 4, Some("oak_log[axis=z]"), 0),
                ("absent", 12, 4, None, 0),
                ("negative-center", -4, -4, Some("oak_log[axis=x]"), 1),
            ] {
                assert!(game.run_command(&format!("setblock {x} 200 {z} stone strict")));
                if let Some(other) = counterpart {
                    assert!(
                        game.run_command(&format!("setblock {} 200 {z} {other} strict", x + 1))
                    );
                }
                assert!(game.run_command(&format!(
                    "execute if block {} 200 {z} {}",
                    x + 1,
                    counterpart.unwrap_or("air")
                )));
                let before = (game.world.block(x, 200, z), game.world.block(x + 1, 200, z));
                assert!(game.run_command(&format!("setblock {x} 200 10 chest strict")));
                let Some(BlockEntity::Chest(initial)) =
                    game.world.block_entities.get(&(x, 200, 10))
                else {
                    panic!("real output chest")
                };
                assert!(initial.iter().all(Option::is_none));
                let prefix = if x < 0 {
                    "execute positioned -8.25 200.125 -8.75 run "
                } else {
                    ""
                };
                assert!(game.run_command(&format!(
                    "{prefix}loot insert {x} 200 10 mine {x} 200 {z} stick"
                )));
                let Some(BlockEntity::Chest(output)) = game.world.block_entities.get(&(x, 200, 10))
                else {
                    panic!("actual output")
                };
                assert!(
                    output
                        .iter()
                        .flatten()
                        .any(|s| s.item.as_ref() == "minecraft:emerald" && s.count == 2),
                    "pack override and unconditional roll are real"
                );
                let diamonds: u32 = output
                    .iter()
                    .flatten()
                    .filter(|s| s.item.as_ref() == "minecraft:diamond")
                    .map(|s| s.count)
                    .sum();
                if diamonds != expected {
                    differences.push((label, diamonds, expected));
                }
                assert_eq!(
                    (game.world.block(x, 200, z), game.world.block(x + 1, 200, z)),
                    before
                );
            }
            assert_eq!(game.world.game_time, clock);
            assert!(
                differences.is_empty(),
                "actual offset block predicate differences: {differences:?}"
            );
        });
    }
}

#[cfg(test)]
mod loot_location_shape_tests {
    use super::*;
    use horizon_client::{chunk::Chunk, chunk_map::ChunkMap};
    use horizon_world::registry::BlockRegistry;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn location_shape_uses_loaded_facts_and_rejects_whole_unsupported_conjunctions() {
        let registry = BlockRegistry::from_facts_json(
            &br#"{"defaults":{"minecraft:air":0,"minecraft:oak_log":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["log",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}]
        ]}"#[..],
        )
        .unwrap();
        let mut positive = Chunk::empty(0, 0);
        positive.set(5, 200, 4, 1);
        let mut negative = Chunk::empty(-1, -1);
        negative.set(13, 200, 12, 1);
        let mut chunks = ChunkMap::default();
        chunks.insert((0, 0), Arc::new(positive));
        chunks.insert((-1, -1), Arc::new(negative));
        let items = horizon_client::items::Items::for_tests(Default::default());
        let valid = json!({"type":"minecraft:location_check","offsetX":1,"predicate":{"block":{"blocks":"minecraft:oak_log","state":{"axis":"x"}}}});
        assert_eq!(
            loot_location_block(&valid, [4, 200, 4], &chunks, &registry, &items),
            Some(true)
        );
        assert_eq!(
            loot_location_block(&valid, [-4, 200, -4], &chunks, &registry, &items),
            Some(true)
        );
        assert_eq!(
            loot_location_block(&valid, [8, 200, 4], &chunks, &registry, &items),
            Some(false)
        );
        for (key, value) in [
            ("biomes", json!("minecraft:plains")),
            ("light", json!({"light":15})),
            ("dimension", json!("minecraft:overworld")),
        ] {
            let mut unsupported = valid.clone();
            unsupported["predicate"][key] = value;
            assert_eq!(
                loot_location_block(&unsupported, [4, 200, 4], &chunks, &registry, &items),
                None
            );
        }
        for value in [json!(1.5), json!(2147483648_i64), json!("1")] {
            let mut unsupported = valid.clone();
            unsupported["offsetX"] = value;
            assert_eq!(
                loot_location_block(&unsupported, [4, 200, 4], &chunks, &registry, &items),
                None
            );
        }
        let count = chunks.len();
        assert_eq!(
            loot_location_block(&valid, [32, 200, 32], &chunks, &registry, &items),
            Some(false),
            "an unloaded counterpart fails, as Level.isLoaded does"
        );
        assert_eq!(chunks.len(), count, "no chunk is loaded as a side effect");
    }
}

#[cfg(test)]
#[path = "loot_slot_tests.rs"]
mod loot_slot_tests;
