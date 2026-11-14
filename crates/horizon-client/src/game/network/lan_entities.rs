//! The entities beyond mobs and items that a server tracks to its clients
//! (`ChunkMap.TrackedEntity`, `ServerEntity`): boats, minecarts, armor
//! stands, item frames, paintings and the entities Horizon keeps without
//! simulating them, each as vanilla pairs it (`getAddEntityPacket` and the
//! synched data that differs from the defaults), and who rides what.
use super::Game;
use horizon_client::lan::{
    entity::{EntityState, Riding, States},
    playerdata::ints_uuid,
};
use horizon_protocol::entity_data::{Value, serializer};
use {horizon_core::nbt::Tag, horizon_world::registry::BlockStateId};

/// Allocate once: a native summon may need its id before the tracking tick.
pub(super) fn assign_entity_id(next: &mut u32, id: &mut u32) -> u32 {
    if *id == 0 {
        *next += 1;
        *id = *next;
    }
    *id
}

impl Game<'_> {
    /// `Entity.nextEntityId`: the id a new entity takes from the one counter.
    pub(super) fn take_entity_id(&mut self) -> u32 {
        self.next_entity += 1;
        self.next_entity
    }
    /// Whether the id counter is within [`ENTITY_ID_HEADROOM`] of the end: then a mod's spawn is
    /// refused and the tick spawns nothing of its own, named once in the log. Ids already handed
    /// out are never taken back, so a stale handle never names a later entity.
    pub(super) fn entity_ids_exhausted(&mut self) -> bool {
        let exhausted = self.next_entity >= u32::MAX - ENTITY_ID_HEADROOM;
        if exhausted && !self.native_lifecycle.ids_exhausted_told {
            self.native_lifecycle.ids_exhausted_told = true;
            log::error!(
                "entity ids exhausted: {} of {} handed out; no entity spawns until the world reopens",
                self.next_entity,
                u32::MAX
            );
        }
        exhausted
    }
}

/// The ids kept back below `u32::MAX`: far more than one tick hands out (a whole ender dragon
/// takes nine in a row) and the provisional ids a natural spawn pass counts down from the end.
pub(super) const ENTITY_ID_HEADROOM: u32 = 1 << 24;

fn add_state<'a>(
    out: &'a mut States,
    id: u32,
    kind: &str,
    position: [f64; 3],
    yaw: f32,
    on_ground: bool,
) -> &'a mut EntityState {
    let e = out.add(id as i32, super::lan::entity_uuid(id), kind);
    e.position = position;
    e.yaw = yaw;
    e.on_ground = on_ground;
    e
}

fn state(id: u32, kind: &str, position: [f64; 3], yaw: f32, on_ground: bool) -> EntityState {
    let mut e = EntityState::default();
    e.reset(id as i32, super::lan::entity_uuid(id), kind);
    e.position = position;
    e.yaw = yaw;
    e.on_ground = on_ground;
    e
}

/// `VehicleEntity`'s DATA_ID_HURT, DATA_ID_HURTDIR and DATA_ID_DAMAGE
/// (entries 8 to 10) when they differ from their defaults (0, 1, 0).
fn vehicle_meta(e: &mut EntityState, hurt_time: i32, hurt_dir: i32, damage: f32) {
    if hurt_time != 0 {
        e.meta.push((8, serializer::INT, Value::Int(hurt_time)));
    }
    if hurt_dir != 1 {
        e.meta.push((9, serializer::INT, Value::Int(hurt_dir)));
    }
    if damage != 0.0 {
        e.meta.push((10, serializer::FLOAT, Value::Float(damage)));
    }
    e.defaults.push((8, serializer::INT, Value::Int(0)));
    e.defaults.push((9, serializer::INT, Value::Int(1)));
    e.defaults.push((10, serializer::FLOAT, Value::Float(0.0)));
}

/// `AbstractMinecart`'s DATA_ID_CUSTOM_DISPLAY_BLOCK (entry 11, an optional
/// block state, empty by default) and DATA_ID_DISPLAY_OFFSET (entry 12, the
/// kind's `getDefaultDisplayOffset` by default).
fn cart_display_data(
    e: &mut EntityState,
    shown: Option<BlockStateId>,
    offset: Option<i32>,
    default_offset: i32,
) {
    e.defaults
        .push((11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)));
    e.defaults
        .push((12, serializer::INT, Value::Int(default_offset)));
    if let Some(s) = shown {
        e.meta
            .push((11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(s.0 as i32)));
    }
    if let Some(o) = offset.filter(|&o| o != default_offset) {
        e.meta.push((12, serializer::INT, Value::Int(o)));
    }
}

/// An armor stand's pose (`ArmorStandPose`), each part that differs from the
/// default, as DATA_HEAD_POSE to DATA_RIGHT_LEG_POSE (entries 16 to 21).
fn push_stand_poses(meta: &mut Vec<(u8, i32, Value)>, a: &horizon_client::entities::ArmorStand) {
    for (i, (rotation, default)) in a
        .pose
        .iter()
        .zip(horizon_client::entities::ArmorStand::DEFAULT_POSE)
        .enumerate()
    {
        if *rotation != default {
            meta.push((
                16 + i as u8,
                serializer::ROTATIONS,
                Value::Floats(rotation.to_vec()),
            ));
        }
    }
}

#[cfg(test)]
fn stand_poses(a: &horizon_client::entities::ArmorStand) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    push_stand_poses(&mut meta, a);
    meta
}

