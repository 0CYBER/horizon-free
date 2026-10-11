//! Layout of vanilla's `Hud` decorations as draw calls (`GuiGraphicsExtractor`
//! blits and text): the hotbar with its selection, off-hand slot, items and
//! attack indicator, the crosshair, armour, hearts, food, air bubbles, mount
//! hearts and the experience bar and level. Computed without drawing so the
//! parity fixture can compare them with the vanilla client.
use horizon_core::random::{LegacyRandom, RandomSource};

/// One `GuiGraphicsExtractor` call.
#[derive(Clone, Debug, PartialEq)]
pub enum GuiDraw {
    /// `blitSprite(sprite, x, y, w, h)`; `region` = (spriteW, spriteH, u, v)
    /// for the partial overload.
    Sprite {
        sprite: &'static str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        region: Option<[i32; 4]>,
    },
    /// `text(font, text, x, y, argb, dropShadow)`.
    Text {
        text: String,
        x: i32,
        y: i32,
        color: u32,
        shadow: bool,
    },
    /// `item(owner, stack, x, y, seed)` for a hotbar slot (0-8, 40 = off hand)
    /// under the pose's x/y scale about (cx, cy) (the pop squeeze).
    Item {
        slot: u32,
        x: i32,
        y: i32,
        seed: i32,
        scale: [f32; 2],
        pivot: [f32; 2],
    },
}
fn sprite(sprite: &'static str, x: i32, y: i32, w: i32, h: i32) -> GuiDraw {
    GuiDraw::Sprite {
        sprite,
        x,
        y,
        w,
        h,
        region: None,
    }
}

/// `Hud.extractItemHotbar` inputs.
pub struct HotbarInputs {
    pub selected: i32,
    /// The main arm is the right one (the off hand sits left of the hotbar).
    pub right_handed: bool,
    pub offhand: bool,
    /// Pop time minus partial tick (`ItemStack.getPopTime`) per hotbar slot
    /// that holds an item, then the off hand.
    pub slots: [Option<f32>; 9],
    pub offhand_pop: f32,
    /// `getAttackStrengthScale(0)` when the attack indicator is in HOTBAR mode.
    pub attack: Option<f32>,
}

/// `Hud.extractItemHotbar`.
pub fn hotbar(w: i32, h: i32, i: &HotbarInputs) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let center = w / 2;
    out.push(sprite("hud/hotbar", center - 91, h - 22, 182, 22));
    out.push(sprite(
        "hud/hotbar_selection",
        center - 91 - 1 + i.selected * 20,
        h - 22 - 1,
        24,
        23,
    ));
    // The off hand is opposite the main arm.
    let offhand_left = i.right_handed;
    if i.offhand {
        if offhand_left {
            out.push(sprite(
                "hud/hotbar_offhand_left",
                center - 91 - 29,
                h - 23,
                29,
                24,
            ));
        } else {
            out.push(sprite(
                "hud/hotbar_offhand_right",
                center + 91,
                h - 23,
                29,
                24,
            ));
        }
    }
    let slot = |slot: u32, x: i32, y: i32, seed: i32, pop: f32, out: &mut Vec<GuiDraw>| {
        let (scale, pivot) = if pop > 0.0 {
            let squeeze = 1.0 + pop / 5.0;
            (
                [1.0 / squeeze, (squeeze + 1.0) / 2.0],
                [(x + 8) as f32, (y + 12) as f32],
            )
        } else {
            ([1.0, 1.0], [0.0, 0.0])
        };
        out.push(GuiDraw::Item {
            slot,
            x,
            y,
            seed,
            scale,
            pivot,
        });
    };
    let mut seed = 1;
    for (n, pop) in i.slots.iter().enumerate() {
        if let Some(pop) = pop {
            slot(
                n as u32,
                center - 90 + n as i32 * 20 + 2,
                h - 16 - 3,
                seed,
                *pop,
                &mut out,
            );
        }
        seed += 1;
    }
    if i.offhand {
        let x = if offhand_left {
            center - 91 - 26
        } else {
            center + 91 + 10
        };
        slot(40, x, h - 16 - 3, seed, i.offhand_pop, &mut out);
    }
    if let Some(scale) = i.attack
        && scale < 1.0
    {
        let y = h - 20;
        let x = if offhand_left {
            center + 91 + 6
        } else {
            center - 91 - 22
        };
        let progress = (scale * 19.0) as i32;
        out.push(sprite(
            "hud/hotbar_attack_indicator_background",
            x,
            y,
            18,
            18,
        ));
        out.push(GuiDraw::Sprite {
            sprite: "hud/hotbar_attack_indicator_progress",
            x,
            y: y + 18 - progress,
            w: 18,
            h: progress,
            region: Some([18, 18, 0, 18 - progress]),
        });
    }
    out
}

