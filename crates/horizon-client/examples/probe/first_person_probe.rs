//! Probe for test_first_person_parity.py: drives horizon_client::first_person
//! and attack_strength over the harness's scripted clicks, targets and use
//! results, printing what VanillaFirstPersonOracle prints.
use horizon_client::{
    Result,
    attack_strength::{AttackStrength, HeldStack},
    first_person::{
        Clicks, Hand, HandHeights, HandSway, HandWorld, Hit, SwingAnimation, SwingState, UseResult,
        swing_duration,
    },
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn fb(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}
fn fl(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn dl(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

/// An `ItemStack` object: the hands hold indices into `Probe::stacks`, so
/// shrinking keeps its identity as vanilla's in-place `shrink` does.
#[derive(Clone)]
struct Stack {
    item: String,
    count: i32,
}
impl Stack {
    fn empty(&self) -> bool {
        self.item == "minecraft:air" || self.count <= 0
    }
    /// `getItem` and `getCount` of an empty stack are air and 0.
    fn effective(&self) -> (&str, i32) {
        if self.empty() {
            ("minecraft:air", 0)
        } else {
            (&self.item, self.count)
        }
    }
}

struct Probe<'a> {
    case: &'a Value,
    step: &'a Value,
    creative: bool,
    stacks: Vec<Stack>,
    /// The stacks in the main and off hand.
    hands: [usize; 2],
    blocks: HashMap<[i32; 3], String>,
    swing: SwingState,
    events: Vec<String>,
    heights: HandHeights,
}
fn hand_name(hand: Hand) -> &'static str {
    match hand {
        Hand::Main => "main",
        Hand::Off => "off",
    }
}
impl Probe<'_> {
    fn stack(&self, hand: Hand) -> &Stack {
        &self.stacks[self.hands[hand as usize]]
    }
    fn new_stack(&mut self, v: &Value) -> usize {
        // ItemStack.EMPTY is index 0.
        if v.is_null() {
            return 0;
        }
        self.stacks.push(Stack {
            item: v[0].as_str().unwrap().into(),
            count: v[1].as_i64().unwrap() as i32,
        });
        self.stacks.len() - 1
    }
    fn table<'b>(&'b self, name: &str) -> &'b Value {
        &self.case["rust"][name]
    }
    fn attack_speed(&self) -> f64 {
        let item = self.stack(Hand::Main).effective().0;
        self.table("attack_speed").get(item).map_or(4.0, dl)
    }
    fn scripted(&mut self, kind: &str, hand: Hand) -> UseResult {
        let name = self.step["uses"][kind].as_str().unwrap().to_string();
        self.events
            .push(format!("use {kind} {} {name}", hand_name(hand)));
        let result = match name.as_str() {
            "success" => UseResult::Success { swing: true },
            "server" | "consume" => UseResult::Success { swing: false },
            "fail" => UseResult::Fail,
            _ => UseResult::Pass,
        };
        if matches!(result, UseResult::Success { .. }) && self.step["shrink"].as_bool().unwrap() {
            let i = self.hands[hand as usize];
            if i != 0 {
                self.stacks[i].count -= 1;
            }
        }
        result
    }
}
impl HandWorld for Probe<'_> {
    fn hit(&mut self) -> Hit {
        let h = &self.step["hit"];
        match h[0].as_str().unwrap() {
            "block" => Hit::Block {
                pos: [0, 1, 2].map(|i| h[i + 1].as_i64().unwrap() as i32),
                face: h[4].as_u64().unwrap() as u8,
            },
            "entity" => Hit::Entity,
            _ => Hit::Miss,
        }
    }
    fn is_air(&mut self, pos: [i32; 3]) -> bool {
        self.blocks.get(&pos).is_none_or(|b| b == "minecraft:air")
    }
    fn creative(&self) -> bool {
        self.creative
    }
    fn spectator(&self) -> bool {
        false
    }
    fn instabuild(&self) -> bool {
        self.creative
    }
    fn hands_busy(&self) -> bool {
        self.step["busy"].as_bool().unwrap()
    }
    fn using_item(&self) -> bool {
        self.step["using"].as_bool().unwrap()
    }
    fn item_enabled(&mut self, _: Hand) -> bool {
        true
    }
    fn held_empty(&mut self, hand: Hand) -> bool {
        self.stack(hand).empty()
    }
    fn held_count(&mut self, hand: Hand) -> i32 {
        self.stack(hand).effective().1
    }
    fn infinite_materials(&self) -> bool {
        self.creative
    }
    fn cannot_attack(&mut self) -> bool {
        false
    }
    fn piercing_weapon(&mut self) -> bool {
        false
    }
    fn attack_animation(&mut self) -> SwingAnimation {
        SwingAnimation::DEFAULT
    }
    fn interact_animation(&mut self, _: Hand) -> SwingAnimation {
        SwingAnimation::DEFAULT
    }
    fn piercing_attack(&mut self, _: SwingAnimation) {
        unreachable!()
    }
    fn attack_entity(&mut self) -> bool {
        self.events.push("attack".into());
        true
    }
    fn block_action_restricted(&mut self, _: [i32; 3]) -> bool {
        false
    }
    fn within_border(&mut self, _: [i32; 3]) -> bool {
        true
    }
    fn attack_block(&mut self, _: [i32; 3]) {}
    /// `BlockBehaviour.getDestroyProgress` with `Player.getDestroySpeed` for
    /// a player on the ground, out of water, with no mining efficiency.
    fn destroy_progress(&mut self, pos: [i32; 3]) -> f32 {
        let block = self.blocks.get(&pos).cloned().unwrap_or_default();
        let b = &self.table("blocks")[block.as_str()];
        let hardness = fl(&b[0]);
        if hardness == -1.0 {
            return 0.0;
        }
        let item = self.stack(Hand::Main).effective().0.to_string();
        let tool = &self.table("tools")[format!("{item} {block}").as_str()];
        let (mut speed, correct) = if tool.is_null() {
            (1.0f32, false)
        } else {
            (fl(&tool[0]), tool[1].as_bool().unwrap())
        };
        if let Some(amp) = self.case["haste"].as_i64() {
            speed *= 1.0 + (amp as i32 + 1) as f32 * 0.2;
        }
        if let Some(amp) = self.case["fatigue"].as_i64() {
            speed *= horizon_core::jmath::pow(0.3, f64::from(amp as i32 + 1)) as f32;
        }
        let harvest = !b[1].as_bool().unwrap() || correct;
        speed / hardness / if harvest { 30.0 } else { 100.0 }
    }
    fn destroy_block(&mut self, pos: [i32; 3]) -> bool {
        let item = self.stack(Hand::Main).effective().0;
        // Item.canDestroyBlock: swords' tools cannot destroy in creative.
        let sword = item.ends_with("_sword");
        let ok = !(sword && self.creative) && !self.is_air(pos);
        if ok {
            self.blocks.insert(pos, "minecraft:air".into());
        }
        self.events
            .push(format!("destroy {} {} {} {ok}", pos[0], pos[1], pos[2]));
        ok
    }
    /// The stack object itself: `destroyingItem` is a reference.
    fn main_hand_key(&mut self) -> String {
        self.hands[0].to_string()
    }
    fn same_destroying_item(&mut self, key: &str) -> bool {
        // ItemStack.EMPTY (index 0) before any break.
        let then = &self.stacks[key.parse::<usize>().unwrap_or(0)];
        then.effective().0 == self.stack(Hand::Main).effective().0
    }
    fn destroy_stage(&mut self, pos: [i32; 3], stage: Option<i32>) {
        self.events.push(format!(
            "stage {} {} {} {}",
            pos[0],
            pos[1],
            pos[2],
            stage.unwrap_or(-1)
        ));
    }
    fn swing(&mut self, hand: Hand, animation: SwingAnimation) {
        let duration = swing_duration(
            animation,
            self.case["haste"].as_i64().map(|a| a as i32),
            self.case["fatigue"].as_i64().map(|a| a as i32),
        );
        let started = self.swing.start_if_able(hand, animation, duration);
        self.events.push(format!(
            "swing {} {} {started}",
            hand_name(hand),
            animation.duration
        ));
    }
    fn reset_attack_strength(&mut self) {
        unreachable!("ResetWorld resets the tickers")
    }
    fn entity_within_border(&mut self) -> bool {
        true
    }
    fn use_on_entity(&mut self, hand: Hand) -> Option<UseResult> {
        self.step["hit"][1]
            .as_bool()
            .unwrap()
            .then(|| self.scripted("entity", hand))
    }
    fn use_on_block(&mut self, hand: Hand) -> UseResult {
        self.scripted("block", hand)
    }
    fn use_item(&mut self, hand: Hand) -> UseResult {
        self.scripted("item", hand)
    }
    fn item_used(&mut self, hand: Hand) {
        self.events.push(format!("used {}", hand_name(hand)));
        self.heights.item_used(hand);
    }
    fn release_using(&mut self) {
        self.events.push("release".into());
    }
}
fn held(stack: &Stack) -> Option<HeldStack> {
    (!stack.empty()).then(|| HeldStack {
        item: stack.item.clone(),
        rest: stack.count.to_string(),
    })
}

