//! The merchant (villager trading) menu: `MerchantMenu` / `MerchantScreen`
//! layout (276x166, drawn 50 px left of the usual 176-wide origin), offer
//! buttons, payment slots, the result slot's `onTake`, villager experience and
//! level-ups with new offers from the next trade set.
use super::{
    Game,
    stations::{INPUT, Kind, RESULT, Station},
};
use horizon_client::mobs::{
    Species,
    render::{PROFESSIONS, VILLAGER_TYPES},
    villager,
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
    merchant::{self, MerchantOffer, TradeContext},
};
use horizon_core::random::RandomSource;

/// The merchant screen is 100 px wider than the 176-px menus it shares an origin with.
const SHIFT: f32 = -50.0;
const GREY: u32 = 0xFF40_4040;

pub(super) struct MerchantState {
    pub(super) profession: String,
    pub(super) variant: String,
    pub(super) level: i32,
    pub(super) xp: i32,
    pub(super) offers: Vec<MerchantOffer>,
    /// `shopItem` / selection hint, scroll offset, and the active offer.
    pub(super) shop_item: usize,
    pub(super) scroll: usize,
    pub(super) active: Option<usize>,
    pub(super) future_xp: i32,
    /// The offer button that keeps focus after a click (drawn highlighted).
    pub(super) focused: Option<usize>,
    /// The trading mob (its offers and experience are written back on
    /// close) and its custom name.
    pub(super) mob: Option<u32>,
    pub(super) name: Option<String>,
}
impl MerchantState {
    /// `Merchant.showProgressBar`: villagers only.
    pub(super) fn progress(&self) -> bool {
        self.profession != "wandering_trader"
    }
}
/// Player inventory slot positions in `MerchantMenu` (x from 108).
pub(super) fn inventory_slot(i: usize) -> (f32, f32) {
    match i {
        0..9 => (108.0 + SHIFT + i as f32 * 18.0, 142.0),
        _ => (
            108.0 + SHIFT + ((i - 9) % 9) as f32 * 18.0,
            84.0 + ((i - 9) / 9) as f32 * 18.0,
        ),
    }
}
/// `MerchantContainer.updateSellItem`.
pub(super) fn update(s: &mut Station, items: &horizon_client::items::Items) {
    let Some(m) = s.merchant.as_mut() else { return };
    let (active, result) = merchant::sell_item(
        &m.offers,
        items,
        s.inputs[0].as_ref(),
        s.inputs[1].as_ref(),
        Some(m.shop_item),
    );
    m.active = active;
    m.future_xp = if result.is_some() {
        active.map_or(0, |i| m.offers[i].xp)
    } else {
        0
    };
    s.result = result;
}