impl Game<'_> {
    /// The entities players started and stopped tracking since the last
    /// tick, told to the mods in order with the hub in `self.lan` and
    /// no player's state swapped in (`ServerEntity.addPairing` and
    /// `removePairing`).
    pub(super) fn run_native_tracking(&mut self) {
        let batches = match self.lan.as_mut() {
            Some(host) => std::mem::take(&mut host.hub.native_tracking),
            None => return,
        };
        if self.remote.is_some() || self.options.execution_role.is_client() {
            return;
        }
        for batch in batches {
            for e in batch.events {
                let (id, kind) = (e.entity as u32, &*e.tracked.kind);
                if e.start {
                    self.native_player_start_tracking(
                        batch.player,
                        &batch.name,
                        id,
                        e.tracked.uuid,
                        kind,
                    );
                } else {
                    self.native_player_stop_tracking(
                        batch.player,
                        &batch.name,
                        id,
                        e.tracked.uuid,
                        kind,
                    );
                }
            }
        }
    }

    /// Who rides what (`Entity.getPassengers`): the players in their vehicles and
    /// mobs on mobs.
    pub(super) fn lan_riding(&mut self, guests: &[(u32, i32)], riding: &mut Riding) {
        riding.clear();
        // Players arrive in boarding order; the first rider drives.
        for &(vehicle, guest) in guests {
            riding.add(vehicle as i32, guest);
        }
        for m in self.mobs.iter().filter(|m| !m.removed) {
            if let Some(vehicle) = m.extra.mount {
                riding.add(vehicle as i32, m.id as i32);
            }
        }
        for (&vehicle, pass) in &mut riding.passengers {
            // Players occupy the leading seats, as in `position_riders`.
            // `extra.rider` orders mob passengers, not a player's driver seat.
            if guests.iter().any(|&(id, _)| id as i32 == vehicle) {
                continue;
            }
            if let Some(driver) = self
                .mobs
                .iter()
                .find(|m| m.id == vehicle as u32)
                .and_then(|m| m.extra.rider)
                && let Some(pos) = pass.iter().position(|&p| p == driver as i32)
                && pos != 0
            {
                let d = pass.remove(pos);
                pass.insert(0, d);
            }
        }
    }

    /// Boats, minecarts, armor stands, item frames, paintings and the
    /// entities kept unsimulated, as guests track them.
    pub(super) fn lan_object_states(
        &mut self,
        out: &mut States,
        drain_minecart_steps: bool,
        after: u32,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let hub = &host.hub;
        let guests: Vec<(u128, i32)> = hub
            .guests
            .iter()
            .filter(|g| hub.here(g))
            .map(|g| (g.uuid, g.entity_id))
            .collect();
        for b in self.boats.iter().filter(|b| b.id > after) {
            let p = b.position;
            let leash = self.boat_leash_link(b, &guests);
            let e = add_state(
                out,
                b.id,
                &b.entity_type(),
                [p.x, p.y, p.z],
                b.yaw,
                b.on_ground,
            );
            // Leashable.setLeashedTo / dropLeash: who holds the boat's lead.
            e.leash = leash;
            e.velocity = [b.velocity.x, b.velocity.y, b.velocity.z];
            vehicle_meta(e, b.hurt_time, b.hurt_dir, b.damage);
            if b.fire_ticks > 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }

            // AbstractBoat.DATA_ID_PADDLE_LEFT and DATA_ID_PADDLE_RIGHT.
            // Omitted false values must reset clients that saw a true value,
            // including when the controlling passenger dismounts mid-stroke.
            for (k, paddling) in b.paddling.iter().enumerate() {
                e.defaults
                    .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(false)));
                if *paddling {
                    e.meta
                        .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            // AbstractBoat.DATA_ID_BUBBLE_TIME, back to 0 once the boat is off
            // the column.
            if b.bubble_time != 0 {
                e.meta
                    .push((13, serializer::INT, Value::Int(b.bubble_time)));
            }
            e.defaults.push((13, serializer::INT, Value::Int(0)));
        }
        let improvements = self.minecart_improvements();
        self.carts.for_each_mut(|c| {
            if c.id <= after {
                return;
            }
            let p = c.position;
            let shown = c.custom_display.as_ref().and_then(|t| {
                horizon_client::entity_nbt::state_from_nbt(self.pack.assets.registry, t)
            });
            let default = super::minecarts::default_display_offset(c.kind) as i32;
            let e = add_state(
                out,
                c.id,
                c.kind.id(),
                [p.x, p.y, p.z],
                c.yaw,
                c.on_ground(),
            );
            e.pitch = c.pitch;
            e.velocity = [c.velocity.x, c.velocity.y, c.velocity.z];
            if improvements {
                // Joining another dimension only takes a spawn snapshot; its
                // regular tracking tick still owns the unsent steps.
                e.minecart_steps = Some(if drain_minecart_steps {
                    std::mem::take(&mut c.lerp_steps)
                } else {
                    Vec::new()
                });
            }
            vehicle_meta(e, c.hurt_time, c.hurt_dir, c.damage);
            cart_display_data(e, shown, c.custom_offset, default);
            match c.kind {
                // MinecartFurnace.DATA_ID_FUEL.
                super::minecarts::CartKind::Furnace => {
                    e.defaults
                        .push((13, serializer::BOOLEAN, Value::Bool(false)));
                    if c.fuel > 0 {
                        e.meta.push((13, serializer::BOOLEAN, Value::Bool(true)));
                    }
                }
                // MinecartCommandBlock.DATA_ID_COMMAND_NAME.
                super::minecarts::CartKind::CommandBlock => {
                    // and DATA_ID_LAST_OUTPUT (a component, defaults empty).
                    let empty = Tag::String("".into());
                    e.defaults
                        .push((13, serializer::STRING, Value::String(String::new())));
                    e.defaults
                        .push((14, serializer::COMPONENT, Value::Text(empty)));
                    if !c.synced_command.is_empty() {
                        e.meta.push((
                            13,
                            serializer::STRING,
                            Value::String(c.synced_command.clone()),
                        ));
                    }
                    if !c.synced_output.is_empty() {
                        e.meta.push((
                            14,
                            serializer::COMPONENT,
                            Value::Text(Tag::String(c.synced_output.as_str().into())),
                        ));
                    }
                }
                _ => {}
            }
        });
        for a in self.armor_stands.iter().filter(|a| a.id > after) {
            let p = a.position;
            let attributes = self.stand_attributes(&a.live);
            let e = add_state(
                out,
                a.id,
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                a.yaw,
                a.on_ground,
            );
            e.head_yaw = a.yaw;
            e.velocity = [a.velocity.x, a.velocity.y, a.velocity.z];
            let mut flags = 0i8;
            if a.fire_ticks > 0 {
                flags |= 1;
            }
            if a.invisible {
                flags |= 0x20;
            }
            if flags != 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(flags)));
            }
            // LivingEntity.DATA_HEALTH_ID: 1 until the stand's max health is set.
            e.meta.push((9, serializer::FLOAT, Value::Float(a.health)));
            // ArmorStand.DATA_CLIENT_FLAGS: small 1, arms 4, no base plate 8, marker 16.
            let flags = i8::from(a.small)
                | i8::from(a.show_arms) << 2
                | i8::from(a.no_base_plate) << 3
                | i8::from(a.marker) << 4;
            if flags != 0 {
                e.meta.push((15, serializer::BYTE, Value::Byte(flags)));
            }
            push_stand_poses(&mut e.meta, a);
            // Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE: a hologram's text.
            if let Some(name) = &a.custom_name {
                e.meta.push((
                    2,
                    serializer::OPTIONAL_COMPONENT,
                    Value::OptText(Some(horizon_client::scoreboard::json_to_tag(
                        &name.component(),
                    ))),
                ));
                if a.name_visible {
                    e.meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            // A name taken away or hidden again (`/data`) is sent as none / false.
            e.defaults
                .push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
            e.defaults
                .push((3, serializer::BOOLEAN, Value::Bool(false)));
            // A pose put back to its default (`/data`) is sent as that default.
            e.defaults.push((15, serializer::BYTE, Value::Byte(0)));
            for (i, default) in horizon_client::entities::ArmorStand::DEFAULT_POSE
                .iter()
                .enumerate()
            {
                e.defaults.push((
                    16 + i as u8,
                    serializer::ROTATIONS,
                    Value::Floats(default.to_vec()),
                ));
            }
            // Every stored slot, by `EquipmentSlot` ordinal: the wire's slot numbers.
            for (slot, item) in a.equipment.iter().enumerate() {
                if item.is_some() {
                    let Some(host) = self.lan.as_mut() else {
                        return;
                    };
                    e.equipment.push((slot as u8, host.hub.stack(item)));
                }
            }
            e.attributes = attributes;
        }
        for f in self.item_frames.iter().filter(|f| f.id > after) {
            // ItemFrame.getAddEntityPacket: the block it hangs in and its direction.
            let e = add_state(out, f.id, f.kind(), f.pos.map(f64::from), 0.0, false);
            e.pitch = match f.facing {
                0 => 90.0,
                1 => -90.0,
                _ => 0.0,
            };
            e.yaw = match f.facing {
                2 => 180.0,
                4 => 90.0,
                5 => 270.0,
                _ => 0.0,
            };
            e.data = i32::from(f.facing);
            if f.invisible {
                e.meta.push((0, serializer::BYTE, Value::Byte(0x20)));
            }
            // HangingEntity.DATA_DIRECTION, south by default: a frame turned back to south
            // (`entity_set_hanging_direction`) clears the clients' value.
            if f.facing != 3 {
                e.meta
                    .push((8, serializer::DIRECTION, Value::Id(i32::from(f.facing))));
            }
            e.defaults.push((8, serializer::DIRECTION, Value::Id(3)));
            if f.item.is_some() {
                let Some(host) = self.lan.as_mut() else {
                    return;
                };
                e.meta.push((
                    9,
                    serializer::ITEM_STACK,
                    Value::Item(host.hub.stack(&f.item)),
                ));
            }
            if f.rotation != 0 {
                e.meta
                    .push((10, serializer::INT, Value::Int(i32::from(f.rotation))));
            }
        }
        for (p, &id) in self.knot_ids.iter().filter(|&(_, &id)| id > after) {
            // LeashFenceKnotEntity.getAddEntityPacket: the fence's block.
            add_state(
                out,
                id,
                "minecraft:leash_knot",
                p.map(f64::from),
                0.0,
                false,
            );
        }
        for p in self.paintings.iter().filter(|p| p.id > after) {
            let e = add_state(
                out,
                p.id,
                "minecraft:painting",
                p.pos.map(f64::from),
                0.0,
                false,
            );
            // Direction.get2DDataValue (south 0, west 1, north 2, east 3) to its
            // 3D data value and yaw.
            let facing = [3, 4, 2, 5][usize::from(p.facing & 3)];
            e.yaw = f32::from(p.facing & 3) * 90.0;
            e.data = facing;
            if facing != 3 {
                e.meta.push((8, serializer::DIRECTION, Value::Id(facing)));
            }
            e.defaults.push((8, serializer::DIRECTION, Value::Id(3)));
            // Painting.DATA_PAINTING_VARIANT_ID, a registry holder (id + 1),
            // set unless it is the registry's first variant (VariantUtils.getAny).
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            if let Some(id) = host
                .hub
                .registry_id("minecraft:painting_variant", &p.variant)
                && id != 0
            {
                e.meta
                    .push((9, serializer::PAINTING_VARIANT, Value::Id(id + 1)));
            }
        }
        for c in self.cushions.iter().filter(|c| c.id > after) {
            let p = c.position;
            let e = add_state(
                out,
                c.id,
                "minecraft:cushion",
                [p.x, p.y, p.z],
                c.yaw,
                false,
            );
            // Cushion.DATA_COLOR, set unless it is the default white.
            if c.color != 0 {
                e.meta
                    .push((8, serializer::DYE_COLOR, Value::Id(i32::from(c.color))));
            }
        }
        // DATA_ITEM, `ItemStack.EMPTY` by default.
        for o in self.ominous_spawners.iter().filter(|o| o.id > after) {
            let e = add_state(
                out,
                o.id,
                "minecraft:ominous_item_spawner",
                o.position,
                0.0,
                false,
            );
            e.meta.push((
                8,
                serializer::ITEM_STACK,
                Value::Item(self.lan.as_mut().and_then(|host| {
                    host.hub
                        .stack(&Some(o.item.clone()).filter(|i| &*i.item != "minecraft:air"))
                })),
            ));
        }
        self.lan_projectile_states(out, after);
        for o in self.others.iter().filter(|o| o.id > after) {
            let p = o.position;
            let e = add_state(out, o.id, &o.kind, [p.x, p.y, p.z], o.yaw, false);
            e.pitch = o.pitch;
            e.head_yaw = o.yaw;
            if let Some(tag) = self.persist.raw.get(&o.id)
                && let Some(item) = self.lan_display_data_kept(o.id, tag, e)
                && let Some((_, _, Value::Item(stack))) =
                    e.meta.iter_mut().find(|(index, _, _)| *index == 23)
            {
                *stack = self
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.stack(&Some(item)));
            }
        }
        // Only this level's displays: the others are kept for their own level's pass.
        KEPT_DISPLAYS.with(|kept| {
            let mut kept = kept.borrow_mut();
            let Some(kept) = kept.get_mut(&self.dim) else {
                return;
            };
            if kept.len() > self.others.len() + 64 {
                let ids: horizon_client::fast_hash::FxHashSet<u32> =
                    self.others.iter().map(|o| o.id).collect();
                kept.retain(|id, _| ids.contains(id));
            }
        });
    }
    /// [`Self::lan_display_data`] for the display `id`, built again only when
    /// its compound changed: a server's holograms and menus are text displays
    /// whose text, item and transformation were parsed again every tick.
    fn lan_display_data_kept(
        &self,
        id: u32,
        tag: &Tag,
        e: &mut EntityState,
    ) -> Option<horizon_client::items::ItemStack> {
        if !matches!(
            e.kind.as_str(),
            "minecraft:text_display" | "minecraft:block_display" | "minecraft:item_display"
        ) {
            return self.lan_display_data(tag, e);
        }
        // Block states are read through the pack's registry: a kept build is
        // only the same for the same one.
        let registry = std::ptr::from_ref(self.pack.assets.registry) as usize;
        KEPT_DISPLAYS.with(|kept| {
            let mut kept = kept.borrow_mut();
            let kept = kept.entry(self.dim).or_default();
            let same =
                |k: &KeptDisplay| k.registry == registry && k.kind == e.kind && k.tag == *tag;
            if let Some(k) = kept.get(&id).filter(|k| same(k)) {
                e.meta.clone_from(&k.meta);
                e.defaults.clone_from(&k.defaults);
                return k.item.clone();
            }
            let item = self.lan_display_data(tag, e);
            kept.insert(
                id,
                KeptDisplay {
                    registry,
                    kind: e.kind.clone(),
                    tag: tag.clone(),
                    meta: e.meta.clone(),
                    defaults: e.defaults.clone(),
                    item: item.clone(),
                },
            );
            item
        })
    }
}