/// `Hud.extractCrosshair` (first person, not hidden for spectators):
/// `attack` is `getAttackStrengthScale(0)` in CROSSHAIR indicator mode and
/// `full` whether the full indicator shows for a target in reach.
pub fn crosshair(w: i32, h: i32, attack: Option<f32>, full: bool) -> Vec<GuiDraw> {
    let mut out = vec![sprite("hud/crosshair", (w - 15) / 2, (h - 15) / 2, 15, 15)];
    if let Some(scale) = attack {
        let (x, y) = (w / 2 - 8, h / 2 - 7 + 16);
        if full {
            out.push(sprite("hud/crosshair_attack_indicator_full", x, y, 16, 16));
        } else if scale < 1.0 {
            let progress = (scale * 17.0) as i32;
            out.push(sprite(
                "hud/crosshair_attack_indicator_background",
                x,
                y,
                16,
                4,
            ));
            out.push(GuiDraw::Sprite {
                sprite: "hud/crosshair_attack_indicator_progress",
                x,
                y,
                w: progress,
                h: 4,
                region: Some([16, 4, 0, 0]),
            });
        }
    }
    out
}

/// `ExperienceBar.extractBackground` and `ContextualBar.extractExperienceLevel`;
/// `width` measures text with the font.
pub fn experience(
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    if bar {
        let (left, top) = (w / 2 - 91, h - 32 + 3);
        let filled = (progress * 183.0) as i32;
        out.push(sprite("hud/experience_bar_background", left, top, 182, 5));
        if filled > 0 {
            out.push(GuiDraw::Sprite {
                sprite: "hud/experience_bar_progress",
                x: left,
                y: top,
                w: filled,
                h: 5,
                region: Some([182, 5, 0, 0]),
            });
        }
    }
    if level > 0 {
        let text = level.to_string();
        let x = (w - width(&text)) / 2;
        let y = h - 24 - 9 - 2;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            out.push(GuiDraw::Text {
                text: text.clone(),
                x: x + dx,
                y: y + dy,
                color: 0xFF00_0000,
                shadow: false,
            });
        }
        out.push(GuiDraw::Text {
            text,
            x,
            y,
            color: 0xFF80_FF20,
            shadow: false,
        });
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeartKind {
    Normal,
    Poisoned,
    Withered,
    Frozen,
}

/// State kept between frames (`Hud.lastHealth`, `displayHealth`, ...).
#[derive(Clone, Debug, Default)]
pub struct HealthHud {
    pub last_health: i32,
    pub display_health: i32,
    /// `Util.getMillis()` of the last health change.
    pub last_health_time: i64,
    pub health_blink_time: i64,
}

pub struct HealthInputs {
    /// `Hud.tickCount`.
    pub tick: i32,
    /// `Util.getMillis()` now.
    pub millis: i64,
    pub health: f32,
    pub max_health: f32,
    pub absorption: f32,
    pub armor: i32,
    pub food: i32,
    pub saturation: f32,
    pub kind: HeartKind,
    pub regeneration: bool,
    pub hunger: bool,
    pub hardcore: bool,
    /// `damageCooldownTime > 0`.
    pub damage_cooldown: bool,
    /// `getAirSupply`, `getMaxAirSupply`, `isEyeInFluid(WATER)`.
    pub air: i32,
    pub max_air: i32,
    pub under_water: bool,
    /// `getVehicleMaxHearts` of the ridden living vehicle (0 = none).
    pub vehicle_hearts: i32,
}

/// `Hud.getVehicleMaxHearts` for a vehicle that shows its health.
pub fn vehicle_max_hearts(max_health: f32) -> i32 {
    (((max_health + 0.5) as i32) / 2).min(30)
}

/// `Hud.extractVehicleHealth`.
pub fn vehicle_health(w: i32, h: i32, hearts: i32, health: f32) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let current = f64::from(health).ceil() as i32;
    let x_right = w / 2 + 91;
    let mut yo = h - 39;
    let mut hearts = hearts;
    let mut base = 0;
    while hearts > 0 {
        let row = hearts.min(10);
        hearts -= row;
        for i in 0..row {
            let xo = x_right - i * 8 - 9;
            out.push(sprite("hud/heart/vehicle_container", xo, yo, 9, 9));
            if i * 2 + 1 + base < current {
                out.push(sprite("hud/heart/vehicle_full", xo, yo, 9, 9));
            }
            if i * 2 + 1 + base == current {
                out.push(sprite("hud/heart/vehicle_half", xo, yo, 9, 9));
            }
        }
        yo -= 10;
        base += 20;
    }
    out
}