impl Game<'_> {
    /// Offers a villager of `profession` gets on reaching `level`.
    fn new_offers(&mut self, profession: &str, variant: &str, level: i32) -> Vec<MerchantOffer> {
        let set = merchant::Trades::set_for(profession, level);
        self.offers_from_set(&set, variant)
    }
    /// `AbstractVillager.addOffersFromTradeSet`.
    fn offers_from_set(&mut self, set: &str, variant: &str) -> Vec<MerchantOffer> {
        let sequence = format!(
            "minecraft:trade_set/{}",
            set.trim_start_matches("minecraft:")
        );
        let random = self
            .potions
            .trade_randoms
            .entry(sequence.clone())
            .or_insert_with(|| merchant::sequence_random(self.level.seed, &sequence));
        let ctx = TradeContext {
            items: &self.res.items,
            enchantments: &self.res.stations.enchantments,
            variant,
        };
        self.res.stations.trades.offers_from_set(&ctx, set, random)
    }
    /// `Villager.mobInteract` / `WanderingTrader.mobInteract`: shake the
    /// head without offers, else open the trading screen.
    pub(super) fn interact_trader(
        &mut self,
        index: usize,
        item: &str,
    ) -> horizon_client::Result<bool> {
        let m = &self.mobs[index];
        let species = m.species();
        if !matches!(species, Species::Villager | Species::WanderingTrader)
            || !m.alive()
            || item == "minecraft:villager_spawn_egg"
            || self.station.is_some()
        {
            return Ok(false);
        }
        if m.is_baby() {
            let mut events = Vec::new();
            villager::set_unhappy(&mut self.mobs[index], &mut events);
            self.handle_mob_events(events)?;
            return Ok(true);
        }
        // AbstractVillager.getOffers: generated on first use.
        if self.mobs[index].extra.trade.is_none() {
            let m = &self.mobs[index];
            let variant = VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()];
            let offers = if species == Species::WanderingTrader {
                let mut v = Vec::new();
                for set in ["buying", "uncommon", "common"] {
                    v.extend(
                        self.offers_from_set(&format!("minecraft:wandering_trader/{set}"), variant),
                    );
                }
                v
            } else {
                let profession = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
                let level = i32::from(m.extra.variant3.max(1));
                if matches!(profession, "none" | "nitwit") {
                    Vec::new()
                } else {
                    self.new_offers(profession, variant, level)
                }
            };
            self.mobs[index].extra.trade = Some(Box::new(merchant::Trader {
                offers,
                ..Default::default()
            }));
        }
        let m = &self.mobs[index];
        let mut trade = m.extra.trade.as_deref().cloned().unwrap_or_default();
        // Villager.updateSpecialPrices: reputation and Hero of the Village.
        if species == Species::Villager {
            let reputation = self.villager_reputation(m.id);
            merchant::update_special_prices(&mut trade.offers, reputation, self.hero_amplifier());
        }
        let m = &self.mobs[index];
        if trade.offers.is_empty() {
            let mut events = Vec::new();
            villager::set_unhappy(&mut self.mobs[index], &mut events);
            self.handle_mob_events(events)?;
            return Ok(true);
        }
        let profession = if species == Species::WanderingTrader {
            "wandering_trader"
        } else {
            PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()]
        };
        let state = MerchantState {
            profession: profession.to_owned(),
            variant: VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()].to_owned(),
            level: i32::from(m.extra.variant3.max(1)).min(5),
            xp: trade.xp,
            offers: trade.offers,
            shop_item: 0,
            scroll: 0,
            active: None,
            future_xp: 0,
            focused: None,
            mob: Some(m.id),
            name: m.custom_name.clone(),
        };
        self.open_merchant_state(state);
        Ok(true)
    }
    /// `Villager.setTradingPlayer(null)`: the menu's offers and experience
    /// go back to the mob.
    pub(super) fn merchant_closed(&mut self, state: &MerchantState) {
        let Some(id) = state.mob else { return };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if state.progress() {
            m.extra.variant3 = state.level.clamp(1, 5) as u8;
        }
        let trade = m.extra.trade.get_or_insert_default();
        trade.xp = state.xp;
        trade.offers = state.offers.clone();
        // setTradingPlayer(null) resets the special prices.
        for offer in &mut trade.offers {
            offer.special_price_diff = 0;
        }
    }
    /// Opens a merchant menu for a villager (or a generated one for tests).
    pub(super) fn open_merchant(&mut self, profession: &str, variant: &str, level: i32) {
        let mut offers = Vec::new();
        for l in 1..=level.clamp(1, 5) {
            offers.extend(self.new_offers(profession, variant, l));
        }
        self.open_merchant_state(MerchantState {
            profession: profession.trim_start_matches("minecraft:").to_owned(),
            variant: variant.to_owned(),
            level: level.clamp(1, 5),
            xp: merchant::min_xp(level.clamp(1, 5)),
            offers,
            shop_item: 0,
            scroll: 0,
            active: None,
            future_xp: 0,
            focused: None,
            mob: None,
            name: None,
        });
    }
    fn open_merchant_state(&mut self, state: MerchantState) {
        let mut s = Station::new_kind(Kind::Merchant, [0, 0, 0]);
        s.merchant = Some(Box::new(state));
        self.station = Some(s);
        self.screen = super::Screen::Inventory;
        self.capture(false);
        self.right = false;
    }
    /// `MerchantResultSlot.onTake` and `Villager.notifyTrade` / `rewardTradeXp`.
    pub(super) fn merchant_take(&mut self, s: &mut Station) {
        let items = &self.res.items;
        let Some(m) = s.merchant.as_mut() else { return };
        let Some(i) = m.active else { return };
        let offer = m.offers[i].clone();
        let (mut a, mut b) = (s.inputs[0].take(), s.inputs[1].take());
        let took = offer.take(items, &mut a, &mut b) || offer.take(items, &mut b, &mut a);
        s.inputs[0] = a;
        s.inputs[1] = b;
        if !took {
            return;
        }
        m.offers[i].uses += 1;
        let mut pop = 3 + self.world.random.next_int_bound(4).unwrap_or(0);
        m.xp += offer.xp;
        let level_up =
            m.progress() && merchant::can_level_up(m.level) && m.xp >= merchant::max_xp(m.level);
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y + 0.5, p.position.z]);
        self.audio
            .play("minecraft:entity.villager.yes", at, 1.0, 1.0);
        // Villager.notifyTrade: the villager remembers the trade.
        let villager = s
            .merchant
            .as_ref()
            .filter(|m| m.progress())
            .and_then(|m| m.mob);
        if let Some(mob) = villager {
            self.villager_traded(mob);
        }
        let Some(m) = s.merchant.as_mut() else { return };
        if level_up {
            let (profession, variant, level) =
                (m.profession.clone(), m.variant.clone(), m.level + 1);
            m.level = level;
            let more = self.new_offers(&profession, &variant, level);
            if let Some(m) = s.merchant.as_mut() {
                m.offers.extend(more);
            }
            pop += 5;
        }
        // The prices follow the villager's new reputation.
        if let Some(mob) = villager {
            let reputation = self.villager_reputation(mob);
            let hero = self.hero_amplifier();
            if let Some(m) = s.merchant.as_mut() {
                merchant::update_special_prices(&mut m.offers, reputation, hero);
            }
        }
        if offer.reward_exp
            && let Some(at) = at
        {
            self.spawn_orbs(at, pop as u32);
        }
    }
    /// `MerchantMenu.tryMoveItems`: return payment, then fill it for the selected offer.
    fn try_move_items(&mut self, index: usize) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let returned: Vec<ItemStack> = s.inputs.iter_mut().filter_map(Option::take).collect();
        for stack in returned {
            if let Some(rest) = self.give(stack)
                && let Some(eye) = self.eye_position()
            {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
        }
        let Some(offer) = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .and_then(|m| m.offers.get(index))
            .cloned()
        else {
            return;
        };
        let wants = [Some(offer.cost_a(&self.res.items)), offer.cost_b()];
        for (slot, want) in wants.into_iter().enumerate() {
            let Some(want) = want else { continue };
            let max = self.res.items.get(&want.item).map_or(64, |i| i.max_stack);
            let mut gathered: Option<ItemStack> = None;
            for i in (9..36).chain(0..9) {
                let Some(s) = self.inventory[i].as_mut() else {
                    continue;
                };
                if !s.same_item_same_components(&ItemStack {
                    count: s.count,
                    ..want.clone()
                }) {
                    continue;
                }
                let have = gathered.as_ref().map_or(0, |g| g.count);
                let n = s.count.min(max - have);
                if n == 0 {
                    break;
                }
                s.count -= n;
                let taken = ItemStack {
                    count: n,
                    ..s.clone()
                };
                if s.count == 0 {
                    self.inventory[i] = None;
                }
                match gathered.as_mut() {
                    Some(g) => g.count += n,
                    None => gathered = Some(taken),
                }
            }
            if let Some(st) = self.station.as_mut() {
                st.inputs[slot] = gathered;
            }
        }
    }
    /// Clicks on the offer list; true when one was hit.
    pub(super) fn merchant_click(&mut self, local: (f32, f32)) -> bool {
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return false;
        };
        for i in 0..7 {
            let (x, y) = (5.0 + SHIFT, 18.0 + 20.0 * i as f32);
            if local.0 >= x
                && local.0 < x + 88.0
                && local.1 >= y
                && local.1 < y + 20.0
                && i + m.scroll < m.offers.len()
            {
                let index = i + m.scroll;
                if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
                    m.shop_item = index;
                    m.focused = Some(i);
                }
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                self.try_move_items(index);
                self.station_changed();
                return true;
            }
        }
        false
    }
    pub(super) fn merchant_scroll(&mut self, d: f32) {
        let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) else {
            return;
        };
        if m.offers.len() > 7 {
            let max = m.offers.len() - 7;
            m.scroll = (m.scroll as i32 - d.signum() as i32).clamp(0, max as i32) as usize;
        }
    }
    /// `MerchantScreen` rendering.
    pub(super) fn draw_merchant(&mut self, batch: &mut HudBatch) -> bool {
        let (ox, oy) = self.inventory_origin(batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return false;
        };
        let (progress, custom) = (m.progress(), m.name.clone());
        let (profession, level, xp, scroll, shop, future, focused) = (
            m.profession.clone(),
            m.level,
            m.xp,
            m.scroll,
            m.shop_item,
            m.future_xp,
            m.focused,
        );
        let offers = m.offers.clone();
        let costs: Vec<ItemStack> = offers.iter().map(|o| o.cost_a(&self.res.items)).collect();
        let atlas = &self.res.atlas;
        batch.fill(
            atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(16, 16, 16, 160),
        );
        if let Some(mut r) = atlas.sprites.get("gui/container/villager").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 276.0 * du;
            r.v1 = r.v0 + 166.0 * dv;
            batch.rect_uv(ox + SHIFT, oy, 276.0, 166.0, r, u32::MAX);
        }
        let e = &self.res.stations.enchantments;
        let tr = |k: &str, f: &str| e.translate(k).unwrap_or(f).to_owned();
        let name = custom.unwrap_or_else(|| {
            if progress {
                tr(
                    &format!("entity.minecraft.villager.{profession}"),
                    &profession,
                )
            } else {
                tr("entity.minecraft.wandering_trader", "Wandering Trader")
            }
        });
        let title = if progress {
            tr("merchant.title", "%s - %s")
                .replacen("%s", &name, 1)
                .replacen("%s", &tr(&format!("merchant.level.{level}"), ""), 1)
        } else {
            name
        };
        let tw = HudBatch::text_width(atlas, &title);
        batch.text(
            atlas,
            &title,
            ox + SHIFT + (49.0 + 138.0 - (tw / 2.0).floor()),
            oy + 6.0,
            GREY,
            false,
        );
        let trades = tr("merchant.trades", "Trades");
        let tw = HudBatch::text_width(atlas, &trades);
        batch.text(
            atlas,
            &trades,
            ox + SHIFT + 5.0 - (tw / 2.0).floor() + 48.0,
            oy + 6.0,
            GREY,
            false,
        );
        batch.text(
            atlas,
            &tr("container.inventory", "Inventory"),
            ox + SHIFT + 107.0,
            oy + 72.0,
            GREY,
            false,
        );
        if offers.get(shop).is_some_and(MerchantOffer::out_of_stock) {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/out_of_stock",
                ox + SHIFT + 182.0,
                oy + 35.0,
            );
        }
        // Experience bar (levels 1-4).
        if progress && level < 5 {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/experience_bar_background",
                ox + SHIFT + 136.0,
                oy + 16.0,
            );
            let min = merchant::min_xp(level);
            if xp >= min && merchant::can_level_up(level) {
                let mult = 102.0 / (merchant::max_xp(level) - min) as f32;
                let w = ((mult * (xp - min) as f32).floor()).min(102.0);
                batch.sprite_part(
                    atlas,
                    "gui/sprites/container/villager/experience_bar_current",
                    ox + SHIFT + 136.0,
                    oy + 16.0,
                    w / 102.0,
                );
                if future > 0 {
                    let fw = (future as f32 * mult).floor().min(102.0 - w);
                    if let Some(mut r) = atlas
                        .sprites
                        .get("gui/sprites/container/villager/experience_bar_result")
                        .copied()
                    {
                        let du = (r.u1 - r.u0) / r.w as f32;
                        r.u0 += w * du;
                        r.u1 = r.u0 + fw * du;
                        batch.rect_uv(ox + SHIFT + 136.0 + w, oy + 16.0, fw, 5.0, r, u32::MAX);
                    }
                }
            }
        }
        // Scroller.
        let steps = offers.len() as i32 + 1 - 7;
        if steps > 1 {
            let left = 139 - (27 + (steps - 1) * 139 / steps);
            let step = 1 + left / steps + 139 / steps;
            let mut y = (scroll as i32 * step).min(113);
            if scroll as i32 == steps - 1 {
                y = 113;
            }
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/scroller",
                ox + SHIFT + 94.0,
                oy + 18.0 + y as f32,
            );
        } else {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/scroller_disabled",
                ox + SHIFT + 94.0,
                oy + 18.0,
            );
        }
        // Offer buttons.
        let mut tooltip: Option<ItemStack> = None;
        for (row, (index, offer)) in offers.iter().enumerate().skip(scroll).take(7).enumerate() {
            let (bx, by) = (ox + SHIFT + 5.0, oy + 18.0 + 20.0 * row as f32);
            let hover = local.0 >= 5.0 + SHIFT
                && local.0 < 93.0 + SHIFT
                && local.1 >= 18.0 + 20.0 * row as f32
                && local.1 < 38.0 + 20.0 * row as f32;
            let sprite = if hover || focused == Some(row) {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            batch.sprite_scaled(&self.res.atlas, sprite, bx, by, 88.0, 20.0, u32::MAX);
            let y = oy + 16.0 + 1.0 + 20.0 * row as f32 + 2.0;
            let base = offer.base_cost_a.stack.clone();
            let cost_a = costs[index].clone();
            let x = ox + SHIFT + 10.0;
            if base.count == cost_a.count {
                self.draw_item(batch, &cost_a, x, y);
            } else {
                // Discounted: the base price struck through, then the new price.
                self.draw_item(
                    batch,
                    &ItemStack {
                        count: 1,
                        ..cost_a.clone()
                    },
                    x,
                    y,
                );
                let atlas = &self.res.atlas;
                let t = base.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                let t = cost_a.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 14.0 + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                batch.sprite(
                    atlas,
                    "gui/sprites/container/villager/discount_strikethrough",
                    x + 7.0,
                    y + 12.0,
                );
            }
            if let Some(b) = offer.cost_b() {
                self.draw_item(batch, &b, ox + SHIFT + 40.0, y);
            }
            let arrow = if offer.out_of_stock() {
                "trade_arrow_out_of_stock"
            } else {
                "trade_arrow"
            };
            batch.sprite(
                &self.res.atlas,
                &format!("gui/sprites/container/villager/{arrow}"),
                ox + SHIFT + 60.0,
                y + 3.0,
            );
            self.draw_item(batch, &offer.result, ox + SHIFT + 73.0, y);
            if hover && self.carried.is_none() {
                let rel = local.0 - (5.0 + SHIFT);
                tooltip = if rel < 20.0 {
                    Some(cost_a)
                } else if rel > 30.0 && rel < 50.0 {
                    offer.cost_b()
                } else if rel > 65.0 {
                    Some(offer.result.clone())
                } else {
                    None
                };
            }
        }
        // Slots, hover highlight, carried stack, tooltip.
        let slots: Vec<usize> = (0..36).chain([INPUT, INPUT + 1, RESULT]).collect();
        for i in slots {
            let (sx, sy) = self.slot_position(i);
            if let Some(stack) = self.slot_mut(i).clone() {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(
                &self.res.atlas,
                ox + sx,
                oy + sy,
                16.0,
                16.0,
                rgba(255, 255, 255, 128),
            );
            if self.carried.is_none() && tooltip.is_none() {
                tooltip = self.slot_mut(slot).clone();
            }
        }
        if let Some(stack) = self.carried.clone() {
            self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
        } else if let Some(stack) = tooltip {
            let lines = self.stack_tooltip(&stack);
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
        true
    }
}
