use super::*;

pub(super) struct Mod;
impl Hooks for Mod {
    fn entity_interact(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        use_item_hand::entity_answer(event)
    }
    fn datapack_sync(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DatapackSyncEvent<'_>,
    ) {
        if event.player.uuid() == 12 {
            event.recipe_types.push("testmod:grinding".into());
        }
    }

    fn loot_function(&self, event: &mut mod_api::LootFunctionEvent<'_>) {
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let members = event.tags.items(tag).unwrap();
        assert_eq!(members, vec![event.stack.item]);
        assert_eq!(event.tags.is_in_tag(event.stack.item, tag), Some(true));
        event.stack.count = 29;
    }
    fn loot_condition(&self, event: &mut mod_api::LootConditionEvent<'_>) {
        if loot_query::guest_condition(event) {
            return;
        }
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let item = event.tags.items(tag).unwrap()[0];
        event.holds = event.tags.is_in_tag(item, tag).unwrap();
    }
    fn register_load_conditions(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterLoadConditionsEvent<'_>,
    ) {
        load_conditions::register(event.conditions);
    }
    fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        entity_renderers::register(event.renderers);
    }
    fn register_entity_layers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityLayersEvent<'_>,
    ) {
        entity_layers::register(event.layers);
    }
    fn register_special_model_renderer(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterSpecialModelRendererEvent<'_>,
    ) {
        special_model_renderers::register(event.renderers);
    }
    fn register(&self, registrar: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {
        cx.warn("wire_test.warn.optional", &["café", "jei"]);
        arm_pose_registration::check(registrar);
        biome_region_registration::register(registrar);
        REGISTERED_ITEM.set(Some(registrar.item_key("minecraft:stone")));
        prepared_equip::register(registrar);
        use_item_hand::register(registrar);
        piglin_answers::register(registrar);
        block_entity_transform::register(registrar);
        crash_sections::register(registrar);
        REGISTERED_BLOCK.set(Some(registrar.block_key("minecraft:stone")));
        REGISTERED_EFFECT.set(Some(registrar.effect_key("minecraft:poison")));
        let kind = registrar.block_entity_type("counter").blocks(&[]).build();
        REGISTERED_KIND.set(Some(kind));
        registrar.block_entity_part(&kind, MachineBlock).unwrap();
    }
    fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::RenderGuiPre)]
    }
    fn listeners(&self) -> &'static [mod_api::Listener] {
        &[mod_api::Listener {
            hook: Some(mod_api::HookId::RenderGuiPre),
            priority: -37,
            receive_cancelled: true,
        }]
    }
    fn register_json_animation_types(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
    ) {
        assert!(
            event.types.is_empty(),
            "the guest starts from its own batch"
        );
        for (target, id) in [(true, "wire_test:pulse"), (false, "wire_test:step")] {
            let id = mod_api::Identifier::parse(id).unwrap();
            event.types.push(if target {
                mod_api::JsonAnimationType::Target(id)
            } else {
                mod_api::JsonAnimationType::Interpolation(id)
            });
        }
    }
    fn render_gui_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        assert!(event.cancelled);
        event.cancelled = false;
        mod_api::Flow::Continue
    }
    fn item_tooltip(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        let mask = u32::from(event.display.name_visible(false))
            | (u32::from(event.display.name_visible(true)) << 1)
            | (u32::from(event.display.shows("minecraft:banner_patterns")) << 2)
            | (u32::from(event.display.shows("test:hidden")) << 3);
        event
            .lines
            .push((mod_api::Text::literal("projection"), mask));
    }
    fn apply_effect_tick(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ApplyEffectTickEvent<'_>,
    ) {
        event.heal(2.5);
        event.hurt("minecraft:wither", 3.);
        event.eat(2, 0.25);
        event.add_exhaustion(0.5);
        event.keep = false;
    }
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        assert!(event.hidden.contains_named("nose"));
        event.hidden.hide_named("hat_rim").unwrap();
        event.hidden.hide(mod_api::HumanoidPart::Head);
        mod_api::Flow::Continue
    }
    fn render_player_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderPlayerPreEvent<'_>,
    ) -> mod_api::Flow {
        // This window's own player is drawn by the mod: every other one isn't.
        assert_eq!(event.player.kind, "minecraft:player");
        event.cancelled = event.who.local;
        event.draw.set_light(event.who.name.len() as u32);
        mod_api::Flow::Continue
    }
    fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderLivingPostEvent<'_>,
    ) {
        assert_eq!(event.entity.kind, "test:mob");
        assert_eq!(event.entity.custom_name, Some("custom"));
        event
            .draw
            .set_light(if event.parts.is_some() { 9 } else { 7 });
    }
    fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderPlayerPostEvent<'_>,
    ) {
        assert_eq!((event.who.name, event.who.local), ("test:player", true));
        event.draw.set_light(42);
    }
    fn item_picked_up_post(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemPickedUpPostEvent<'_>,
    ) {
        PICKED_UP_POST.set(Some((
            event.player,
            event.item.entity().id(),
            event.original.count(),
            event.remaining.as_ref().map(|rest| rest.count()),
        )));
        let mut rest = event.original.clone();
        rest.set_count(3).unwrap();
        *event.remaining = Some(rest);
    }
    fn item_picked_up_partial(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemPickedUpPartialEvent<'_>,
    ) {
        PICKED_UP_PARTIAL.set(Some((
            event.player,
            event.original.count(),
            event.remaining.as_ref().map(|rest| rest.count()),
        )));
        *event.remaining = None;
    }
    fn viewed_level_sound(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::ViewedLevelSoundEvent<'_>,
    ) {
        viewed_level_sound::listen(event);
    }
    fn item_toss(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemTossEvent,
    ) -> mod_api::Flow {
        TOSSED.set(Some((event.player, event.item.entity().id())));
        mod_api::Flow::Cancel
    }
    fn game_rule_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::GameRuleChangedEvent<'_>,
    ) {
        assert_eq!(event.rule, "test:rule");
        if let Some(actor) = event.actor {
            assert_eq!(
                (actor.uuid, actor.entity_id, actor.name),
                (123, 45, "test:actor")
            );
            event.value = mod_api::GameRuleValue::Bool(true);
        } else {
            event.value = mod_api::GameRuleValue::Bool(false);
        }
    }
    fn container_loot_filled(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::ContainerLootFilledEvent<'_>,
    ) {
        container_loot_filled::listen(event);
    }
    fn item_fished(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemFishedEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.fished.hook, 19);
        assert_eq!(event.drops.len(), 1);
        event.drops.clear();
        event
            .drops
            .push(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 2).unwrap());
        event.rod_damage = 5;
        mod_api::Flow::Continue
    }
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_>,
    ) {
        assert_eq!(event.context.entity, Some((12, "test:mob")));
        assert_eq!(event.context.table.len(), 300);
        let source = event.context.damage_source.unwrap();
        assert_eq!(
            (source.damage_type, source.causing, source.direct),
            ("minecraft:arrow", 41, 73)
        );
        assert_eq!(source.position, Some([1.0, 2.0, 3.0]));
        assert!(event.context.tool.is_none());
        EXPECTED_LOOT_HAND.with(|expected| {
            assert_eq!(
                event.context.weapon_item(host),
                expected.borrow().clone(),
                "the recorded weapon crosses the generated hook boundary"
            );
            assert_eq!(
                host.entity(source.causing as u32)
                    .and_then(|e| host.living(e))
                    .and_then(|living| living.hand_item(host, mod_api::Hand::Main)),
                expected.borrow().clone(),
                "the complete live hand crosses the generated Host boundary"
            )
        });
        event.drops.clear();
    }
    fn living_drops(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDropsEvent<'_>,
    ) -> mod_api::Flow {
        if event.kind != "test:state_drops" {
            return mod_api::Flow::Continue;
        }
        use item_drop_state::{TARGET, THROWER, state_drop};
        let full = state_drop(Some(THROWER), Some(TARGET), true, -32768);
        let plain = state_drop(None, None, false, 0);
        assert_eq!(*event.drops, vec![plain.clone(), full.clone()]);
        *event.drops = vec![full, plain];
        mod_api::Flow::Continue
    }
    fn item_stacked_on_other(
        &self,
        _: &dyn LoopContext,
        event: &mut mod_api::ItemStackedOnOtherEvent<'_, '_>,
    ) -> mod_api::Flow {
        *event.event.carried.0 = None;
        event.event.cancellation_result = true;
        mod_api::Flow::Continue
    }
    fn living_jump_power(
        &self,
        context: &dyn LoopContext,
        event: &mut mod_api::LivingJumpPowerEvent<'_>,
    ) {
        // This is a plain import made through the generated Guest, not a stubbed reply.
        assert!(matches!(context.side(), mod_api::LoopSide::Server));
        event.entity = 99;
        event.power = 7.25;
        event.kind = "must not write back";
    }
}
