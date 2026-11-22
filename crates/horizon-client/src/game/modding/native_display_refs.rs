//! Display handles for native server mods: block, item and text displays.
//!
//! Horizon keeps a display unsimulated, as the compound vanilla saves
//! (`Display.addAdditionalSaveData`). Reads decode that compound the way the
//! game builds the entity data it sends (so a handle reads what clients see),
//! and a write changes the compound: the next tracking pass sends the changed
//! entries as `SetEntityData`, and the save holds them.
use super::{
    Game,
    commands::{OtherEntity, Target},
};
use horizon_client::{
    items::ItemStack,
    save::item_from_nbt,
    scoreboard::{comp_text, json_to_tag, tag_to_json},
    text::Text,
    text_display::{display_data, text_display_data},
};
use horizon_protocol::entity_data::Value;
use mod_api::{
    Billboard, DisplayKind, DisplayRef, EntityRef, ItemDisplayContext, TextAlignment,
    Transformation,
};
use {
    horizon_core::geometry::Aabb,
    horizon_core::nbt::{Compound, Tag},
    horizon_world::registry::BlockStateId,
};

impl OtherEntity {
    /// The display's physical box is a single point at its feet. Its NBT width
    /// and height only change the culling box used by the renderer.
    pub(super) fn display_bounds(&self) -> Option<Aabb> {
        DisplayKind::from_entity_type(&self.kind)?;
        Some(Aabb::new(self.position, self.position))
    }
}

/// The entity data entry `id` of a display's compound: what it sets, else the default.
fn entry(kind: DisplayKind, tag: &Tag, id: u8) -> Option<Value> {
    let find = |tag: &Tag, all: bool| {
        let mut data = display_data(tag, all);
        if kind == DisplayKind::Text {
            data.extend(text_display_data(tag, all));
        }
        data.into_iter().find(|e| e.0 == id).map(|e| e.2)
    };
    find(tag, false).or_else(|| find(&Tag::Compound(Compound::new()), true))
}

fn floats(values: &[f32]) -> Tag {
    Tag::List(values.iter().map(|x| Tag::Float(x.to_bits())).collect())
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|x| x.is_finite())
}

/// A quaternion `Transformation.EXTENDED_CODEC` can normalize.
fn rotation(q: &[f32; 4]) -> bool {
    finite(q) && q.iter().map(|x| x * x).sum::<f32>() > 1.0e-12
}