fn run(case: &Value, out: &mut impl Write) -> Result<()> {
    let empty = Value::Null;
    let mut probe = Probe {
        case,
        step: &empty,
        creative: case["creative"].as_bool().unwrap(),
        stacks: vec![Stack {
            item: "minecraft:air".into(),
            count: 0,
        }],
        hands: [0, 0],
        blocks: case["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    [0, 1, 2].map(|i| b[i].as_i64().unwrap() as i32),
                    b[3].as_str().unwrap().to_string(),
                )
            })
            .collect(),
        swing: SwingState::default(),
        events: Vec::new(),
        heights: HandHeights::default(),
    };
    probe.hands[0] = probe.new_stack(&case["main"]);
    probe.hands[1] = probe.new_stack(&case["off"]);
    writeln!(out, "attack_speed {:x}", probe.attack_speed().to_bits())?;
    let mut clicks = Clicks::default();
    let mut strength = AttackStrength::default();
    // FirstPersonHandsAndItems' visible items start as ItemStack.EMPTY.
    let mut visible = [0usize, 0usize];
    // The player's xRot and yRot, and the hands' lag behind them.
    let mut rot = [0.0f32; 2];
    let mut sway = HandSway::default();
    for (s, step) in case["steps"].as_array().unwrap().iter().enumerate() {
        probe.step = step;
        probe.events.clear();
        if !step["set"].is_null() {
            let i = probe.new_stack(&step["set"]["stack"]);
            let hand = if step["set"]["hand"] == "main" { 0 } else { 1 };
            probe.hands[hand] = i;
        }
        clicks.tick(
            &mut ResetWorld {
                probe: &mut probe,
                strength: &mut strength,
            },
            step["ac"].as_u64().unwrap() as u32,
            step["uc"].as_u64().unwrap() as u32,
            step["ad"].as_bool().unwrap(),
            step["ud"].as_bool().unwrap(),
        );
        probe.swing.tick();
        if let Some(r) = step["rot"].as_array() {
            rot = [fl(&r[0]), fl(&r[1])];
        }
        sway.tick(rot[0], rot[1]);
        let main = probe.stack(Hand::Main).clone();
        strength.tick(held(&main));
        // FirstPersonHandsAndItems.tick: shouldInstantlyReplaceVisibleItem
        // (ItemStack.matchesIgnoringComponents; every item plays the swap
        // animation), then the heights.
        let mut changed = [false; 2];
        for h in 0..2 {
            let (a, b) = (visible[h], probe.hands[h]);
            let same = a == b || probe.stacks[a].effective() == probe.stacks[b].effective();
            if same {
                visible[h] = b;
            }
            changed[h] = visible[h] != b;
        }
        let speed = probe.attack_speed();
        let replace = probe.heights.tick(
            probe.step["busy"].as_bool().unwrap(),
            strength.swap_scale(1.0, speed),
            changed,
        );
        for h in 0..2 {
            if replace[h] {
                visible[h] = probe.hands[h];
            }
        }
        for e in &probe.events {
            writeln!(out, "{e}")?;
        }
        let sw = &probe.swing;
        let current = sw.current.map_or("- 0".to_string(), |c| {
            format!("{} {}", hand_name(c.hand), c.duration_ticks)
        });
        let h = &probe.heights;
        let d = &clicks.destroying;
        writeln!(
            out,
            "tick {s} {current} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
            sw.ticks,
            fb(sw.animation),
            fb(sw.old_animation),
            fb(sw.animation(0.5)),
            strength.ticker,
            strength.swap_ticker,
            fb(strength.swap_scale(1.0, speed)),
            fb(h.main),
            fb(h.old_main),
            fb(h.off),
            fb(h.old_off),
            clicks.miss_time,
            clicks.right_click_delay,
            d.delay,
            d.active,
            fb(d.progress),
            fb(sway.x_bob),
            fb(sway.y_bob),
            fb(sway.x_bob_o),
            fb(sway.y_bob_o)
        )?;
    }
    Ok(())
}

