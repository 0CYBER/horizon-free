use crate::game::Game;
use horizon_client::command::Context;
use horizon_gameplay::commands::worldborder::WorldborderHost;

impl WorldborderHost for Game<'_> {
    fn border_size(&self) -> f64 {
        self.border.size()
    }
    fn border_remaining_ticks(&self) -> i64 {
        self.border.remaining_ticks()
    }
    fn border_lerp_to(&mut self, to: f64, ticks: i64) {
        self.border.lerp_to(to, ticks);
    }
    fn border_set_size(&mut self, size: f64) {
        self.border.set_size(size);
    }
    fn border_bounds_changed(&mut self) {
        self.native_world_border_bounds_changed();
    }
    fn border_center(&self) -> [f64; 2] {
        self.border.center()
    }
    fn border_set_center(&mut self, center: [f64; 2]) {
        self.border.set_center(center);
    }
    fn border_center_changed(&mut self) {
        self.native_world_border_center_changed();
    }
    fn position_arg_or_source(&self, ctx: &Context, name: &str) -> [f64; 3] {
        self.position_arg(ctx, name)
            .unwrap_or_else(|| self.source_position())
    }
    fn border_damage_slot(&self, amount: bool) -> f64 {
        if amount {
            self.border.damage_per_block
        } else {
            self.border.safe_zone
        }
    }
    fn set_border_damage_slot(&mut self, amount: bool, value: f64) {
        if amount {
            self.border.damage_per_block = value;
        } else {
            self.border.safe_zone = value;
        }
    }
    fn border_warning_distance(&self) -> i32 {
        self.border.warning_distance
    }
    fn set_border_warning_distance(&mut self, distance: i32) {
        self.border.warning_distance = distance;
    }
    fn border_warning_time(&self) -> i32 {
        self.border.warning_time
    }
    fn set_border_warning_time(&mut self, ticks: i32) {
        self.border.warning_time = ticks;
    }
}
