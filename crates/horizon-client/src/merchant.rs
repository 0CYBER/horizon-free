//! Villager trading: the data-driven `trade_set` / `villager_trade` registries,
//! offer generation (`AbstractVillager.addOffersFromTradeSet` with each
//! set's `RandomSequence`), `MerchantOffer` pricing, demand and stock, and the
//! `VillagerData` level thresholds. The villager entity itself is not part of
//! this module; anything that can trade calls [`Merchant`].
use crate::{
    enchant::Enchantments,
    items::{ItemStack, Items},
};
use horizon_core::random::{RandomSource, XoroshiroRandom, mix_stafford13, seed_from_hash};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

/// `VillagerData.NEXT_LEVEL_XP_THRESHOLDS`.
pub const LEVEL_XP: [i32; 5] = [0, 10, 70, 150, 250];
pub fn can_level_up(level: i32) -> bool {
    (1..5).contains(&level)
}
pub fn min_xp(level: i32) -> i32 {
    if can_level_up(level) { LEVEL_XP[level as usize - 1] } else { 0 }
}
pub fn max_xp(level: i32) -> i32 {
    if can_level_up(level) { LEVEL_XP[level as usize] } else { 0 }
}
/// The professions with trades, as in `VillagerProfession`.
pub const PROFESSIONS: &[&str] = &[
    "armorer", "butcher", "cartographer", "cleric", "farmer", "fisherman", "fletcher", "leatherworker", "librarian", "mason", "shepherd", "toolsmith", "weaponsmith",
];

/// An `ItemCost`: item and count (component predicates are not used by the
/// 26.3 trade data except for one exact match, kept as the stack's data).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCost {
    pub stack: ItemStack,
}
impl ItemCost {
    pub fn test(&self, s: &ItemStack) -> bool {
        s.item == self.stack.item && (self.stack.extra.is_none() || s.data() == self.stack.data())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantOffer {
    pub base_cost_a: ItemCost,
    pub cost_b: Option<ItemCost>,
    pub result: ItemStack,
    pub uses: i32,
    pub max_uses: i32,
    pub reward_exp: bool,
    pub special_price_diff: i32,
    pub demand: i32,
    pub price_multiplier: f32,
    pub xp: i32,
}
impl MerchantOffer {
    fn max_stack(items: &Items, s: &ItemStack) -> u32 {
        items.get(&s.item).map_or(64, |i| i.max_stack)
    }
    /// `getModifiedCostCount` for cost A.
    pub fn cost_a_count(&self, items: &Items) -> u32 {
        let base = self.base_cost_a.stack.count as i32;
        let demand_diff = ((base as f32 * self.demand as f32 * self.price_multiplier).floor() as i32).max(0);
        (base + demand_diff + self.special_price_diff).clamp(1, Self::max_stack(items, &self.base_cost_a.stack) as i32) as u32
    }
    pub fn cost_a(&self, items: &Items) -> ItemStack {
        ItemStack { count: self.cost_a_count(items), ..self.base_cost_a.stack.clone() }
    }
    pub fn cost_b(&self) -> Option<ItemStack> {
        self.cost_b.as_ref().map(|c| c.stack.clone())
    }
    pub fn out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }
    /// `MerchantOffer.updateDemand` (on restock).
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }
    /// `satisfiedBy(buyA, buyB)`.
    pub fn satisfied_by(&self, items: &Items, a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
        let Some(a) = a else { return false };
        if !self.base_cost_a.test(a) || a.count < self.cost_a_count(items) {
            return false;
        }
        match (&self.cost_b, b) {
            (None, b) => b.is_none(),
            (Some(c), Some(b)) => c.test(b) && b.count >= c.stack.count,
            (Some(_), None) => false,
        }
    }
    /// `MerchantOffer.take`: consume the payment; false when not satisfied.
    pub fn take(&self, items: &Items, a: &mut Option<ItemStack>, b: &mut Option<ItemStack>) -> bool {
        if !self.satisfied_by(items, a.as_ref(), b.as_ref()) {
            return false;
        }
        let n = self.cost_a_count(items);
        if let Some(s) = a.as_mut() {
            s.count -= n;
            if s.count == 0 {
                *a = None;
            }
        }
        if let (Some(c), Some(s)) = (&self.cost_b, b.as_mut()) {
            s.count -= c.stack.count;
            if s.count == 0 {
                *b = None;
            }
        }
        true
    }
}
/// `MerchantOffers.getRecipeFor`: the hinted offer if it fits, else the first that does.
pub fn recipe_for(offers: &[MerchantOffer], items: &Items, a: Option<&ItemStack>, b: Option<&ItemStack>, hint: Option<usize>) -> Option<usize> {
    if let Some(h) = hint
        && offers.get(h).is_some_and(|o| o.satisfied_by(items, a, b))
    {
        return Some(h);
    }
    offers.iter().position(|o| o.satisfied_by(items, a, b))
}
/// `MerchantContainer.updateSellItem`: (active offer, result).
pub fn sell_item(offers: &[MerchantOffer], items: &Items, a: Option<&ItemStack>, b: Option<&ItemStack>, hint: Option<usize>) -> (Option<usize>, Option<ItemStack>) {
    let (a, b) = if a.is_none() { (b, None) } else { (a, b) };
    if a.is_none() {
        return (None, None);
    }
    let mut found = recipe_for(offers, items, a, b, hint);
    let mut active = found;
    if found.is_none_or(|i| offers[i].out_of_stock()) {
        active = found;
        found = recipe_for(offers, items, b, a, hint);
    }
    match found {
        Some(i) if !offers[i].out_of_stock() => (Some(i), Some(offers[i].result.clone())),
        _ => (active, None),
    }
}