fn heart(
    kind: Option<HeartKind>,
    absorbing: bool,
    hardcore: bool,
    half: bool,
    blink: bool,
) -> &'static str {
    // Hud.HeartType sprites: [full, full_blinking, half, half_blinking] x [normal, hardcore].
    let table: [&'static str; 8] = match (kind, absorbing) {
        (None, _) => [
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
        ],
        (_, true) => [
            "hud/heart/absorbing_full",
            "hud/heart/absorbing_full_blinking",
            "hud/heart/absorbing_half",
            "hud/heart/absorbing_half_blinking",
            "hud/heart/absorbing_hardcore_full",
            "hud/heart/absorbing_hardcore_full_blinking",
            "hud/heart/absorbing_hardcore_half",
            "hud/heart/absorbing_hardcore_half_blinking",
        ],
        (Some(HeartKind::Normal), _) => [
            "hud/heart/full",
            "hud/heart/full_blinking",
            "hud/heart/half",
            "hud/heart/half_blinking",
            "hud/heart/hardcore_full",
            "hud/heart/hardcore_full_blinking",
            "hud/heart/hardcore_half",
            "hud/heart/hardcore_half_blinking",
        ],
        (Some(HeartKind::Poisoned), _) => [
            "hud/heart/poisoned_full",
            "hud/heart/poisoned_full_blinking",
            "hud/heart/poisoned_half",
            "hud/heart/poisoned_half_blinking",
            "hud/heart/poisoned_hardcore_full",
            "hud/heart/poisoned_hardcore_full_blinking",
            "hud/heart/poisoned_hardcore_half",
            "hud/heart/poisoned_hardcore_half_blinking",
        ],
        (Some(HeartKind::Withered), _) => [
            "hud/heart/withered_full",
            "hud/heart/withered_full_blinking",
            "hud/heart/withered_half",
            "hud/heart/withered_half_blinking",
            "hud/heart/withered_hardcore_full",
            "hud/heart/withered_hardcore_full_blinking",
            "hud/heart/withered_hardcore_half",
            "hud/heart/withered_hardcore_half_blinking",
        ],
        (Some(HeartKind::Frozen), _) => [
            "hud/heart/frozen_full",
            "hud/heart/frozen_full_blinking",
            "hud/heart/frozen_half",
            "hud/heart/frozen_half_blinking",
            "hud/heart/frozen_hardcore_full",
            "hud/heart/frozen_hardcore_full_blinking",
            "hud/heart/frozen_hardcore_half",
            "hud/heart/frozen_hardcore_half_blinking",
        ],
    };
    table[usize::from(hardcore) * 4 + usize::from(half) * 2 + usize::from(blink)]
}

fn ceil(x: f32) -> i32 {
    // Mth.ceil.
    let i = x as i32;
    if x > i as f32 { i + 1 } else { i }
}