/// A display's synched data as [`Game::lan_display_data`] built it from
/// `tag`, kept for the display's id on the thread that tracks entities, per
/// level: a server thread tracks each dimension's `others` in turn.
struct KeptDisplay {
    registry: usize,
    kind: String,
    tag: Tag,
    meta: Vec<(u8, i32, Value)>,
    defaults: Vec<(u8, i32, Value)>,
    /// An item display's item and the index in `meta` its stack goes in.
    item: Option<horizon_client::items::ItemStack>,
}
thread_local! {
    static KEPT_DISPLAYS: std::cell::RefCell<
        horizon_client::fast_hash::FxHashMap<super::Dim, horizon_client::fast_hash::FxHashMap<u32, KeptDisplay>>,
    > = Default::default();
}

impl Game<'_> {
    /// The synched data of a text, block or item display kept unsimulated,
    /// from its compound (`Display.readAdditionalSaveData`), with the
    /// defaults to send when a value goes back to one.
    fn lan_display_data(
        &self,
        tag: &Tag,
        e: &mut EntityState,
    ) -> Option<horizon_client::items::ItemStack> {
        use horizon_client::text_display::{display_data, text_display_data};
        let mut item_stack = None;
        let empty = Tag::Compound(Default::default());
        let (meta, defaults) = match e.kind.as_str() {
            "minecraft:text_display" => (
                text_display_data(tag, false),
                text_display_data(&empty, true),
            ),
            "minecraft:block_display" => {
                // DATA_BLOCK_STATE_ID, air by default.
                let state = |t: Option<BlockStateId>| {
                    (
                        23,
                        serializer::BLOCK_STATE,
                        Value::Id(t.map_or(0, |s| s.0 as i32)),
                    )
                };
                let air = self.pack.assets.registry.default_state("minecraft:air");
                let shown = tag.get("block_state").and_then(|t| self.state_from_nbt(t));
                (vec![state(shown.or(air))], vec![state(air)])
            }
            "minecraft:item_display" => {
                // DATA_ITEM_STACK and DATA_ITEM_DISPLAY_ID (`ItemDisplayContext`).
                item_stack = tag
                    .get("item")
                    .and_then(horizon_client::save::item_from_nbt);
                let context = match tag.get("item_display") {
                    Some(Tag::String(s)) => match s.to_string_lossy().as_str() {
                        "thirdperson_lefthand" => 1,
                        "thirdperson_righthand" => 2,
                        "firstperson_lefthand" => 3,
                        "firstperson_righthand" => 4,
                        "head" => 5,
                        "gui" => 6,
                        "ground" => 7,
                        "fixed" => 8,
                        "on_shelf" => 9,
                        _ => 0,
                    },
                    _ => 0,
                };
                (
                    vec![
                        (23, serializer::ITEM_STACK, Value::Item(None)),
                        (24, serializer::BYTE, Value::Byte(context)),
                    ],
                    vec![
                        (23, serializer::ITEM_STACK, Value::Item(None)),
                        (24, serializer::BYTE, Value::Byte(0)),
                    ],
                )
            }
            _ => return None,
        };
        e.meta = display_data(tag, false);
        e.meta.extend(meta);
        e.defaults = display_data(&empty, true);
        e.defaults.extend(defaults);
        item_stack
    }

    /// `ClientboundAddEntityPacket(Entity, ServerEntity, int)`: the entity's
    /// own UUID (`Entity.getUUID`), the one `/data`, saves and UUID
    /// selectors use, so a client can name it back (the crosshair UUID that
    /// `ClientSuggestionProvider.getSelectedEntities` suggests).
    pub(super) fn lan_entity_uuids(&mut self, states: &mut States) {
        // A bare state is never paired: its UUID is never read.
        for e in states.as_mut_slice().iter_mut().filter(|e| !e.bare) {
            e.uuid = ints_uuid(self.uuid_for(e.id as u32));
            if let Some(bits) = self.persist.forced_meta.get(&(e.id as u32)) {
                e.forced_meta = *bits;
            }
        }
    }

    /// ServerEntity.sendChanges clears needsSync once it has looked, and
    /// SteppedInterpolationTracker.updateTracking clears syncPosition: once the states for the
    /// tracking pass are taken.
    pub(super) fn lan_syncs_looked(&mut self) {
        for m in &mut self.mobs {
            m.needs_sync = false;
            m.sync_position = false;
        }
        self.creatures
            .projectiles
            .for_each_mut(|p| p.needs_sync = false);
        for d in &mut self.dropped {
            d.needs_sync = false;
        }
        self.tridents.iter_mut().for_each(|t| t.needs_sync = false);
    }

    /// Gives the entities Horizon keeps without an id one (`Entity.id`, from
    /// the same counter as the rest) before they are first tracked.
    pub(super) fn lan_assign_ids(&mut self) {
        // LeashFenceKnotEntity: the tie made the knot of a fence post
        // (game/leads.rs `tie_knot`); the tracker shows that same entity.
        // `notifyLeasheeRemoved` discards it once the last lead is gone.
        self.forget_untied_knots();
        let next = &mut self.next_entity;
        use super::entity_residency::assign_resident_ids;
        assign_resident_ids!(self.tnts, t, next, None);
        assign_resident_ids!(self.falling, f, next, Some(&f.attachment_lifetime));
        assign_resident_ids!(
            self.creatures.projectiles,
            p,
            next,
            Some(&p.attachment_lifetime)
        );
        assign_resident_ids!(self.tridents, t, next, None);
        assign_resident_ids!(self.rockets, r, next, None);
        assign_resident_ids!(self.crystals, c, next, None);
        self.bolts.iter_mut().for_each(|b| {
            assign_entity_id(next, &mut b.id);
        });
        assign_resident_ids!(self.ominous_spawners, o, next, None);
        self.potions.thrown_ids().for_each(|id| {
            assign_entity_id(next, id);
        });
        self.end.eye_ids().for_each(|id| {
            assign_entity_id(next, id);
        });
        assign_resident_ids!(self.breath_clouds, b, next, None);
    }

    /// Primed TNT, falling blocks, projectiles, tridents, firework rockets and
    /// end crystals, as guests track them.
    fn lan_projectile_states(&mut self, out: &mut States, after: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let hub = &mut host.hub;
        let owner = |who: Option<horizon_client::mobs::Who>| match who {
            Some(horizon_client::mobs::Who::Mob(id)) => id as i32,
            Some(horizon_client::mobs::Who::Player) | None => 0,
        };
        let at = |p: horizon_core::geometry::Vec3| [p.x, p.y, p.z];
        for f in self.fangs.list.iter().filter(|f| f.id > after) {
            add_state(out, f.id, "minecraft:evoker_fangs", f.pos, f.yaw, false);
        }
        // LightningBolt: each client draws and flashes its own.
        for b in self.bolts.iter().filter(|b| b.id > after) {
            add_state(out, b.id, "minecraft:lightning_bolt", b.at, 0.0, false);
        }
        for t in self.tnts.iter().filter(|t| t.id > after) {
            let e = add_state(out, t.id, "minecraft:tnt", at(t.position), 0.0, t.on_ground);
            e.velocity = at(t.velocity);
            // PrimedTnt.DATA_FUSE_ID, 80 by default; requiresPrecisePosition
            // while it touches a block.
            if t.fuse != 80 {
                e.meta.push((8, serializer::INT, Value::Int(t.fuse)));
            }
            e.precise = t.on_ground;
        }
        for f in self.falling.iter().filter(|f| f.id > after) {
            let e = add_state(
                out,
                f.id,
                "minecraft:falling_block",
                at(f.position),
                0.0,
                f.on_ground,
            );
            e.velocity = at(f.velocity);
            // getAddEntityPacket: the block state; DATA_START_POS.
            e.data = f.state.0 as i32;
            if f.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            let start = [f.start_pos.x, f.start_pos.y, f.start_pos.z];
            if start != [0; 3] {
                e.meta.push((8, serializer::BLOCK_POS, Value::Pos(start)));
            }
        }
        for p in self.creatures.projectiles.iter().filter(|p| p.id > after) {
            use horizon_client::projectiles::ProjectileKind as K;
            // A spectral arrow is its own entity type.
            let kind = p.entity_type();
            let data = match p.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(p.owner),
            };
            let e = add_state(out, p.id, kind, at(p.position), p.yaw, p.on_ground);
            e.pitch = p.pitch;
            e.velocity = at(p.velocity);
            e.needs_sync = p.needs_sync;
            // Projectile.getAddEntityPacket: the owner's id.
            e.data = data;
            let uuid = e.uuid;
            // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.baseTick's
            // `setSharedFlagOnFire(remainingFireTicks > 0)`, which a client's
            // isOnFire reads to draw the flames (a Flame bow's arrow).
            let fire = p.arrow.as_ref().map_or(p.synced_fire, |a| a.on_fire());
            if fire {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            if p.air_supply != 300 {
                e.meta.push((1, serializer::INT, Value::Int(p.air_supply)));
            }
            match &p.kind {
                K::Arrow { critical, .. } => {
                    let a = p.arrow.as_deref();
                    push_arrow_meta(
                        &mut e.meta,
                        *critical,
                        a.is_some_and(|a| a.no_physics),
                        a.map_or(0, |a| a.pierce),
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    );
                    // Arrow.ID_EFFECT_COLOR, -1 by default.
                    let color = p.arrow_effect_color();
                    if color != -1 {
                        e.meta.push((11, serializer::INT, Value::Int(color)));
                    }
                }
                // ThrownTrident.ID_LOYALTY (none for a mob's) and ID_FOIL, on
                // AbstractArrow's flags.
                K::Trident => {
                    let a = p.arrow.as_deref();
                    push_arrow_meta(
                        &mut e.meta,
                        false,
                        a.is_some_and(|a| a.no_physics),
                        0,
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    );
                    if a.is_some_and(|a| a.pickup_item.has_any_enchantments()) {
                        e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
                    }
                }
                // WitherSkull.DATA_DANGEROUS.
                K::WitherSkull { dangerous: true } => {
                    e.meta.push((8, serializer::BOOLEAN, Value::Bool(true)));
                }
                // ThrowableItemProjectile.DATA_ITEM_STACK: the potion's
                // `potion_contents`, whose effects the guest's splash and
                // whose colour its drawing need. A plain splash potion is the
                // client's default and is not sent.
                K::Potion(stack, _) if stack.extra.is_some() || stack.count != 1 => {
                    e.meta.push((
                        8,
                        serializer::ITEM_STACK,
                        Value::Item(hub.stack(&Some((**stack).clone()))),
                    ));
                }
                // Fireball.DATA_ITEM_STACK: the item it is drawn as, which a
                // guest has no other way of learning. A plain fire charge is
                // the client's default and is not sent.
                K::SmallFireball | K::LargeFireball { .. }
                    if p.item.as_ref().is_some_and(|i| {
                        &*i.item != "minecraft:fire_charge" || i.count != 1 || i.extra.is_some()
                    }) =>
                {
                    e.meta
                        .push((8, serializer::ITEM_STACK, Value::Item(hub.stack(&p.item))));
                }
                _ => {}
            }
            // ChunkMap.addEntity (#7461): a fresh arrow joined before its first tick
            // is paired as it joined with IN_GROUND=false, and its first update is deferred
            // to the next tick's send_changes so impact updates (e.g. point-blank wall
            // collision) arrive after the client's first tick has completed.
            if !hub.tracks(p.id as i32)
                && let Some(a) = p.arrow.as_deref()
                && let (Some(spawn_pos), Some(spawn_vel), Some((pitch, yaw))) =
                    (a.spawn_pos, a.spawn_velocity, a.spawn_rot)
            {
                let mut joined = state(p.id, kind, at(spawn_pos), yaw, false);
                joined.pitch = pitch;
                joined.velocity = at(spawn_vel);
                joined.needs_sync = p.needs_sync;
                joined.data = data;
                joined.uuid = uuid;
                if a.on_fire() {
                    joined.meta.push((0, serializer::BYTE, Value::Byte(1)));
                }
                match &p.kind {
                    K::Arrow { critical, .. } => {
                        push_arrow_meta(&mut joined.meta, *critical, a.no_physics, a.pierce, false);
                        let color = p.arrow_effect_color();
                        if color != -1 {
                            joined.meta.push((11, serializer::INT, Value::Int(color)));
                        }
                    }
                    K::Trident => {
                        push_arrow_meta(&mut joined.meta, false, a.no_physics, 0, false);
                        if a.pickup_item.has_any_enchantments() {
                            joined
                                .meta
                                .push((12, serializer::BOOLEAN, Value::Bool(true)));
                        }
                    }
                    _ => {}
                }
                hub.note_fresh_joined(&[joined]);
            }
        }
        // Thrown potions and bottles: ThrowableItemProjectile.DATA_ITEM_STACK
        // (the potion's colour), which a plain bottle's type already has.
        for (id, kind, stack, position, velocity) in
            self.potions.thrown_tracked().filter(|(id, ..)| *id > after)
        {
            let e = add_state(out, id, kind, at(position), 0.0, false);
            e.velocity = at(velocity);
            if stack.extra.is_some() || stack.count != 1 {
                e.meta.push((
                    8,
                    serializer::ITEM_STACK,
                    Value::Item(hub.stack(&Some(stack.clone()))),
                ));
            }
        }
        // EyeOfEnder: its item is the plain eye (DATA_ITEM_STACK's default).
        for (id, position, velocity) in self.end.eyes_tracked().filter(|(id, ..)| *id > after) {
            let e = add_state(out, id, "minecraft:eye_of_ender", position, 0.0, false);
            e.velocity = velocity;
        }
        for t in self.tridents.iter().filter(|t| t.id > after) {
            let data = match t.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(Some(horizon_client::mobs::Who::Player)),
            };
            let e = add_state(
                out,
                t.id,
                "minecraft:trident",
                at(t.position),
                t.yaw,
                t.on_ground,
            );
            e.pitch = t.pitch;
            e.velocity = at(t.velocity);
            e.needs_sync = t.needs_sync;
            // Projectile.getAddEntityPacket: the owner's id, the guest's that
            // threw it.
            e.data = data;
            push_arrow_meta(&mut e.meta, false, t.no_physics, 0, t.in_ground);
            // Entity.DATA_SHARED_FLAGS_ID: FLAG_ONFIRE.
            if t.fire_ticks > 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            // ThrownTrident.ID_LOYALTY and ID_FOIL.
            if t.loyalty != 0 {
                e.meta
                    .push((11, serializer::BYTE, Value::Byte(t.loyalty as i8)));
            }
            if t.stack.has_any_enchantments() {
                e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        for r in self.rockets.iter().filter(|r| r.id > after) {
            let plain = &*r.item.item == "minecraft:firework_rocket"
                && r.item.count == 1
                && r.item.extra.is_none();
            let item = (!plain).then(|| hub.stack(&Some(r.item.clone())));
            let boosted = match r.guest {
                Some(conn) => hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .map(|g| g.entity_id),
                None => None,
            };
            let e = add_state(
                out,
                r.id,
                "minecraft:firework_rocket",
                at(r.position),
                0.0,
                r.on_ground,
            );
            e.velocity = at(r.velocity);
            // DATA_ID_FIREWORKS_ITEM once set from a stack, DATA_ATTACHED_TO_TARGET,
            // DATA_SHOT_AT_ANGLE.
            if let Some(item) = item {
                e.meta.push((8, serializer::ITEM_STACK, Value::Item(item)));
            }
            if r.attached
                && let Some(player) = boosted
            {
                e.meta
                    .push((9, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(player + 1)));
            }
            if r.angled {
                e.meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        for c in self.crystals.iter().filter(|c| c.id > after) {
            let e = add_state(
                out,
                c.id,
                "minecraft:end_crystal",
                at(c.position),
                0.0,
                false,
            );
            // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM (true by default).
            // The beam's default is listed so that clearing it (the respawn
            // sequence ending) is sent to the guests instead of leaving the old beam.
            e.defaults
                .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)));
            if c.beam.is_some() {
                e.meta
                    .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(c.beam)));
            }
            if !c.show_bottom {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(false)));
            }
        }
        // The dragon's breath and lingering potion clouds (AreaEffectCloud's DATA_RADIUS,
        // DATA_WAITING for its 20-tick wait, and DATA_PARTICLE): the
        // client draws them from these (`tick_remote_clouds`).
        let breath = hub.particle(
            &horizon_client::particles::ParticleOptions::simple(
                horizon_client::particles::ParticleType::DragonBreath,
            ),
            None,
        );
        for b in self.breath_clouds.iter().filter(|b| b.id > after) {
            // The hub is out of `self.lan` while these states are built, so
            // the particle must be encoded through the hub passed in: going
            // through `network_particle` would drop it and leave the client
            // on the default white particle.
            let particle = match &b.potion {
                Some(p) => hub.particle(&p.particle(), None),
                None => breath.clone(),
            };
            let e = add_state(out, b.id, "minecraft:area_effect_cloud", b.at, 0.0, false);
            e.meta.push((8, serializer::FLOAT, Value::Float(b.radius)));
            if b.waiting() {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(true)));
            }
            if let Some(p) = &particle {
                e.meta
                    .push((10, serializer::PARTICLE, Value::Particle(p.clone())));
            }
            e.defaults
                .push((9, serializer::BOOLEAN, Value::Bool(false)));
        }
        out.extend(self.lan_dragon_state());
    }
}