/// The probe with the attack-strength tickers it resets.
struct ResetWorld<'p, 'c> {
    probe: &'p mut Probe<'c>,
    strength: &'p mut AttackStrength,
}
macro_rules! forward {
    ($($name:ident($($arg:ident: $t:ty),*) -> $r:ty;)*) => {
        $(fn $name(&mut self, $($arg: $t),*) -> $r { self.probe.$name($($arg),*) })*
    };
}
impl HandWorld for ResetWorld<'_, '_> {
    forward! {
        hit() -> Hit;
        is_air(pos: [i32; 3]) -> bool;
        item_enabled(hand: Hand) -> bool;
        held_empty(hand: Hand) -> bool;
        held_count(hand: Hand) -> i32;
        cannot_attack() -> bool;
        piercing_weapon() -> bool;
        attack_animation() -> SwingAnimation;
        interact_animation(hand: Hand) -> SwingAnimation;
        piercing_attack(a: SwingAnimation) -> ();
        attack_entity() -> bool;
        block_action_restricted(pos: [i32; 3]) -> bool;
        within_border(pos: [i32; 3]) -> bool;
        attack_block(pos: [i32; 3]) -> ();
        destroy_progress(pos: [i32; 3]) -> f32;
        destroy_block(pos: [i32; 3]) -> bool;
        main_hand_key() -> String;
        same_destroying_item(key: &str) -> bool;
        destroy_stage(pos: [i32; 3], stage: Option<i32>) -> ();
        swing(hand: Hand, a: SwingAnimation) -> ();
        entity_within_border() -> bool;
        use_on_entity(hand: Hand) -> Option<UseResult>;
        use_on_block(hand: Hand) -> UseResult;
        use_item(hand: Hand) -> UseResult;
        item_used(hand: Hand) -> ();
        release_using() -> ();
    }
    fn creative(&self) -> bool {
        self.probe.creative()
    }
    fn spectator(&self) -> bool {
        false
    }
    fn instabuild(&self) -> bool {
        self.probe.instabuild()
    }
    fn hands_busy(&self) -> bool {
        self.probe.hands_busy()
    }
    fn using_item(&self) -> bool {
        self.probe.using_item()
    }
    fn infinite_materials(&self) -> bool {
        self.probe.infinite_materials()
    }
    fn reset_attack_strength(&mut self) {
        self.strength.reset();
    }
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in input.lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        run(&c, &mut out)?;
    }
    out.flush()?;
    Ok(())
}