/// `RandomSequence(worldSeed ^ salt, id)` with the default salt 0.
pub fn sequence_random(world_seed: i64, id: &str) -> XoroshiroRandom {
    const SILVER: u64 = 0x6a09_e667_f3bc_c909;
    const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;
    let lo = world_seed as u64 ^ SILVER;
    let hi = lo.wrapping_add(GOLDEN);
    let key = seed_from_hash(id);
    XoroshiroRandom::from_state(mix_stafford13(lo ^ key[0]), mix_stafford13(hi ^ key[1]))
}

/// The trade registries.
pub struct Trades {
    pub trades: HashMap<String, Value>,
    /// villager_trade tags, ordered.
    pub tags: HashMap<String, Vec<String>>,
    pub sets: HashMap<String, Value>,
}
fn walk(dir: &Path, base: &Path, out: &mut HashMap<String, Value>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "json")
            && let Ok(rel) = p.strip_prefix(base)
            && let Ok(bytes) = fs::read(&p)
            && let Ok(v) = serde_json::from_slice(&bytes)
        {
            out.insert(format!("minecraft:{}", rel.with_extension("").to_string_lossy().replace('\\', "/")), v);
        }
    }
}
impl Trades {
    pub fn load(root: &Path) -> Self {
        let data = root.join("data/minecraft");
        let (mut trades, mut raw_tags, mut sets) = (HashMap::new(), HashMap::new(), HashMap::new());
        walk(&data.join("villager_trade"), &data.join("villager_trade"), &mut trades);
        walk(&data.join("tags/villager_trade"), &data.join("tags/villager_trade"), &mut raw_tags);
        walk(&data.join("trade_set"), &data.join("trade_set"), &mut sets);
        let raw: HashMap<String, Vec<String>> = raw_tags
            .into_iter()
            .map(|(k, v)| (k, v["values"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_owned)).collect()))
            .collect();
        fn expand(name: &str, raw: &HashMap<String, Vec<String>>, out: &mut Vec<String>, depth: u32) {
            for v in raw.get(name).into_iter().flatten() {
                match v.strip_prefix('#') {
                    Some(t) if depth < 16 => expand(t, raw, out, depth + 1),
                    Some(_) => {}
                    None if !out.contains(v) => out.push(v.clone()),
                    None => {}
                }
            }
        }
        let tags = raw
            .keys()
            .map(|k| {
                let mut v = Vec::new();
                expand(k, &raw, &mut v, 0);
                (k.clone(), v)
            })
            .collect();
        Self { trades, tags, sets }
    }
    /// `VillagerProfession.getTrades(level)`.
    pub fn set_for(profession: &str, level: i32) -> String {
        format!("minecraft:{}/level_{level}", profession.trim_start_matches("minecraft:"))
    }
    fn trade_list(&self, spec: &Value) -> Vec<String> {
        match spec {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.clone()],
            },
            Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        }
    }
    /// `AbstractVillager.addOffersFromTradeSet`.
    pub fn offers_from_set<R: RandomSource>(&self, ctx: &TradeContext<'_>, set_id: &str, random: &mut R) -> Vec<MerchantOffer> {
        let Some(set) = self.sets.get(set_id) else { return vec![] };
        let mut potential = self.trade_list(&set["trades"]);
        let amount = int_provider(&set["amount"], random);
        let duplicates = set["allow_duplicates"].as_bool().unwrap_or(false);
        let mut out = Vec::new();
        while (out.len() as i32) < amount && !potential.is_empty() {
            let roll = random.next_int_bound(potential.len() as i32).unwrap_or(0) as usize;
            let id = if duplicates { potential[roll].clone() } else { potential.remove(roll) };
            match self.trades.get(&id).and_then(|t| offer_from(ctx, t, random)) {
                Some(o) => out.push(o),
                None if duplicates => {
                    potential.remove(roll);
                }
                None => {}
            }
        }
        out
    }
}
/// What a villager trade's loot context provides.
pub struct TradeContext<'a> {
    pub items: &'a Items,
    pub enchantments: &'a Enchantments,
    /// Villager type (`minecraft:plains`, ...), for `merchant_predicate`.
    pub variant: &'a str,
}
fn int_provider<R: RandomSource>(v: &Value, random: &mut R) -> i32 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:uniform" => {
                let (min, max) = (o["min"].as_i64().unwrap_or(0) as i32, o["max"].as_i64().unwrap_or(0) as i32);
                if min >= max { min } else { min + random.next_int_bound(max - min + 1).unwrap_or(0) }
            }
            "minecraft:constant" => o["value"].as_i64().unwrap_or(0) as i32,
            _ => o.get("value").and_then(Value::as_i64).unwrap_or(0) as i32,
        },
        _ => 0,
    }
}
fn cost(items: &Items, v: &Value, additional: i32) -> ItemCost {
    let id = v["id"].as_str().unwrap_or("minecraft:air");
    let max = items.get(id).map_or(64, |i| i.max_stack) as i32;
    let count = (v["count"].as_i64().unwrap_or(1) as i32 + additional).clamp(0, max);
    ItemCost { stack: ItemStack::new(id, count as u32) }
}
fn variant_ok(pred: &Value, variant: &str) -> bool {
    match &pred["predicate"]["minecraft:predicates"]["minecraft:villager/variant"] {
        Value::String(s) => s == variant,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(variant)),
        _ => true,
    }
}
/// `VillagerTrade.getOffer`.
pub fn offer_from<R: RandomSource>(ctx: &TradeContext<'_>, t: &Value, random: &mut R) -> Option<MerchantOffer> {
    if let Some(p) = t.get("merchant_predicate")
        && !variant_ok(p, ctx.variant)
    {
        return None;
    }
    let gives = &t["gives"];
    let mut result = ItemStack::new(gives["id"].as_str()?, gives["count"].as_u64().unwrap_or(1) as u32);
    let mut additional = 0;
    let modifiers: Vec<&Value> = match &t["given_item_modifier"] {
        Value::Array(a) => a.iter().collect(),
        v @ Value::Object(_) => vec![v],
        _ => vec![],
    };
    for m in modifiers {
        let (stack, extra) = apply_modifier(ctx, m, result, random)?;
        result = stack;
        additional += extra;
    }
    if let Some(Value::String(tag)) = t.get("double_trade_price_enchantments")
        && result.stored_enchantments().iter().any(|(e, _)| ctx.enchantments.in_tag(tag, e))
    {
        additional *= 2;
    }
    let a = cost(ctx.items, &t["wants"], additional);
    if a.stack.count < 1 {
        return None;
    }
    let b = t.get("additional_wants").map(|w| cost(ctx.items, w, 0));
    if b.as_ref().is_some_and(|b| b.stack.count < 1) {
        return None;
    }
    Some(MerchantOffer {
        base_cost_a: a,
        cost_b: b,
        result,
        uses: 0,
        max_uses: (t["max_uses"].as_i64().unwrap_or(4) as i32).max(1),
        reward_exp: true,
        special_price_diff: 0,
        demand: 0,
        price_multiplier: (t["reputation_discount"].as_f64().unwrap_or(0.0) as f32).max(0.0),
        xp: (t["xp"].as_i64().unwrap_or(1) as i32).max(0),
    })
}
/// The loot functions trades use; None discards the offer.
fn apply_modifier<R: RandomSource>(ctx: &TradeContext<'_>, m: &Value, stack: ItemStack, random: &mut R) -> Option<(ItemStack, i32)> {
    let e = ctx.enchantments;
    let pool = |options: &Value| -> Vec<Arc<str>> {
        match options {
            Value::String(s) if s.starts_with('#') => e.tag(s).to_vec(),
            Value::String(s) => vec![Arc::from(s.as_str())],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(Arc::from)).collect(),
            _ => e.list.iter().map(|x| x.id.clone()).collect(),
        }
    };
    match m["type"].as_str().or(m["function"].as_str()).unwrap_or("") {
        "minecraft:enchant_with_levels" => {
            let levels = int_provider(&m["levels"], random);
            // EnchantmentHelper.enchantItem on the loot context's random.
            let chosen = e.select(ctx.items, random, &stack.item, levels, &pool(&m["options"]));
            let mut out = if &*stack.item == "minecraft:book" { ItemStack::new("minecraft:enchanted_book", stack.count) } else { stack };
            for (id, l) in chosen {
                out.enchant(&id, l);
            }
            let extra = if m["include_additional_cost_component"].as_bool().unwrap_or(false) && levels > 0 { levels } else { 0 };
            Some((out, extra))
        }
        "minecraft:enchant_randomly" => {
            let book = &*stack.item == "minecraft:book";
            let only_compatible = m["only_compatible"].as_bool().unwrap_or(true);
            let options: Vec<Arc<str>> = pool(&m["options"]).into_iter().filter(|id| book || !only_compatible || e.can_enchant(ctx.items, id, &stack.item)).collect();
            if options.is_empty() {
                return Some((stack, 0));
            }
            let id = options[random.next_int_bound(options.len() as i32).unwrap_or(0) as usize].clone();
            let max = e.get(&id).map_or(1, |d| d.max_level) as i32;
            let level = if max <= 1 { 1 } else { 1 + random.next_int_bound(max).unwrap_or(0) };
            let mut out = if book { ItemStack::new("minecraft:enchanted_book", 1) } else { stack };
            out.enchant(&id, level as u32);
            let extra = if m["include_additional_cost_component"].as_bool().unwrap_or(false) {
                2 + random.next_int_bound(5 + level * 10).unwrap_or(0) + 3 * level
            } else {
                0
            };
            Some((out, extra))
        }
        "minecraft:filtered" => {
            let filter = &m["item_filter"];
            let item_ok = filter["items"].as_str().is_none_or(|i| i == &*stack.item);
            let needs = |key: &str, have: bool| filter["predicates"][key].as_array().is_none_or(|_| have);
            let ok = item_ok && needs("minecraft:enchantments", !stack.enchantments().is_empty()) && needs("minecraft:stored_enchantments", !stack.stored_enchantments().is_empty());
            if ok {
                Some((stack, 0))
            } else if m["on_fail"]["type"] == "minecraft:discard" {
                None
            } else {
                Some((stack, 0))
            }
        }
        "minecraft:set_potion" => Some((stack.with_potion(m["id"].as_str().unwrap_or("minecraft:water")), 0)),
        "minecraft:discard" => None,
        // Dyes, stew effects, random potions and exploration maps keep the plain item.
        _ => Some((stack, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn items() -> Items {
        Items::for_tests(HashMap::new())
    }
    fn offer(cost: u32, b: Option<u32>) -> MerchantOffer {
        MerchantOffer {
            base_cost_a: ItemCost { stack: ItemStack::new("minecraft:emerald", cost) },
            cost_b: b.map(|n| ItemCost { stack: ItemStack::new("minecraft:book", n) }),
            result: ItemStack::new("minecraft:iron_boots", 1),
            uses: 0,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier: 0.2,
            xp: 1,
        }
    }

    #[test]
    fn demand_raises_prices() {
        let items = items();
        let mut o = offer(4, None);
        assert_eq!(o.cost_a_count(&items), 4);
        o.uses = 12;
        o.update_demand();
        assert_eq!(o.demand, 12);
        // 4 + floor(4 * 12 * 0.2) = 13.
        assert_eq!(o.cost_a_count(&items), 13);
        o.special_price_diff = -20;
        assert_eq!(o.cost_a_count(&items), 1);
    }

    #[test]
    fn payment_and_selection() {
        let items = items();
        let offers = vec![offer(5, None), offer(1, Some(1))];
        let mut a = Some(ItemStack::new("minecraft:emerald", 6));
        let mut b = None;
        assert_eq!(sell_item(&offers, &items, a.as_ref(), b.as_ref(), None).0, Some(0));
        assert!(offers[0].take(&items, &mut a, &mut b));
        assert_eq!(a.as_ref().unwrap().count, 1);
        // Payment in the second slot only still matches (A and B swap).
        let book = Some(ItemStack::new("minecraft:book", 1));
        let (i, r) = sell_item(&offers, &items, None, Some(&ItemStack::new("minecraft:emerald", 5)), None);
        assert_eq!((i, r.map(|r| r.item)), (Some(0), Some(Arc::from("minecraft:iron_boots"))));
        assert_eq!(sell_item(&offers, &items, a.as_ref(), book.as_ref(), Some(1)).0, Some(1));
        let mut out = offers[0].clone();
        out.uses = 12;
        assert_eq!(sell_item(&[out], &items, Some(&ItemStack::new("minecraft:emerald", 5)), None, None).1, None);
    }

    #[test]
    fn sequence_random_is_stable() {
        let mut a = sequence_random(12345, "minecraft:trade_set/armorer/level_1");
        let mut b = sequence_random(12345, "minecraft:trade_set/armorer/level_1");
        let mut c = sequence_random(12345, "minecraft:trade_set/butcher/level_1");
        let (x, y, z) = (a.next_long(), b.next_long(), c.next_long());
        assert_eq!(x, y);
        assert_ne!(x, z);
        assert_eq!(min_xp(2), 10);
        assert_eq!(max_xp(2), 70);
        assert!(!can_level_up(5));
    }

    #[test]
    fn offers_follow_the_trade_data() {
        let items = items();
        let e = crate::enchant::tests::sample();
        let mut trades = HashMap::new();
        trades.insert("minecraft:t/boots".into(), json!({"gives":{"id":"minecraft:iron_boots"},"max_uses":12,"reputation_discount":0.2,"wants":{"count":4,"id":"minecraft:emerald"}}));
        trades.insert("minecraft:t/helmet".into(), json!({"gives":{"id":"minecraft:iron_helmet"},"max_uses":12,"reputation_discount":0.2,"wants":{"count":5,"id":"minecraft:emerald"}}));
        trades.insert("minecraft:t/desert".into(), json!({"gives":{"id":"minecraft:sand"},"wants":{"id":"minecraft:emerald"},"merchant_predicate":{"entity":"this","predicate":{"minecraft:predicates":{"minecraft:villager/variant":"minecraft:desert"}},"type":"minecraft:entity_properties"}}));
        let mut tags = HashMap::new();
        tags.insert("minecraft:t/level_1".into(), vec!["minecraft:t/boots".into(), "minecraft:t/helmet".into(), "minecraft:t/desert".into()]);
        let mut sets = HashMap::new();
        sets.insert("minecraft:t/level_1".into(), json!({"amount": 5, "trades": "#minecraft:t/level_1"}));
        let t = Trades { trades, tags, sets };
        let ctx = TradeContext { items: &items, enchantments: &e, variant: "minecraft:plains" };
        let offers = t.offers_from_set(&ctx, "minecraft:t/level_1", &mut sequence_random(1, "x"));
        // The desert-only trade is skipped for a plains villager; no duplicates.
        assert_eq!(offers.len(), 2);
        assert_ne!(offers[0].result.item, offers[1].result.item);
        assert!(offers.iter().all(|o| o.max_uses == 12 && (o.price_multiplier - 0.2).abs() < 1e-6));
    }
}