/// `AbstractArrow`'s ID_FLAGS (critical 1, no physics 2), PIERCE_LEVEL and
/// IN_GROUND, when they differ from the defaults.
fn push_arrow_meta(
    meta: &mut Vec<(u8, i32, Value)>,
    critical: bool,
    no_physics: bool,
    pierce: u8,
    in_ground: bool,
) {
    let flags = i8::from(critical) | i8::from(no_physics) << 1;
    if flags != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(flags)));
    }
    if pierce != 0 {
        meta.push((9, serializer::BYTE, Value::Byte(pierce as i8)));
    }
    if in_ground {
        meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NBT constructors assign ids. Reinstall unbound owners to exercise delayed tracking.
    macro_rules! pending {
        ($list:expr) => {{
            let mut owner = $list.remove($list.len() - 1);
            owner.id = 0;
            $list.push(owner);
        }};
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn tracking_ids_preserve_all_family_interleaving() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            for (kind, nbt) in [
                ("minecraft:tnt", "{fuse:80s}"),
                (
                    "minecraft:falling_block",
                    "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
                ),
                ("minecraft:arrow", "{}"),
                ("minecraft:trident", "{}"),
                ("minecraft:firework_rocket", "{LifeTime:20}"),
                ("minecraft:end_crystal", "{}"),
                (
                    "minecraft:ominous_item_spawner",
                    "{item:{id:\"minecraft:stone\",count:1}}",
                ),
                ("minecraft:area_effect_cloud", "{Duration:200,Radius:3.0f}"),
            ] {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
            }
            pending!(game.tnts);
            pending!(game.falling);
            pending!(game.creatures.projectiles);
            pending!(game.tridents);
            pending!(game.rockets);
            pending!(game.crystals);
            pending!(game.ominous_spawners);
            pending!(game.breath_clouds);
            game.spawn_remote_bolt(0, [1.0, 70.0, 1.0]);
            game.potions.add_remote_thrown(
                0,
                "minecraft:splash_potion",
                Default::default(),
                Default::default(),
            );
            game.end.add_remote_eye(0, [1.0, 70.0, 1.0], [0.0; 3]);
            let base = game.next_entity;
            let joined = game.persist.joined_seq;
            let announced = game.persist.live.clone();
            let mut slots = crate::game::entity::taken_pass_tests::SlotIdentities::default();
            for _ in 0..2 {
                game.lan_assign_ids();
                let ids = [
                    game.tnts[0].id,
                    game.falling[0].id,
                    game.creatures.projectiles[0].id,
                    game.tridents[0].id,
                    game.rockets[0].id,
                    game.crystals[0].id,
                    game.bolts[0].id,
                    game.ominous_spawners[0].id,
                    *game.potions.thrown_ids().next().unwrap(),
                    *game.end.eye_ids().next().unwrap(),
                    game.breath_clouds[0].id,
                ];
                assert_eq!(
                    ids,
                    std::array::from_fn::<_, 11, _>(|i| base + i as u32 + 1)
                );
                assert_eq!(game.next_entity, base + 11);
                assert_eq!(game.persist.joined_seq, joined);
                assert_eq!(game.persist.live, announced);
                let epoch = game.native_lifecycle.epoch().unwrap();
                slots.observe(&mut game.falling, epoch, |f| f.id);
                game.creatures.projectiles.for_each_mut(|p| {
                    let id = p.id;
                    slots.observe_one(p, epoch, id);
                });
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn tracking_ids_refuse_bound_zero_owners() {
        for kind in ["minecraft:falling_block", "minecraft:arrow"] {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let nbt = if kind == "minecraft:arrow" {
                    "{}"
                } else {
                    "{BlockState:{Name:\"minecraft:sand\"},Time:1}"
                };
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
                let epoch = game.native_lifecycle.epoch().unwrap();
                let mut slots = crate::game::entity::taken_pass_tests::SlotIdentities::default();
                if kind == "minecraft:arrow" {
                    pending!(game.creatures.projectiles);
                    game.creatures
                        .projectiles
                        .for_each_mut(|p| slots.observe_one(p, epoch, 0));
                } else {
                    pending!(game.falling);
                    slots.observe(&mut game.falling, epoch, |f| f.id);
                }
                let before = game.next_entity;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.lan_assign_ids()
                }));
                assert!(result.is_err());
                assert_eq!(game.next_entity, before);
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn tracking_ids_only_number_the_spawn_buffer_during_a_taken_pass() {
        macro_rules! check {
            ($kind:expr, $nbt:expr, $game:ident, $list:expr) => {
                crate::game::native_entity_refs::tests::with_game(|$game| {
                    $game
                        .summon_with_nbt($kind, [1.0, 70.0, 1.0], $nbt)
                        .unwrap();
                    pending!($list);
                    let taken = $list.lift();
                    $game
                        .summon_with_nbt($kind, [2.0, 70.0, 1.0], $nbt)
                        .unwrap();
                    pending!($list);
                    let base = $game.next_entity;
                    $game.lan_assign_ids();
                    assert_eq!(taken[0].id, 0);
                    assert_eq!($list[0].id, base + 1);
                    $list.restore(
                        taken,
                        horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
                    );
                    $game.lan_assign_ids();
                    assert_eq!(
                        $list.iter().map(|p| p.id).collect::<Vec<_>>(),
                        [base + 2, base + 1]
                    );
                    assert_eq!($game.next_entity, base + 2);
                });
            };
        }
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.summon_with_nbt("minecraft:arrow", [1.0, 70.0, 1.0], "{}")
                .unwrap();
            pending!(game.creatures.projectiles);
            let taken = game.take_entities();
            game.summon_with_nbt("minecraft:arrow", [2.0, 70.0, 1.0], "{}")
                .unwrap();
            pending!(game.creatures.projectiles);
            let base = game.next_entity;
            game.lan_assign_ids();
            assert_eq!(game.creatures.projectiles[0].id, base + 1);
            game.restore_entities(taken);
            assert_eq!(game.creatures.projectiles[0].id, 0);
            game.lan_assign_ids();
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>(),
                [base + 2, base + 1]
            );
            assert_eq!(game.next_entity, base + 2);
        });
        check!(
            "minecraft:area_effect_cloud",
            "{Duration:200,Radius:3.0f}",
            game,
            game.breath_clouds
        );
        check!(
            "minecraft:ominous_item_spawner",
            "{}",
            game,
            game.ominous_spawners
        );
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn late_dragon_ids_reserve_parts_before_crystals() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.summon_with_nbt("minecraft:ender_dragon", [1.0, 70.0, 1.0], "{}")
                .unwrap();
            let mut dragon = game.dragon.extract().unwrap();
            dragon.id = 0;
            assert!(game.dragon.install(dragon).is_ok());
            for _ in 0..2 {
                game.summon_with_nbt("minecraft:end_crystal", [1.0, 70.0, 1.0], "{}")
                    .unwrap();
                pending!(game.crystals);
            }
            let base = game.next_entity;
            for _ in 0..2 {
                game.assign_dragon_and_crystal_ids();
                assert_eq!(game.dragon.as_ref().unwrap().id, base + 1);
                assert_eq!(
                    game.crystals.iter().map(|c| c.id).collect::<Vec<_>>(),
                    [base + 10, base + 11]
                );
                assert_eq!(game.next_entity, base + 11);
                game.lan_assign_ids();
                assert_eq!(game.next_entity, base + 11);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn kept_display_data_is_what_a_build_gives() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let snbt = |s: &str| horizon_client::command::snbt::parse_fully(s).unwrap();
            let displays: [(&str, Tag, Tag); 3] = [
                (
                    "minecraft:text_display",
                    snbt(r#"{text:{text:"Shop",color:"gold"},billboard:"center",line_width:120}"#),
                    snbt(r#"{text:{text:"Sold out",bold:1b},billboard:"center",background:0}"#),
                ),
                (
                    "minecraft:block_display",
                    snbt(
                        r#"{block_state:{Name:"minecraft:oak_stairs",Properties:{facing:"east"}}}"#,
                    ),
                    snbt(r#"{block_state:{Name:"minecraft:glass"},brightness:{sky:15,block:3}}"#),
                ),
                (
                    "minecraft:item_display",
                    snbt(r#"{item:{id:"minecraft:diamond",count:3},item_display:"gui"}"#),
                    snbt(
                        r#"{item:{id:"minecraft:apple",count:1},item_display:"head",glow_color_override:255}"#,
                    ),
                ),
            ];
            let state = |kind: &str| EntityState {
                kind: kind.into(),
                ..Default::default()
            };
            for (id, (kind, first, second)) in displays.iter().enumerate() {
                let id = 900 + id as u32;
                // Ticks with the first compound, then the second, then the first
                // again: each answer is a fresh build's.
                for tag in [first, first, first, second, second, first] {
                    let mut built = state(kind);
                    let built_item = game.lan_display_data(tag, &mut built);
                    assert!(!built.meta.is_empty(), "{kind}");
                    let mut kept = state(kind);
                    let kept_item = game.lan_display_data_kept(id, tag, &mut kept);
                    assert_eq!(format!("{kept_item:?}"), format!("{built_item:?}"));
                    assert_eq!(
                        format!("{:?}", kept.meta),
                        format!("{:?}", built.meta),
                        "{kind}"
                    );
                    assert_eq!(
                        format!("{:?}", kept.defaults),
                        format!("{:?}", built.defaults),
                        "{kind}"
                    );
                }
            }
        });
    }

    #[test]
    fn summoned_ids_are_unique_and_survive_the_tracking_pass() {
        let mut next = 40;
        let mut roots = [0; 3];
        let returned = roots.each_mut().map(|id| assign_entity_id(&mut next, id));
        assert_eq!(returned, [41, 42, 43]);
        // Tracking sees the same roots plus an unrelated deferred entity.
        for id in &mut roots {
            assign_entity_id(&mut next, id);
        }
        let mut later = 0;
        assert_eq!(assign_entity_id(&mut next, &mut later), 44);
        assert_eq!(roots, returned);
        assert_eq!(next, 44);
    }

    #[test]
    fn armor_stand_poses_send_only_what_differs() {
        let mut stand = horizon_client::entities::ArmorStand::new(1, Default::default(), 0.0);
        stand.pose[0] = [10.0, 20.0, 0.0];
        let meta = stand_poses(&stand);
        assert_eq!(
            meta,
            vec![(
                16,
                serializer::ROTATIONS,
                Value::Floats(vec![10.0, 20.0, 0.0])
            )]
        );
        let plain = horizon_client::entities::ArmorStand::new(1, Default::default(), 0.0);
        assert!(stand_poses(&plain).is_empty());
    }

    /// Issue #8050: a cart's custom display block and offset reach clients
    /// as entries 11 and 12, the offset only off the kind's default.
    #[test]
    fn cart_display_block_and_offset_are_synched() {
        let mut e = state(1, "minecraft:minecart", [0.0; 3], 0.0, false);
        cart_display_data(&mut e, Some(BlockStateId(42)), Some(14), 6);
        assert_eq!(
            e.meta,
            vec![
                (11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(42)),
                (12, serializer::INT, Value::Int(14)),
            ]
        );
        assert_eq!(
            e.defaults,
            vec![
                (11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)),
                (12, serializer::INT, Value::Int(6)),
            ]
        );
        let mut plain = state(2, "minecraft:hopper_minecart", [0.0; 3], 0.0, false);
        cart_display_data(&mut plain, None, Some(1), 1);
        assert!(plain.meta.is_empty());
        assert_eq!(plain.defaults[1], (12, serializer::INT, Value::Int(1)));
    }

    #[test]
    fn add_state_and_meta_helpers_reuse_pooled_capacity() {
        let mut states = States::default();
        let (kind_ptr, meta_ptr, meta_cap) = {
            let e = add_state(
                &mut states,
                10,
                "minecraft:armor_stand",
                [1.0, 2.0, 3.0],
                45.0,
                true,
            );
            let mut s = horizon_client::entities::ArmorStand::new(10, Default::default(), 0.0);
            s.pose[0] = [5.0, 0.0, 0.0];
            push_stand_poses(&mut e.meta, &s);
            vehicle_meta(e, 2, 1, 3.5);
            (e.kind.as_ptr(), e.meta.as_ptr(), e.meta.capacity())
        };
        assert!(meta_cap >= 2);

        states.clear();
        let (kind_ptr2, meta_ptr2, meta_cap2) = {
            let e = add_state(
                &mut states,
                11,
                "minecraft:boat",
                [4.0, 5.0, 6.0],
                0.0,
                false,
            );
            vehicle_meta(e, 0, 1, 0.0);
            (e.kind.as_ptr(), e.meta.as_ptr(), e.meta.capacity())
        };
        assert_eq!(kind_ptr2, kind_ptr);
        assert_eq!(meta_ptr2, meta_ptr);
        assert_eq!(meta_cap2, meta_cap);
    }
}

#[cfg(test)]
mod vehicle_meta_tests {
    use super::*;

    #[test]
    fn vehicle_wobble_state_carries_its_defaults() {
        let mut e = state(5, "minecraft:minecart", [0.0; 3], 0.0, false);
        vehicle_meta(&mut e, 0, 1, 0.0);
        assert!(e.meta.is_empty());
        assert_eq!(
            e.defaults,
            vec![
                (8, serializer::INT, Value::Int(0)),
                (9, serializer::INT, Value::Int(1)),
                (10, serializer::FLOAT, Value::Float(0.0)),
            ]
        );
        vehicle_meta(&mut e, 4, -1, 3.0);
        assert_eq!(e.meta.len(), 3);
    }
}