impl HealthHud {
    /// `Hud.extractPlayerHealth` for a screen of `w` x `h` GUI pixels.
    pub fn layout(&mut self, i: &HealthInputs, w: i32, h: i32) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let tick = i64::from(i.tick);
        let current = ceil(i.health);
        let blink = self.health_blink_time > tick && (self.health_blink_time - tick) / 3 % 2 == 1;
        if current < self.last_health && i.damage_cooldown {
            self.last_health_time = i.millis;
            self.health_blink_time = tick + 20;
        } else if current > self.last_health && i.damage_cooldown {
            self.last_health_time = i.millis;
            self.health_blink_time = tick + 10;
        }
        if i.millis - self.last_health_time > 1000 {
            self.display_health = current;
            self.last_health_time = i.millis;
        }
        self.last_health = current;
        let old = self.display_health;
        let mut random = LegacyRandom::new(i64::from(i.tick.wrapping_mul(312_871)));
        let mut next = |bound: i32| random.next_int_bound(bound).unwrap_or(0);
        let (x_left, x_right, y_base) = (w / 2 - 91, w / 2 + 91, h - 39);
        let max_health = i.max_health.max(old.max(current) as f32);
        let absorption = ceil(i.absorption);
        let rows = ceil((max_health + absorption as f32) / 2.0 / 10.0);
        let row_height = (10 - (rows - 2)).max(3);
        let mut y_air = y_base - 10;
        let offset_index = if i.regeneration {
            i.tick % ceil(max_health + 5.0)
        } else {
            -1
        };
        if i.armor > 0 {
            let y = y_base - (rows - 1) * row_height - 10;
            for n in 0..10 {
                let x = x_left + n * 8;
                if n * 2 + 1 < i.armor {
                    out.push(sprite("hud/armor_full", x, y, 9, 9));
                }
                if n * 2 + 1 == i.armor {
                    out.push(sprite("hud/armor_half", x, y, 9, 9));
                }
                if n * 2 + 1 > i.armor {
                    out.push(sprite("hud/armor_empty", x, y, 9, 9));
                }
            }
        }
        let containers = f64::from(max_health / 2.0).ceil() as i32;
        let absorb_containers = (f64::from(absorption) / 2.0).ceil() as i32;
        for c in (0..containers + absorb_containers).rev() {
            let (row, column) = (c / 10, c % 10);
            let x = x_left + column * 8;
            let mut y = y_base - row * row_height;
            if current + absorption <= 4 {
                y += next(2);
            }
            if c < containers && c == offset_index {
                y -= 2;
            }
            out.push(sprite(
                heart(None, false, i.hardcore, false, blink),
                x,
                y,
                9,
                9,
            ));
            let halves = c * 2;
            if c >= containers {
                let ah = halves - containers * 2;
                if ah < absorption {
                    let withered = i.kind == HeartKind::Withered;
                    let name = heart(
                        Some(i.kind),
                        !withered,
                        i.hardcore,
                        ah + 1 == absorption,
                        false,
                    );
                    out.push(sprite(name, x, y, 9, 9));
                }
            }
            if blink && halves < old {
                let name = heart(Some(i.kind), false, i.hardcore, halves + 1 == old, true);
                out.push(sprite(name, x, y, 9, 9));
            }
            if halves < current {
                let name = heart(
                    Some(i.kind),
                    false,
                    i.hardcore,
                    halves + 1 == current,
                    false,
                );
                out.push(sprite(name, x, y, 9, 9));
            }
        }
        if i.vehicle_hearts == 0 {
            let (empty, half, full) = if i.hunger {
                (
                    "hud/food_empty_hunger",
                    "hud/food_half_hunger",
                    "hud/food_full_hunger",
                )
            } else {
                ("hud/food_empty", "hud/food_half", "hud/food_full")
            };
            for n in 0..10 {
                let mut y = y_base;
                if i.saturation <= 0.0 && i.tick % (i.food * 3 + 1) == 0 {
                    y += next(3) - 1;
                }
                let x = x_right - n * 8 - 9;
                out.push(sprite(empty, x, y, 9, 9));
                if n * 2 + 1 < i.food {
                    out.push(sprite(full, x, y, 9, 9));
                }
                if n * 2 + 1 == i.food {
                    out.push(sprite(half, x, y, 9, 9));
                }
            }
            y_air -= 10;
        }
        // Hud.extractAirBubbles.
        let max_air = i.max_air.max(1);
        let air = i.air.clamp(0, max_air);
        if i.under_water || air < max_air {
            let rows = (f64::from(i.vehicle_hearts) / 10.0).ceil() as i32;
            let y = y_air - (rows - 1) * 10;
            let bubble = |offset: i32| ceil(((air + offset) * 10) as f32 / max_air as f32);
            let full = bubble(-2);
            let popping = bubble(0);
            let delay = i32::from(air != 0 && i.under_water);
            let empty = 10 - bubble(delay);
            for b in 1..=10 {
                let x = x_right - (b - 1) * 8 - 9;
                if b <= full {
                    out.push(sprite("hud/air", x, y, 9, 9));
                } else if full != popping && b == popping && i.under_water {
                    out.push(sprite("hud/air_bursting", x, y, 9, 9));
                } else if b > 10 - empty {
                    let wobble = if empty == 10 && i.tick % 2 == 0 {
                        next(2)
                    } else {
                        0
                    };
                    out.push(sprite("hud/air_empty", x, y + wobble, 9, 9));
                }
            }
        }
        out
    }
}

/// Draws the sprite and text calls with the HUD batch; items are left to the
/// caller (they need the item renderer).
pub fn paint(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, draws: &[GuiDraw]) {
    for d in draws {
        match d {
            GuiDraw::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
            } => {
                let Some(mut r) = atlas.sprites.get(&format!("gui/sprites/{sprite}")).copied()
                else {
                    continue;
                };
                if let Some([sw, sh, u, v]) = *region {
                    let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                    let u0 = r.u0 + du * u as f32 / sw as f32;
                    let v0 = r.v0 + dv * v as f32 / sh as f32;
                    r.u1 = u0 + du * *w as f32 / sw as f32;
                    r.v1 = v0 + dv * *h as f32 / sh as f32;
                    r.u0 = u0;
                    r.v0 = v0;
                }
                batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, u32::MAX);
            }
            GuiDraw::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => {
                let [a, r, g, b] = color.to_be_bytes();
                let rgba = crate::hud::rgba(r, g, b, a);
                batch.text(atlas, text, *x as f32, *y as f32, rgba, *shadow);
            }
            GuiDraw::Item { .. } => {}
        }
    }
}
