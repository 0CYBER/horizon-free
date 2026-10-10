//! `Hud.extractPlayerHealth`: armour, hearts, food. The layout is computed as
//! a list of sprites so it can be tested without drawing.
use std::time::Instant;

/// `java.util.Random`, for the HUD's per-tick jitter.
pub(super) struct JavaRandom(i64);
impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        Self((seed ^ 0x5_DEEC_E66D) & ((1 << 48) - 1))
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5_DEEC_E66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i32
    }
    pub fn next_int(&mut self, bound: i32) -> i32 {
        let mut r = self.next(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((i64::from(bound) * i64::from(r)) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.next(31);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HeartKind {
    Normal,
    Poisoned,
    Withered,
    Absorbing,
}
impl HeartKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Normal => "",
            Self::Poisoned => "poisoned_",
            Self::Withered => "withered_",
            Self::Absorbing => "absorbing_",
        }
    }
}

/// State kept between frames (`Hud.lastHealth`, `displayHealth`, ...).
pub(super) struct HealthHud {
    last_health: i32,
    display_health: i32,
    last_health_time: Instant,
    health_blink_time: i64,
}
impl Default for HealthHud {
    fn default() -> Self {
        Self {
            last_health: 0,
            display_health: 0,
            last_health_time: Instant::now(),
            health_blink_time: 0,
        }
    }
}

pub(super) struct HealthInputs {
    pub tick: u64,
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
    /// A mount with its own hearts replaces the food bar.
    pub show_food: bool,
}

fn heart_sprite(kind: HeartKind, hardcore: bool, half: bool, blink: bool) -> String {
    let (hc, h) = (
        if hardcore { "hardcore_" } else { "" },
        if half { "half" } else { "full" },
    );
    let b = if blink { "_blinking" } else { "" };
    format!("gui/sprites/hud/heart/{}{hc}{h}{b}", kind.prefix())
}

impl HealthHud {
    /// Sprites (name, x, y) in draw order for a screen of `w` x `h` GUI pixels.
    pub fn layout(
        &mut self,
        i: &HealthInputs,
        w: i32,
        h: i32,
        now: Instant,
    ) -> Vec<(String, i32, i32)> {
        let mut out = Vec::new();
        let tick = i64::try_from(i.tick).unwrap_or(i64::MAX);
        let current = i.health.ceil() as i32;
        let blink = self.health_blink_time > tick && (self.health_blink_time - tick) / 3 % 2 == 1;
        if current < self.last_health && i.damage_cooldown {
            self.last_health_time = now;
            self.health_blink_time = tick + 20;
        } else if current > self.last_health && i.damage_cooldown {
            self.last_health_time = now;
            self.health_blink_time = tick + 10;
        }
        if now.duration_since(self.last_health_time).as_millis() > 1000 {
            self.display_health = current;
            self.last_health_time = now;
        }
        self.last_health = current;
        let old = self.display_health;
        let mut random = JavaRandom::new(i64::from((i.tick as i32).wrapping_mul(312_871)));
        let (x_left, x_right, y_base) = (w / 2 - 91, w / 2 + 91, h - 39);
        let max_health = i.max_health.max(old.max(current) as f32);
        let absorption = i.absorption.ceil() as i32;
        let rows = ((max_health + absorption as f32) / 2.0 / 10.0).ceil() as i32;
        let row_height = (10 - (rows - 2)).max(3);
        let offset_index = if i.regeneration {
            (i.tick as i32).rem_euclid((max_health + 5.0).ceil() as i32)
        } else {
            -1
        };
        if i.armor > 0 {
            let y = y_base - (rows - 1) * row_height - 10;
            for n in 0..10 {
                let name = match (n * 2 + 1).cmp(&i.armor) {
                    std::cmp::Ordering::Less => "armor_full",
                    std::cmp::Ordering::Equal => "armor_half",
                    std::cmp::Ordering::Greater => "armor_empty",
                };
                out.push((format!("gui/sprites/hud/{name}"), x_left + n * 8, y));
            }
        }
        let containers = (max_health / 2.0).ceil() as i32;
        let absorb_containers = (absorption as f32 / 2.0).ceil() as i32;
        let container = |hardcore: bool, blink: bool| {
            format!(
                "gui/sprites/hud/heart/container{}{}",
                if hardcore { "_hardcore" } else { "" },
                if blink { "_blinking" } else { "" }
            )
        };
        for c in (0..containers + absorb_containers).rev() {
            let (row, column) = (c / 10, c % 10);
            let x = x_left + column * 8;
            let mut y = y_base - row * row_height;
            if current + absorption <= 4 {
                y += random.next_int(2);
            }
            if c < containers && c == offset_index {
                y -= 2;
            }
            out.push((container(i.hardcore, blink), x, y));
            let halves = c * 2;
            if c >= containers {
                let ah = halves - containers * 2;
                if ah < absorption {
                    let kind = if i.kind == HeartKind::Withered {
                        i.kind
                    } else {
                        HeartKind::Absorbing
                    };
                    out.push((
                        heart_sprite(kind, i.hardcore, ah + 1 == absorption, false),
                        x,
                        y,
                    ));
                }
            }
            if blink && halves < old {
                out.push((
                    heart_sprite(i.kind, i.hardcore, halves + 1 == old, true),
                    x,
                    y,
                ));
            }
            if halves < current {
                out.push((
                    heart_sprite(i.kind, i.hardcore, halves + 1 == current, false),
                    x,
                    y,
                ));
            }
        }
        if i.show_food {
            let suffix = if i.hunger { "_hunger" } else { "" };
            for n in 0..10 {
                let mut y = y_base;
                if i.saturation <= 0.0 && (i.tick as i32).rem_euclid(i.food * 3 + 1) == 0 {
                    y += random.next_int(3) - 1;
                }
                let x = x_right - n * 8 - 9;
                out.push((format!("gui/sprites/hud/food_empty{suffix}"), x, y));
                if n * 2 + 1 < i.food {
                    out.push((format!("gui/sprites/hud/food_full{suffix}"), x, y));
                }
                if n * 2 + 1 == i.food {
                    out.push((format!("gui/sprites/hud/food_half{suffix}"), x, y));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_matches_reference() {
        // new Random(42).nextInt(10) sequence: 0, 3, 8, 4, 0.
        let mut r = JavaRandom::new(42);
        let v: Vec<i32> = (0..5).map(|_| r.next_int(10)).collect();
        assert_eq!(v, [0, 3, 8, 4, 0]);
    }
}