impl Game<'_> {
    /// The display's kind and entity id, or None for a stale handle, a client,
    /// or an entity that isn't a display.
    fn display_entity(&self, display: DisplayRef) -> Option<(DisplayKind, u32)> {
        let Target::Other(id) = self.native_entity_target(display.entity())? else {
            return None;
        };
        let kind = self.others.iter().find(|o| o.id == id)?.kind.as_str();
        Some((DisplayKind::from_entity_type(kind)?, id))
    }

    /// The display's compound, if it is a display of kind `only` (any when None).
    fn display_tag(
        &self,
        display: DisplayRef,
        only: Option<DisplayKind>,
    ) -> Option<(DisplayKind, Tag)> {
        let (kind, id) = self.display_entity(display)?;
        if only.is_some_and(|only| only != kind) {
            return None;
        }
        let tag = self.persist.raw.get(&id).cloned();
        Some((kind, tag.unwrap_or_else(|| Tag::Compound(Compound::new()))))
    }

    fn display_value(
        &self,
        display: DisplayRef,
        only: Option<DisplayKind>,
        id: u8,
    ) -> Option<Value> {
        let (kind, tag) = self.display_tag(display, only)?;
        entry(kind, &tag, id)
    }

    fn display_float(&self, display: DisplayRef, id: u8) -> Option<f32> {
        match self.display_value(display, None, id)? {
            Value::Float(v) => Some(v),
            _ => None,
        }
    }

    fn display_int(&self, display: DisplayRef, only: Option<DisplayKind>, id: u8) -> Option<i32> {
        match self.display_value(display, only, id)? {
            Value::Int(v) => Some(v),
            _ => None,
        }
    }

    fn display_byte(&self, display: DisplayRef, only: DisplayKind, id: u8) -> Option<i8> {
        match self.display_value(display, Some(only), id)? {
            Value::Byte(v) => Some(v),
            _ => None,
        }
    }

    /// Edits the compound of a display of kind `only` (any when None).
    fn display_edit(
        &mut self,
        display: DisplayRef,
        only: Option<DisplayKind>,
        edit: impl FnOnce(&mut Compound),
    ) -> bool {
        let Some((kind, id)) = self.display_entity(display) else {
            return false;
        };
        if only.is_some_and(|only| only != kind) {
            return false;
        }
        let tag = self
            .persist
            .raw
            .entry(id)
            .or_insert_with(|| Tag::Compound(Compound::new()));
        let Tag::Compound(c) = tag else {
            return false;
        };
        edit(c);
        true
    }

    fn display_put(
        &mut self,
        display: DisplayRef,
        only: Option<DisplayKind>,
        key: &str,
        value: Tag,
    ) -> bool {
        self.display_edit(display, only, |c| {
            c.insert(key.into(), value);
        })
    }

    fn display_put_float(&mut self, display: DisplayRef, key: &str, value: f32) -> bool {
        value.is_finite() && self.display_put(display, None, key, Tag::Float(value.to_bits()))
    }

    /// Narrows an entity to a display, or None for another kind, a stale
    /// handle or a client.
    pub(super) fn display(&self, entity: EntityRef) -> Option<DisplayRef> {
        let display = DisplayRef::new(entity);
        self.display_entity(display).map(|_| display)
    }

    /// Which `Display` subclass the entity is.
    pub(super) fn display_kind(&self, display: DisplayRef) -> Option<DisplayKind> {
        Some(self.display_entity(display)?.0)
    }

    /// `Display.createTransformation`: the transformation as shown.
    pub(super) fn display_transformation(&self, display: DisplayRef) -> Option<Transformation> {
        let (kind, tag) = self.display_tag(display, None)?;
        let part = |id| match entry(kind, &tag, id)? {
            Value::Floats(v) => Some(v),
            _ => None,
        };
        Some(Transformation {
            translation: part(11)?.try_into().ok()?,
            scale: part(12)?.try_into().ok()?,
            left_rotation: part(13)?.try_into().ok()?,
            right_rotation: part(14)?.try_into().ok()?,
        })
    }

    /// Sets the transformation. False for a value that isn't finite or a
    /// zero quaternion. Clients ease to it when the start delay is set
    /// (`set_start_interpolation`).
    pub(super) fn display_set_transformation(
        &mut self,
        display: DisplayRef,
        transformation: Transformation,
    ) -> bool {
        let t = transformation;
        if !finite(&t.translation)
            || !finite(&t.scale)
            || !rotation(&t.left_rotation)
            || !rotation(&t.right_rotation)
        {
            return false;
        }
        let tag = Tag::compound([
            ("translation", floats(&t.translation)),
            ("left_rotation", floats(&t.left_rotation)),
            ("scale", floats(&t.scale)),
            ("right_rotation", floats(&t.right_rotation)),
        ]);
        if !self.display_put(display, None, "transformation", tag) {
            return false;
        }
        let index = self
            .others
            .iter()
            .position(|o| o.id == display.entity().id())
            .unwrap();
        self.others.update(index, |o| {
            if o.kind == "minecraft:item_display" {
                o.item_transformation = Some(t.into());
            }
        });
        true
    }

    /// `Display.getBillboardConstraints`.
    pub(super) fn display_billboard(&self, display: DisplayRef) -> Option<Billboard> {
        match self.display_value(display, None, 15)? {
            Value::Byte(v) => Billboard::ALL.get(usize::try_from(v).ok()?).copied(),
            _ => None,
        }
    }

    /// `Display.setBillboardConstraints`.
    pub(super) fn display_set_billboard(
        &mut self,
        display: DisplayRef,
        billboard: Billboard,
    ) -> bool {
        let name = Tag::String(billboard.name().into());
        self.display_put(display, None, "billboard", name)
    }

    /// `Display.getBrightnessOverride`: block and sky light, 0 to 15 each, or None for no override.
    pub(super) fn display_brightness(&self, display: DisplayRef) -> Option<(u8, u8)> {
        let packed = self.display_int(display, None, 16)?;
        (packed != -1).then_some((((packed >> 4) & 15) as u8, ((packed >> 20) & 15) as u8))
    }

    /// `Display.setBrightnessOverride`; None clears the override. False for a
    /// light level above 15.
    pub(super) fn display_set_brightness(
        &mut self,
        display: DisplayRef,
        brightness: Option<(u8, u8)>,
    ) -> bool {
        if brightness.is_some_and(|(block, sky)| block > 15 || sky > 15) {
            return false;
        }
        self.display_edit(display, None, |c| match brightness {
            Some((block, sky)) => {
                let light = Tag::compound([
                    ("block", Tag::Int(i32::from(block))),
                    ("sky", Tag::Int(i32::from(sky))),
                ]);
                c.insert("brightness".into(), light);
            }
            None => {
                c.remove(&"brightness".into());
            }
        })
    }

    /// `Display.getViewRange`.
    pub(super) fn display_view_range(&self, display: DisplayRef) -> Option<f32> {
        self.display_float(display, 17)
    }

    /// `Display.setViewRange`. False for a value that isn't finite.
    pub(super) fn display_set_view_range(&mut self, display: DisplayRef, range: f32) -> bool {
        self.display_put_float(display, "view_range", range)
    }

    /// `Display.getShadowRadius`.
    pub(super) fn display_shadow_radius(&self, display: DisplayRef) -> Option<f32> {
        self.display_float(display, 18)
    }

    /// `Display.setShadowRadius`. False for a value that isn't finite.
    pub(super) fn display_set_shadow_radius(&mut self, display: DisplayRef, radius: f32) -> bool {
        self.display_put_float(display, "shadow_radius", radius)
    }

    /// `Display.getShadowStrength`.
    pub(super) fn display_shadow_strength(&self, display: DisplayRef) -> Option<f32> {
        self.display_float(display, 19)
    }

    /// `Display.setShadowStrength`. False for a value that isn't finite.
    pub(super) fn display_set_shadow_strength(
        &mut self,
        display: DisplayRef,
        strength: f32,
    ) -> bool {
        self.display_put_float(display, "shadow_strength", strength)
    }

    /// `Display.getWidth`: the culling box's width, 0 for none.
    pub(super) fn display_width(&self, display: DisplayRef) -> Option<f32> {
        self.display_float(display, 20)
    }

    /// `Display.setWidth`. False for a value that isn't finite.
    pub(super) fn display_set_width(&mut self, display: DisplayRef, width: f32) -> bool {
        self.display_put_float(display, "width", width)
    }

    /// `Display.getHeight`: the culling box's height, 0 for none.
    pub(super) fn display_height(&self, display: DisplayRef) -> Option<f32> {
        self.display_float(display, 21)
    }

    /// `Display.setHeight`. False for a value that isn't finite.
    pub(super) fn display_set_height(&mut self, display: DisplayRef, height: f32) -> bool {
        self.display_put_float(display, "height", height)
    }

    /// `Display.getGlowColorOverride`: an ARGB color, or -1 for none.
    pub(super) fn display_glow_color_override(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, None, 22)
    }

    /// `Display.setGlowColorOverride`.
    pub(super) fn display_set_glow_color_override(
        &mut self,
        display: DisplayRef,
        color: i32,
    ) -> bool {
        self.display_put(display, None, "glow_color_override", Tag::Int(color))
    }

    /// `Display.getTransformationInterpolationDuration`, in ticks.
    pub(super) fn display_interpolation_duration(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, None, 9)
    }

    /// `Display.setTransformationInterpolationDuration`.
    pub(super) fn display_set_interpolation_duration(
        &mut self,
        display: DisplayRef,
        ticks: i32,
    ) -> bool {
        self.display_put(display, None, "interpolation_duration", Tag::Int(ticks))
    }

    /// `Display.getTransformationInterpolationDelay`: ticks before an
    /// interpolation starts.
    pub(super) fn display_start_interpolation(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, None, 8)
    }

    /// `Display.setTransformationInterpolationDelay`. Like vanilla, every
    /// call is sent to the clients that track the display, even with the
    /// delay it already has, and each client restarts its interpolation.
    pub(super) fn display_set_start_interpolation(
        &mut self,
        display: DisplayRef,
        ticks: i32,
    ) -> bool {
        let Some((_, id)) = self.display_entity(display) else {
            return false;
        };
        // SynchedEntityData.set(DATA_..._START_DELTA_TICKS_ID, ticks, true).
        self.persist.force_meta(id, 8);
        self.display_put(display, None, "start_interpolation", Tag::Int(ticks))
    }

    /// `Display.getPosRotInterpolationDuration`: ticks a move eases over, 0 to 59.
    pub(super) fn display_teleport_duration(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, None, 10)
    }

    /// `Display.setPosRotInterpolationDuration`, kept from 0 to 59 as
    /// vanilla's load keeps it.
    pub(super) fn display_set_teleport_duration(
        &mut self,
        display: DisplayRef,
        ticks: i32,
    ) -> bool {
        let ticks = Tag::Int(ticks.clamp(0, 59));
        self.display_put(display, None, "teleport_duration", ticks)
    }

    /// `TextDisplay.getText`; None for another kind of display.
    pub(super) fn display_text(&self, display: DisplayRef) -> Option<Text> {
        match self.display_value(display, Some(DisplayKind::Text), 23)? {
            Value::Text(tag) => Some(comp_text(&tag_to_json(&tag))),
            _ => None,
        }
    }

    /// `TextDisplay.setText`; false for another kind of display.
    pub(super) fn display_set_text(&mut self, display: DisplayRef, text: Text) -> bool {
        let text = json_to_tag(&text.component());
        self.display_put(display, Some(DisplayKind::Text), "text", text)
    }

    /// `TextDisplay.getLineWidth`.
    pub(super) fn display_line_width(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, Some(DisplayKind::Text), 24)
    }

    /// `TextDisplay.setLineWidth`; false for another kind of display.
    pub(super) fn display_set_line_width(&mut self, display: DisplayRef, width: i32) -> bool {
        self.display_put(
            display,
            Some(DisplayKind::Text),
            "line_width",
            Tag::Int(width),
        )
    }

    /// `TextDisplay.getBackgroundColor`: ARGB.
    pub(super) fn display_background(&self, display: DisplayRef) -> Option<i32> {
        self.display_int(display, Some(DisplayKind::Text), 25)
    }

    /// `TextDisplay.setBackgroundColor`; false for another kind of display.
    pub(super) fn display_set_background(&mut self, display: DisplayRef, argb: i32) -> bool {
        self.display_put(
            display,
            Some(DisplayKind::Text),
            "background",
            Tag::Int(argb),
        )
    }

    /// `TextDisplay.getTextOpacity`: -1 is opaque, else 0 to 127 then -128 to -2.
    pub(super) fn display_text_opacity(&self, display: DisplayRef) -> Option<i8> {
        self.display_byte(display, DisplayKind::Text, 26)
    }

    /// `TextDisplay.setTextOpacity`; false for another kind of display.
    pub(super) fn display_set_text_opacity(&mut self, display: DisplayRef, opacity: i8) -> bool {
        self.display_put(
            display,
            Some(DisplayKind::Text),
            "text_opacity",
            Tag::Byte(opacity),
        )
    }

    fn display_text_flag(&self, display: DisplayRef, bit: i8) -> Option<bool> {
        Some(self.display_byte(display, DisplayKind::Text, 27)? & bit != 0)
    }

    fn display_set_text_flag(&mut self, display: DisplayRef, key: &str, on: bool) -> bool {
        let value = Tag::Byte(i8::from(on));
        self.display_put(display, Some(DisplayKind::Text), key, value)
    }

    /// `TextDisplay.FLAG_SHADOW`.
    pub(super) fn display_text_shadow(&self, display: DisplayRef) -> Option<bool> {
        self.display_text_flag(display, 1)
    }

    /// Sets the shadow flag; false for another kind of display.
    pub(super) fn display_set_text_shadow(&mut self, display: DisplayRef, on: bool) -> bool {
        self.display_set_text_flag(display, "shadow", on)
    }

    /// `TextDisplay.FLAG_SEE_THROUGH`.
    pub(super) fn display_see_through(&self, display: DisplayRef) -> Option<bool> {
        self.display_text_flag(display, 2)
    }

    /// Sets the see-through flag; false for another kind of display.
    pub(super) fn display_set_see_through(&mut self, display: DisplayRef, on: bool) -> bool {
        self.display_set_text_flag(display, "see_through", on)
    }

    /// `TextDisplay.FLAG_USE_DEFAULT_BACKGROUND`.
    pub(super) fn display_default_background(&self, display: DisplayRef) -> Option<bool> {
        self.display_text_flag(display, 4)
    }

    /// Sets the default-background flag; false for another kind of display.
    pub(super) fn display_set_default_background(&mut self, display: DisplayRef, on: bool) -> bool {
        self.display_set_text_flag(display, "default_background", on)
    }

    /// `TextDisplay.getAlign`, from the left and right flags.
    pub(super) fn display_alignment(&self, display: DisplayRef) -> Option<TextAlignment> {
        let flags = self.display_byte(display, DisplayKind::Text, 27)?;
        Some(match (flags & 8 != 0, flags & 16 != 0) {
            (true, _) => TextAlignment::Left,
            (_, true) => TextAlignment::Right,
            _ => TextAlignment::Center,
        })
    }

    /// Sets the alignment; false for another kind of display.
    pub(super) fn display_set_alignment(
        &mut self,
        display: DisplayRef,
        alignment: TextAlignment,
    ) -> bool {
        let name = Tag::String(alignment.name().into());
        self.display_put(display, Some(DisplayKind::Text), "alignment", name)
    }

    /// `BlockDisplay.getBlockState`; None for another kind of display.
    pub(super) fn display_block_state(&self, display: DisplayRef) -> Option<BlockStateId> {
        let (_, tag) = self.display_tag(display, Some(DisplayKind::Block))?;
        tag.get("block_state")
            .and_then(|t| self.state_from_nbt(t))
            .or_else(|| self.pack.assets.registry.default_state("minecraft:air"))
    }

    /// `BlockDisplay.setBlockState`; false for another kind of display or an unknown state.
    pub(super) fn display_set_block_state(
        &mut self,
        display: DisplayRef,
        state: BlockStateId,
    ) -> bool {
        if self.pack.assets.registry.state(state).is_none() {
            return false;
        }
        let tag = self.state_to_nbt(state);
        self.display_put(display, Some(DisplayKind::Block), "block_state", tag)
    }

    /// `ItemDisplay.getItemStack`; None for another kind of display or an empty stack.
    pub(super) fn display_item(&self, display: DisplayRef) -> Option<ItemStack> {
        let (_, tag) = self.display_tag(display, Some(DisplayKind::Item))?;
        item_from_nbt(tag.get("item")?)
    }

    /// `ItemDisplay.setItemStack`; None empties it. False for another kind of display.
    pub(super) fn display_set_item(
        &mut self,
        display: DisplayRef,
        stack: Option<ItemStack>,
    ) -> bool {
        let stack = stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        self.display_edit(display, Some(DisplayKind::Item), |c| match stack {
            Some(stack) => {
                c.insert("item".into(), horizon_client::save::item_to_nbt(&stack));
            }
            None => {
                c.remove(&"item".into());
            }
        })
    }

    /// `ItemDisplay.getItemTransform`: how the item is displayed.
    pub(super) fn display_item_context(&self, display: DisplayRef) -> Option<ItemDisplayContext> {
        let (_, tag) = self.display_tag(display, Some(DisplayKind::Item))?;
        let name = match tag.get("item_display") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        Some(ItemDisplayContext::from_name(&name).unwrap_or(ItemDisplayContext::None))
    }

    /// `ItemDisplay.setItemTransform`; false for another kind of display.
    pub(super) fn display_set_item_context(
        &mut self,
        display: DisplayRef,
        context: ItemDisplayContext,
    ) -> bool {
        let name = Tag::String(context.name().into());
        self.display_put(display, Some(DisplayKind::Item), "item_display", name)
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
    use super::*;
    use horizon_core::geometry::Vec3;
    use mod_api::WorldEpoch;

    fn push(game: &mut Game<'_>, id: u32, kind: &str) -> DisplayRef {
        game.options.dedicated = Some(Vec::new());
        game.others.push(OtherEntity {
            attachment_lifetime: Default::default(),
            knot_check: None,
            eye: None,
            join: Default::default(),
            item_transformation: None,
            interaction: None,
            id,
            kind: kind.into(),
            position: Vec3::new(1.0, 70.0, 1.0),
            yaw: 0.0,
            pitch: 0.0,
        });
        let entity = game.entity(id).unwrap();
        game.display(entity).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn displays_narrow_by_kind_and_share_the_display_state() {
        with_game(|game| {
            let text = push(game, 21, "minecraft:text_display");
            let block = push(game, 22, "minecraft:block_display");
            let item = push(game, 23, "minecraft:item_display");
            assert_eq!(text.kind(game), Some(DisplayKind::Text));
            assert_eq!(block.kind(game), Some(DisplayKind::Block));
            assert_eq!(item.kind(game), Some(DisplayKind::Item));
            assert_eq!(text.entity(), game.entity(21).unwrap());
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 24,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            assert!(game.display(game.entity(24).unwrap()).is_none());

            assert_eq!(text.transformation(game), Some(Transformation::IDENTITY));
            let moved = Transformation {
                translation: [0.0, 1.5, 0.0],
                left_rotation: [0.0, 1.0, 0.0, 0.0],
                scale: [2.0, 2.0, 2.0],
                right_rotation: [0.0, 0.0, 0.0, 1.0],
            };
            assert!(text.set_transformation(game, moved));
            assert_eq!(text.transformation(game), Some(moved));
            let mut bad = moved;
            bad.scale[1] = f32::NAN;
            assert!(!text.set_transformation(game, bad));
            bad = moved;
            bad.left_rotation = [0.0; 4];
            assert!(!text.set_transformation(game, bad));
            assert_eq!(text.transformation(game), Some(moved));

            assert_eq!(item.billboard(game), Some(Billboard::Fixed));
            assert!(item.set_billboard(game, Billboard::Center));
            assert_eq!(item.billboard(game), Some(Billboard::Center));
            assert_eq!(block.brightness(game), None);
            assert!(block.set_brightness(game, Some((15, 3))));
            assert_eq!(block.brightness(game), Some((15, 3)));
            assert!(!block.set_brightness(game, Some((16, 0))));
            assert!(block.set_brightness(game, None));
            assert_eq!(block.brightness(game), None);
            assert_eq!(block.view_range(game), Some(1.0));
            assert!(block.set_view_range(game, 4.0));
            assert!(!block.set_view_range(game, f32::INFINITY));
            assert_eq!(block.view_range(game), Some(4.0));
            assert!(block.set_shadow_radius(game, 0.5));
            assert!(block.set_shadow_strength(game, 0.25));
            assert!(block.set_width(game, 2.0));
            assert!(block.set_height(game, 3.0));
            assert!(block.set_glow_color_override(game, 0xff00ff));
            assert_eq!(
                (
                    block.shadow_radius(game),
                    block.shadow_strength(game),
                    block.width(game),
                    block.height(game),
                    block.glow_color_override(game)
                ),
                (Some(0.5), Some(0.25), Some(2.0), Some(3.0), Some(0xff00ff))
            );
            assert!(item.set_interpolation_duration(game, 8));
            assert!(item.set_start_interpolation(game, -1));
            assert!(item.set_teleport_duration(game, 99));
            assert_eq!(
                (
                    item.interpolation_duration(game),
                    item.start_interpolation(game),
                    item.teleport_duration(game)
                ),
                (Some(8), Some(-1), Some(59))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn display_kinds_keep_their_own_state_and_refuse_the_others() {
        with_game(|game| {
            let text = push(game, 21, "minecraft:text_display");
            let block = push(game, 22, "minecraft:block_display");
            let item = push(game, 23, "minecraft:item_display");
            assert_eq!(text.line_width(game), Some(200));
            assert_eq!(text.background(game), Some(0x4000_0000));
            assert_eq!(text.text_opacity(game), Some(-1));
            assert_eq!(text.alignment(game), Some(TextAlignment::Center));
            assert!(
                text.set_text(game, mod_api::Text::literal("hello"))
                    .unwrap()
            );
            assert!(text.set_line_width(game, 80));
            assert!(text.set_background(game, 0x20ff0000));
            assert!(text.set_text_opacity(game, 100));
            assert!(text.set_text_shadow(game, true));
            assert!(text.set_see_through(game, true));
            assert!(text.set_default_background(game, true));
            assert!(text.set_alignment(game, TextAlignment::Right));
            assert_eq!(
                text.text(game).unwrap().unwrap(),
                mod_api::Text::literal("hello")
            );
            assert_eq!(
                (
                    text.line_width(game),
                    text.background(game),
                    text.text_opacity(game),
                    text.text_shadow(game),
                    text.see_through(game),
                    text.default_background(game),
                    text.alignment(game)
                ),
                (
                    Some(80),
                    Some(0x20ff0000),
                    Some(100),
                    Some(true),
                    Some(true),
                    Some(true),
                    Some(TextAlignment::Right)
                )
            );
            assert!(text.set_alignment(game, TextAlignment::Left));
            assert!(text.set_text_shadow(game, false));
            assert_eq!(text.alignment(game), Some(TextAlignment::Left));
            assert_eq!(text.text_shadow(game), Some(false));
            assert_eq!(text.see_through(game), Some(true));

            let stone = game
                .block_state_id("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            let stairs = game
                .block_state_id("minecraft:oak_stairs[facing=east]")
                .unwrap()
                .try_into_api()
                .unwrap();
            let air = game
                .block_state_id("minecraft:air")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert_eq!(block.block_state(game), Some(air));
            assert!(block.set_block_state(game, stone));
            assert_eq!(block.block_state(game), Some(stone));
            assert!(block.set_block_state(game, stairs));
            assert_eq!(block.block_state(game), Some(stairs));
            let invalid = mod_api::BlockStateRecord::new(stairs.block(), u16::MAX).into();
            assert!(!block.set_block_state(game, invalid));
            assert_eq!(block.block_state(game), Some(stairs));

            let sword = ItemStack {
                item: "minecraft:diamond_sword".into(),
                count: 1,
                damage: 3,
                extra: None,
            };
            assert_eq!(item.item(game), None);
            assert_eq!(item.item_context(game), Some(ItemDisplayContext::None));
            let sword = sword.try_into_api().unwrap();
            assert_eq!(item.set_item(game, Some(sword.clone())), Ok(true));
            assert!(item.set_item_context(game, ItemDisplayContext::Head));
            assert_eq!(item.item(game), Some(sword));
            assert_eq!(item.item_context(game), Some(ItemDisplayContext::Head));
            assert_eq!(item.set_item(game, None), Ok(true));
            assert_eq!(item.item(game), None);

            // The wrong kind reads None and writes nothing.
            assert_eq!(block.text(game).unwrap(), None);
            assert!(!block.set_text(game, mod_api::Text::literal("x")).unwrap());
            assert!(!text.set_block_state(game, stone));
            assert_eq!(text.block_state(game), None);
            assert!(!block.set_item_context(game, ItemDisplayContext::Gui));
            assert!(!item.set_alignment(game, TextAlignment::Left));
            assert_eq!(item.line_width(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn display_writes_reach_entity_data_and_the_saved_entity() {
        with_game(|game| {
            let text = push(game, 21, "minecraft:text_display");
            assert!(
                text.set_text(game, mod_api::Text::literal("saved"))
                    .unwrap()
            );
            assert!(text.set_billboard(game, Billboard::Vertical));
            assert!(text.set_view_range(game, 2.5));
            let raw = game.persist.raw.get(&21).unwrap().clone();
            // The entity data the tracker sends: the changed entries only.
            let data = display_data(&raw, false);
            assert!(data.iter().any(|e| e.0 == 15 && e.2 == Value::Byte(1)));
            assert!(data.iter().any(|e| e.0 == 17 && e.2 == Value::Float(2.5)));
            assert!(text_display_data(&raw, false).iter().any(|e| e.0 == 23));
            // The saved entity is the compound `/data get entity` shows.
            let o = game.others.iter().find(|o| o.id == 21).unwrap();
            let saved = game.other_to_nbt(o, [1, 2, 3, 4]);
            assert_eq!(
                saved.get("billboard"),
                Some(&Tag::String("vertical".into()))
            );
            assert_eq!(saved.get("view_range"), Some(&Tag::Float(2.5f32.to_bits())));
            assert_eq!(
                comp_text(&tag_to_json(saved.get("text").unwrap())).plain(),
                "saved"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn each_start_delay_write_forces_a_send_even_when_equal() {
        with_game(|game| {
            let text = push(game, 21, "minecraft:text_display");
            let other = push(game, 22, "minecraft:item_display");
            assert!(game.persist.forced_meta.is_empty());
            for _ in 0..2 {
                assert!(text.set_start_interpolation(game, 0));
                assert_eq!(text.start_interpolation(game), Some(0));
                assert_eq!(game.persist.forced_meta.get(&21), Some(&[1 << 8, 0, 0, 0]));
                // The tracking pass spends the force; the next write sets it again.
                game.spend_forced_meta();
            }
            // Other writes, and writes that fail, force nothing.
            assert!(other.set_view_range(game, 3.0));
            assert!(game.persist.forced_meta.is_empty());
            assert!(text.entity().discard(game));
            assert!(!text.set_start_interpolation(game, 0));
            assert!(game.persist.forced_meta.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stale_removed_and_client_displays_do_nothing() {
        with_game(|game| {
            let text = push(game, 21, "minecraft:text_display");
            let stale = DisplayRef::new(EntityRef::new(
                WorldEpoch::new(text.entity().epoch().get() + 1),
                21,
            ));
            assert_eq!(stale.kind(game), None);
            assert_eq!(stale.view_range(game), None);
            assert!(!stale.set_view_range(game, 2.0));
            assert!(!stale.set_text(game, mod_api::Text::literal("x")).unwrap());
            assert!(text.entity().discard(game));
            assert_eq!(text.kind(game), None);
            assert!(!text.set_width(game, 1.0));
        });
    }
}
