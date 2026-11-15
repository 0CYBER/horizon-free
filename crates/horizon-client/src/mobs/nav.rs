//! `PathNavigation` and its ground, wall-climbing, water-bound, flying and
//! amphibious variants: path creation, waypoint following, corner cutting,
//! stuck detection and path timeouts.
use super::{
    Mob, MobWorld,
    path::{Evaluator, Finder, Path, PathMob, PathNode, PathType},
};
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NavKind {
    #[default]
    Ground,
    WallClimber,
    Water {
        breach: bool,
    },
    Flying,
    /// `AmphibiousPathNavigation` (`Axolotl`, `Drowned`, `Turtle`, and the
    /// `Frog`, whose `FrogPathNavigation.prefersShallowSwimming` costs the
    /// water nodes deeper than ten blocks under sea level.
    Amphibious {
        prefers_shallow: bool,
    },
}
#[derive(Clone, Debug)]
pub struct Navigation {
    pub kind: NavKind,
    pub path: Option<Path>,
    pub speed_modifier: f64,
    tick: i64,
    last_stuck_check: i64,
    last_stuck_pos: Vec3,
    timeout_node: [i32; 3],
    timeout_timer: i64,
    last_timeout_check: i64,
    timeout_limit: f64,
    pub max_distance_to_waypoint: f32,
    delayed_recompute: bool,
    /// `ServerLevel.sendBlockUpdated` asked for a recompute: made at the next tick.
    pending_recompute: bool,
    time_last_recompute: i64,
    pub target_pos: Option<[i32; 3]>,
    reach_range: i32,
    pub max_visited_multiplier: f32,
    pub stuck: bool,
    /// Set through [`Mob::set_required_path_length`].
    pub required_path_length: f32,
    /// `PathFinder.maxVisitedNodes`: the base follow range times 16 when the
    /// navigation is made, then [`Mob::update_max_visited_nodes`].
    pub max_visited_nodes: i32,
    pub can_float: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub can_walk_over_fences: bool,
    pub avoid_sun: bool,
    pub below_surface: bool,
    /// WallClimberNavigation.pathToPosition
    climb_to: Option<[i32; 3]>,
}
/// A controlling passenger uses the vehicle's actual navigation, but the
/// passenger's private navigation still ticks in Mob.serverAiStep.
#[derive(Clone, Debug)]
pub(super) struct Redirect {
    pub own: Navigation,
    pub body: PathMob,
    pub follow_range: f32,
    pub passenger: bool,
    pub in_liquid: bool,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            kind: NavKind::Ground,
            path: None,
            speed_modifier: 0.0,
            tick: 0,
            last_stuck_check: 0,
            last_stuck_pos: Vec3::ZERO,
            timeout_node: [0; 3],
            timeout_timer: 0,
            last_timeout_check: 0,
            timeout_limit: 0.0,
            max_distance_to_waypoint: 0.5,
            delayed_recompute: false,
            pending_recompute: false,
            time_last_recompute: 0,
            target_pos: None,
            reach_range: 0,
            max_visited_multiplier: 1.0,
            stuck: false,
            required_path_length: 16.0,
            max_visited_nodes: 256,
            can_float: false,
            can_open_doors: false,
            can_pass_doors: true,
            can_walk_over_fences: false,
            avoid_sun: false,
            below_surface: false,
            climb_to: None,
        }
    }
}
impl Navigation {
    pub fn is_done(&self) -> bool {
        self.path.as_ref().is_none_or(Path::is_done)
    }
    pub fn is_in_progress(&self) -> bool {
        !self.is_done()
    }
    pub fn stop(&mut self) {
        self.path = None;
    }
    /// A mob whose AI step a mod paused (`mobs::ai_pause`) keeps its path,
    /// and `doStuckDetection`'s timeout clock does not count the pause: on
    /// resume it adds one tick, not the paused game time.
    pub(super) fn hold_timeout_clock(&mut self, game_time: i64) {
        if self.path.as_ref().is_some_and(|p| !p.is_done()) {
            self.last_timeout_check = game_time;
        }
    }
    fn reset_stuck_timeout(&mut self) {
        self.timeout_node = [0; 3];
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.stuck = false;
    }
    fn evaluator(&self) -> Evaluator {
        match self.kind {
            NavKind::Ground | NavKind::WallClimber => Evaluator::Walk,
            NavKind::Water { breach } => Evaluator::Swim { breach },
            NavKind::Flying => Evaluator::Fly,
            NavKind::Amphibious { prefers_shallow } => Evaluator::Amphibious { prefers_shallow },
        }
    }
    pub fn can_cut_corner(t: PathType) -> bool {
        !matches!(
            t,
            PathType::FireInNeighbor | PathType::DamagingInNeighbor | PathType::WalkableDoor
        )
    }
}
/// `LivingEntity.getComfortableFallDistance`: the drop an entity accepts on
/// top of what it was asked for.
fn comfortable_fall_distance(f: f32) -> i32 {
    (f + 3.0).floor() as i32
}
impl Mob {
    pub fn path_mob(&self) -> PathMob {
        if let Some(redirect) = &self.ai.navigation_redirect {
            return redirect.body.clone();
        }
        PathMob {
            position: self.position,
            width: self.width(),
            height: self.height(),
            on_ground: self.on_ground,
            in_water: self.fluids.in_water,
            max_up_step: self.kind.step_height as f32,
            max_fall_distance: self.max_fall_distance(),
            malus: self.ai.malus,
            can_float: self.ai.nav.can_float,
            can_open_doors: self.ai.nav.can_open_doors,
            can_pass_doors: self.ai.nav.can_pass_doors,
            can_walk_over_fences: self.ai.nav.can_walk_over_fences,
            stands_on_lava: self.kind.flags.lava_walker,
        }
    }
    /// `Mob.getMaxFallDistance` (normal difficulty), with `Creeper`'s
    /// override: a creeper that has a target falls by its own health, so a
    /// full-health one walks off a 22-block drop instead of 3.
    pub fn max_fall_distance(&self) -> i32 {
        if self.ai.target.is_none() {
            return comfortable_fall_distance(0.0);
        }
        if self.species() == super::Species::Creeper {
            return comfortable_fall_distance(self.health - 1.0);
        }
        let mut sacrifice = (self.health - self.attrs.max_health * 0.33) as i32;
        sacrifice -= 4;
        comfortable_fall_distance(sacrifice.max(0) as f32)
    }
    pub fn malus(&self, t: PathType) -> f32 {
        self.ai.malus[t as usize].unwrap_or_else(|| t.malus())
    }
    pub fn set_malus(&mut self, t: PathType, v: f32) {
        self.ai.malus[t as usize] = Some(v);
    }
    fn navigation_position(&self) -> Vec3 {
        self.ai
            .navigation_redirect
            .as_ref()
            .map_or(self.position, |r| r.body.position)
    }
    fn navigation_height(&self) -> f32 {
        self.ai
            .navigation_redirect
            .as_ref()
            .map_or_else(|| self.height(), |r| r.body.height)
    }
    fn navigation_in_water(&self) -> bool {
        self.ai
            .navigation_redirect
            .as_ref()
            .map_or(self.fluids.in_water, |r| r.body.in_water)
    }
    /// PathNavigation.getMaxPathLength.
    fn max_path_length(&self) -> f32 {
        self.ai
            .navigation_redirect
            .as_ref()
            .map_or(self.attrs.follow_range as f32, |r| r.follow_range)
            .max(self.ai.nav.required_path_length)
    }
    /// `PathNavigation.updatePathfinderMaxVisitedNodes`: after a follow range
    /// change and a new required path length.
    pub fn update_max_visited_nodes(&mut self) {
        self.ai.nav.max_visited_nodes = (self.max_path_length() * 16.0).floor() as i32;
    }
    /// `PathNavigation.setRequiredPathLength`.
    pub fn set_required_path_length(&mut self, length: f32) {
        self.ai.nav.required_path_length = length;
        self.update_max_visited_nodes();
    }
    fn can_update_path(&self) -> bool {
        match self.ai.nav.kind {
            // GroundPathNavigation.canUpdatePath: onGround() || isInLiquid() ||
            // isPassenger(). A mob riding a vehicle keeps making and following
            // paths where it sits (`create_path_raw` and `recompute_path` refuse
            // without this).
            NavKind::Ground | NavKind::WallClimber => {
                self.ai.navigation_redirect.as_ref().map_or_else(
                    || self.on_ground || self.is_in_liquid() || self.extra.mount.is_some(),
                    |r| r.body.on_ground || r.in_liquid || r.passenger,
                )
            }
            NavKind::Water { breach } => {
                breach
                    || self
                        .ai
                        .navigation_redirect
                        .as_ref()
                        .map_or_else(|| self.is_in_liquid(), |r| r.in_liquid)
            }
            NavKind::Flying | NavKind::Amphibious { .. } => true,
        }
    }
    fn temp_mob_pos(&self, w: &MobWorld) -> Vec3 {
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => Vec3::new(
                self.navigation_position().x,
                f64::from(self.surface_y(w)),
                self.navigation_position().z,
            ),
            NavKind::Water { .. } | NavKind::Amphibious { .. } => Vec3::new(
                self.navigation_position().x,
                self.navigation_position().y + f64::from(self.navigation_height()) * 0.5,
                self.navigation_position().z,
            ),
            NavKind::Flying => self.navigation_position(),
        }
    }
    fn surface_y(&self, w: &MobWorld) -> i32 {
        let (x, z) = (
            self.navigation_position().x.floor() as i32,
            self.navigation_position().z.floor() as i32,
        );
        if self.navigation_in_water() && self.ai.nav.can_float {
            let mut y = self.navigation_position().y.floor() as i32;
            let start = y;
            let mut steps = 0;
            while w.facts.get(w.world.block(x, y, z)).fluid == crate::blocks::Fluid::Water {
                y += 1;
                steps += 1;
                if steps > 16 {
                    return start;
                }
            }
            y
        } else {
            (self.navigation_position().y + 0.5).floor() as i32
        }
    }
    /// GroundPathNavigation.findSurfacePosition.
    fn surface_position(&self, w: &MobWorld, mut p: [i32; 3]) -> [i32; 3] {
        let shape = w.world.shape;
        let min_y = shape.min_y;
        let max_y = shape.max_y() - 1;
        let air = |p: [i32; 3]| {
            !shape.contains_y(p[1]) || w.facts.is_air(w.world.block(p[0], p[1], p[2]))
        };
        let solid = |p: [i32; 3]| shape.contains_y(p[1]) && w.solid(p[0], p[1], p[2]);
        if air(p) {
            let mut q = [p[0], (p[1] - 1).min(max_y), p[2]];
            while q[1] >= min_y && air(q) {
                q[1] -= 1;
            }
            if q[1] >= min_y {
                return [q[0], q[1] + 1, q[2]];
            }
            q[1] = (p[1] + 1).max(min_y);
            while q[1] <= max_y && air(q) {
                q[1] += 1;
            }
            p = q;
        }
        if !solid(p) {
            return p;
        }
        let mut q = [p[0], p[1] + 1, p[2]];
        while q[1] <= max_y && solid(q) {
            q[1] += 1;
        }
        q
    }
    pub(super) fn create_path_raw(
        &mut self,
        w: &MobWorld,
        targets: &[[i32; 3]],
        reach: i32,
        max_len: f32,
    ) -> Option<Path> {
        if targets.is_empty()
            || self.navigation_position().y < f64::from(w.world.bottom_y())
            || !self.can_update_path()
        {
            return None;
        }
        if let (Some(path), Some(tp)) = (&self.ai.nav.path, self.ai.nav.target_pos)
            && !path.is_done()
            && targets.contains(&tp)
        {
            return Some(path.clone());
        }
        let pm = self.path_mob();
        let max_visited =
            (self.ai.nav.max_visited_nodes as f32 * self.ai.nav.max_visited_multiplier) as i32;
        let finder = Finder::new(w, &pm, self.ai.nav.evaluator());
        let path = finder.find(targets, max_len, reach, max_visited);
        if let Some(p) = &path {
            self.ai.nav.target_pos = Some(p.target);
            self.ai.nav.reach_range = reach;
            self.ai.nav.reset_stuck_timeout();
        }
        path
    }
    /// PathNavigation.createPath(BlockPos, reach) with the ground surface search.
    pub fn create_path_to(&mut self, w: &MobWorld, mut p: [i32; 3], reach: i32) -> Option<Path> {
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => {
                if !w.world.loaded(p[0], p[2]) {
                    return None;
                }
                if self.ai.nav.kind == NavKind::WallClimber {
                    self.ai.nav.climb_to = Some(p);
                }
                if !self.ai.nav.below_surface {
                    p = self.surface_position(w, p);
                }
            }
            _ => {}
        }
        let max = self.max_path_length();
        self.create_path_raw(w, &[p], reach, max)
    }
    /// PathNavigation.createPath(Set<BlockPos>, reach): the nearest of
    /// several targets, without the ground surface search.
    pub fn create_path_to_any(
        &mut self,
        w: &MobWorld,
        targets: &[[i32; 3]],
        reach: i32,
    ) -> Option<Path> {
        let max = self.max_path_length();
        self.create_path_raw(w, targets, reach, max)
    }
    /// PathNavigation.createPath(Entity, reach).
    pub fn create_path_to_entity(
        &mut self,
        w: &MobWorld,
        at: [f64; 3],
        reach: i32,
    ) -> Option<Path> {
        let p = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        self.create_path_to(w, p, reach)
    }
    /// `PathNavigation.moveTo(Path, speed)`: `trimPath` runs before the
    /// node count is checked, so a path the trim empties is refused.
    pub fn nav_move_to_path(&mut self, w: &MobWorld, path: Option<Path>, speed: f64) -> bool {
        let Some(path) = path else {
            self.ai.nav.path = None;
            return false;
        };
        if !path.same_as(self.ai.nav.path.as_ref()) {
            self.ai.nav.path = Some(path);
        }
        if self.ai.nav.is_done() {
            return false;
        }
        self.trim_path(w);
        if self.ai.nav.path.as_ref().is_none_or(|p| p.nodes.is_empty()) {
            return false;
        }
        self.ai.nav.speed_modifier = speed;
        self.ai.nav.last_stuck_check = self.ai.nav.tick;
        // getTempMobPos needs the world for ground navigation; position is close enough
        // for the first stuck sample and is refreshed 100 ticks later.
        self.ai.nav.last_stuck_pos = self.navigation_position();
        true
    }
    /// `PathNavigation.moveTo(Path, speed)` for a path a mod computed
    /// (`MobRef::move_to_path`): each position is floored to its block and
    /// classified here, one cached static lookup per node, then the path is
    /// trimmed and followed exactly as `moveTo(Path)` follows an engine path.
    ///
    /// A supplied path keeps no recompute target, so a block change on it or
    /// the stuck check drops it and no search is run in its place (vanilla
    /// would re-search to the previous target; this is the stricter rule the
    /// issue asks for). The per-node timeout drops it as it drops any path.
    pub fn nav_move_to_supplied(&mut self, w: &MobWorld, nodes: &[[f64; 3]], speed: f64) -> bool {
        let nodes: Vec<PathNode> = nodes
            .iter()
            .map(|at| PathNode {
                x: at[0].floor() as i32,
                y: at[1].floor() as i32,
                z: at[2].floor() as i32,
                // Replaced by the engine's classification below.
                kind: PathType::Blocked,
            })
            .collect();
        let target = nodes.last().map_or([0; 3], |n| [n.x, n.y, n.z]);
        let mut path = Path {
            nodes,
            next: 0,
            target,
            dist_to_target: 0.0,
            reached: true,
        };
        self.classify_path_nodes(w, &mut path);
        // Our rule, not vanilla's: a supplied path stores no recompute target,
        // so a block change on it drops it instead of re-searching.
        self.ai.nav.target_pos = None;
        self.ai.nav.reach_range = 0;
        self.ai.nav.pending_recompute = false;
        self.ai.nav.delayed_recompute = false;
        // A supplied path supersedes any half-finished wall climb to an old
        // entity target.
        self.ai.nav.climb_to = None;
        self.ai.nav.reset_stuck_timeout();
        self.nav_move_to_path(w, Some(path), speed)
    }
    /// Classify every node of a supplied path, through this mob's navigation
    /// evaluator and the world's cached classification. The result is
    /// information for the follower (the corner-cut rule), never a filter.
    fn classify_path_nodes(&self, w: &MobWorld, path: &mut Path) {
        if path.nodes.is_empty() {
            return;
        }
        let body = self.path_mob();
        let finder = Finder::new(w, &body, self.ai.nav.evaluator());
        for node in &mut path.nodes {
            node.kind = finder.path_type_at(node.x, node.y, node.z);
        }
    }
    /// PathNavigation.moveTo(x, y, z, speed): a path to the containing block.
    pub fn nav_move_to(&mut self, w: &MobWorld, at: [f64; 3], speed: f64) -> bool {
        let p = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        let path = self.create_path_to(w, p, 1);
        self.nav_move_to_path(w, path, speed)
    }
    pub fn nav_move_to_entity(
        &mut self,
        w: &MobWorld,
        at: [f64; 3],
        reach: i32,
        speed: f64,
    ) -> bool {
        let path = self.create_path_to_entity(w, at, reach);
        if self.ai.nav.kind == NavKind::WallClimber && path.is_none() {
            // WallClimberNavigation.moveTo(Entity): climb straight towards it.
            self.ai.nav.climb_to = Some([
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ]);
            self.ai.nav.speed_modifier = speed;
            return true;
        }
        path.is_some() && self.nav_move_to_path(w, path, speed)
    }
    pub fn nav_stop(&mut self) {
        self.ai.nav.path = None;
    }
    fn recompute_path(&mut self, w: &MobWorld) {
        // Our rule for a path a mod supplied (it keeps no recompute target):
        // a block change on it, or anything else that asks for a recompute,
        // drops it. No search is ever run in its place, so a stale target is
        // never revived.
        if self.ai.nav.target_pos.is_none() {
            self.ai.nav.path = None;
            self.ai.nav.delayed_recompute = false;
            self.ai.nav.time_last_recompute = w.game_time;
            return;
        }
        if w.game_time - self.ai.nav.time_last_recompute <= 20 || !self.can_update_path() {
            self.ai.nav.delayed_recompute = true;
        } else if let Some(t) = self.ai.nav.target_pos {
            self.ai.nav.path = None;
            let reach = self.ai.nav.reach_range;
            self.ai.nav.path = self.create_path_to(w, t, reach);
            self.ai.nav.time_last_recompute = w.game_time;
            self.ai.nav.delayed_recompute = false;
        }
    }
    /// `PathNavigation.trimPath` for cauldrons, then
    /// `GroundPathNavigation.trimPath` with `avoidSun`: a mob that is out of the
    /// sun keeps only the nodes before the first one the sky sees.
    fn trim_path(&mut self, w: &MobWorld) {
        if let Some(path) = &mut self.ai.nav.path {
            path.raise_cauldron_nodes(|x, y, z| {
                w.has_tag("minecraft:cauldrons", w.block_name(x, y, z))
            });
        }
        if !self.ai.nav.avoid_sun
            || !matches!(self.ai.nav.kind, NavKind::Ground | NavKind::WallClimber)
        {
            return;
        }
        let at = [
            self.navigation_position().x.floor() as i32,
            (self.navigation_position().y + 0.5).floor() as i32,
            self.navigation_position().z.floor() as i32,
        ];
        if w.can_see_sky(at[0], at[1], at[2]) {
            return;
        }
        if let Some(path) = &mut self.ai.nav.path
            && let Some(i) = path.nodes.iter().position(|n| w.can_see_sky(n.x, n.y, n.z))
        {
            path.nodes.truncate(i);
        }
    }
    /// `PathNavigation.shouldRecomputePath`: a block changed at `pos` near the
    /// middle of what is left of the path.
    pub fn should_recompute_path(&self, pos: [i32; 3]) -> bool {
        let nav = &self.ai.nav;
        if nav.delayed_recompute {
            return false;
        }
        let Some(path) = nav.path.as_ref().filter(|p| !p.is_done()) else {
            return false;
        };
        let Some(end) = path.nodes.last() else {
            return false;
        };
        let middle = Vec3::new(
            (f64::from(end.x) + self.navigation_position().x) / 2.0,
            (f64::from(end.y) + self.navigation_position().y) / 2.0,
            (f64::from(end.z) + self.navigation_position().z) / 2.0,
        );
        super::doors::closer_to_center(pos, middle, (path.nodes.len() - path.next) as f64)
    }
    /// `ServerLevel.sendBlockUpdated` for a mob that should re-path: the
    /// recompute happens at the start of its next navigation tick.
    pub fn queue_path_recompute(&mut self) {
        self.ai.nav.pending_recompute = true;
    }
    /// Tick the private field while goal access is redirected to the vehicle.
    pub fn nav_tick(&mut self, w: &MobWorld) {
        let mut redirect = self.ai.navigation_redirect.take();
        if let Some(r) = &mut redirect {
            std::mem::swap(&mut self.ai.nav, &mut r.own);
        }
        self.tick_own_navigation(w);
        if let Some(r) = &mut redirect {
            std::mem::swap(&mut self.ai.nav, &mut r.own);
        }
        self.ai.navigation_redirect = redirect;
    }
    /// PathNavigation.tick.
    fn tick_own_navigation(&mut self, w: &MobWorld) {
        self.ai.nav.tick += 1;
        if std::mem::take(&mut self.ai.nav.pending_recompute) {
            self.recompute_path(w);
        }
        if self.ai.nav.delayed_recompute {
            self.recompute_path(w);
        }
        if self.ai.nav.kind == NavKind::WallClimber && self.ai.nav.is_done() {
            if let Some(t) = self.ai.nav.climb_to {
                let width = f64::from(self.width());
                let center = |p: [i32; 3]| {
                    let (dx, dy, dz) = (
                        f64::from(p[0]) + 0.5 - self.position.x,
                        f64::from(p[1]) + 0.5 - self.position.y,
                        f64::from(p[2]) + 0.5 - self.position.z,
                    );
                    dx * dx + dy * dy + dz * dz < width * width
                };
                let same_level = [t[0], self.position.y.floor() as i32, t[2]];
                if !center(t) && (self.position.y <= f64::from(t[1]) || !center(same_level)) {
                    let speed = self.ai.nav.speed_modifier;
                    self.ai.mov.set_wanted(
                        f64::from(t[0]),
                        f64::from(t[1]),
                        f64::from(t[2]),
                        speed,
                    );
                } else {
                    self.ai.nav.climb_to = None;
                }
            }
            return;
        }
        if self.ai.nav.is_done() {
            return;
        }
        if self.can_update_path() {
            self.follow_the_path(w);
        } else if let Some(path) = &self.ai.nav.path
            && !path.is_done()
        {
            let mob = self.temp_mob_pos(w);
            let pos = path.entity_pos_at(path.next, self.width());
            let advance = match self.ai.nav.kind {
                NavKind::Flying => {
                    self.position.x.floor() == pos.x.floor()
                        && self.position.y.floor() == pos.y.floor()
                        && self.position.z.floor() == pos.z.floor()
                }
                _ => {
                    mob.y > pos.y
                        && !self.on_ground
                        && mob.x.floor() == pos.x.floor()
                        && mob.z.floor() == pos.z.floor()
                }
            };
            if advance && let Some(p) = &mut self.ai.nav.path {
                p.advance();
            }
        }
        if let Some(path) = &self.ai.nav.path
            && !path.is_done()
        {
            let target = path.entity_pos_at(path.next, self.width());
            let y = match self.ai.nav.kind {
                NavKind::Ground | NavKind::WallClimber => {
                    let (bx, by, bz) = (
                        target.x.floor() as i32,
                        target.y.floor() as i32,
                        target.z.floor() as i32,
                    );
                    if w.facts.is_air(w.world.block(bx, by.saturating_sub(1), bz)) {
                        target.y
                    } else {
                        w.floor_level(bx, by, bz)
                    }
                }
                _ => target.y,
            };
            let speed = self.ai.nav.speed_modifier;
            self.ai.mov.set_wanted(target.x, y, target.z, speed);
        }
    }
    fn follow_the_path(&mut self, w: &MobWorld) {
        let mob_pos = self.temp_mob_pos(w);
        let width = self.width();
        self.ai.nav.max_distance_to_waypoint = if width > 0.75 {
            width / 2.0
        } else {
            0.75 - width / 2.0
        };
        let vertical = match self.ai.nav.kind {
            NavKind::Water { .. } => 0.5,
            _ => 1.0,
        };
        let Some(path) = &self.ai.nav.path else {
            return;
        };
        let n = path.next_node();
        let xd = (self.position.x - (f64::from(n.x) + 0.5)).abs();
        let yd = (self.position.y - f64::from(n.y)).abs();
        let zd = (self.position.z - (f64::from(n.z) + 0.5)).abs();
        let max = f64::from(self.ai.nav.max_distance_to_waypoint);
        let close = xd < max && zd < max && yd < vertical;
        if (close || Navigation::can_cut_corner(n.kind) && self.should_target_next_node(w, mob_pos))
            && let Some(p) = &mut self.ai.nav.path
        {
            p.advance();
        }
        self.do_stuck_detection(w, mob_pos);
    }
    // Vanilla's NaN-sensitive `!(a < b)` form.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn should_target_next_node(&self, w: &MobWorld, mob: Vec3) -> bool {
        let Some(path) = &self.ai.nav.path else {
            return false;
        };
        if path.next + 1 >= path.nodes.len() {
            return false;
        }
        let n = path.nodes[path.next];
        let current = Vec3::new(f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5);
        let d = |a: Vec3, b: Vec3| (a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2);
        if d(mob, current) >= 4.0 {
            return false;
        }
        if self.can_move_directly(w, mob, path.entity_pos_at(path.next, self.width())) {
            return true;
        }
        let m = path.nodes[path.next + 1];
        let next = Vec3::new(f64::from(m.x) + 0.5, f64::from(m.y), f64::from(m.z) + 0.5);
        let to_current = Vec3::new(current.x - mob.x, current.y - mob.y, current.z - mob.z);
        let to_next = Vec3::new(next.x - mob.x, next.y - mob.y, next.z - mob.z);
        let (cs, ns) = (to_current.length_squared(), to_next.length_squared());
        if !(ns < cs) && !(cs < 0.5) {
            return false;
        }
        let a = to_current.normalize();
        let b = to_next.normalize();
        a.x * b.x + a.y * b.y + a.z * b.z < 0.0
    }
    fn can_move_directly(&self, w: &MobWorld, from: Vec3, to: Vec3) -> bool {
        match self.ai.nav.kind {
            NavKind::Flying => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                true,
            ),
            NavKind::Water { .. } => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                false,
            ),
            NavKind::Amphibious { .. } if self.is_in_liquid() => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                false,
            ),
            _ => false,
        }
    }
    fn do_stuck_detection(&mut self, w: &MobWorld, mob: Vec3) {
        let speed = self.ai.speed;
        let nav = &mut self.ai.nav;
        if nav.tick - nav.last_stuck_check > 100 {
            let effective = if speed >= 1.0 { speed } else { speed * speed };
            let threshold = f64::from(effective * 100.0 * 0.25);
            let d = (mob.x - nav.last_stuck_pos.x).powi(2)
                + (mob.y - nav.last_stuck_pos.y).powi(2)
                + (mob.z - nav.last_stuck_pos.z).powi(2);
            if d < threshold * threshold {
                nav.stuck = true;
                nav.path = None;
            } else {
                nav.stuck = false;
            }
            nav.last_stuck_check = nav.tick;
            nav.last_stuck_pos = mob;
        }
        if let Some(path) = &nav.path
            && !path.is_done()
        {
            let n = path.next_node();
            let pos = [n.x, n.y, n.z];
            if pos == nav.timeout_node {
                nav.timeout_timer += w.game_time - nav.last_timeout_check;
            } else {
                nav.timeout_node = pos;
                let c = Vec3::new(f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5);
                let dist =
                    ((mob.x - c.x).powi(2) + (mob.y - c.y).powi(2) + (mob.z - c.z).powi(2)).sqrt();
                nav.timeout_limit = if speed > 0.0 {
                    dist / f64::from(speed) * 20.0
                } else {
                    0.0
                };
            }
            if nav.timeout_limit > 0.0 && nav.timeout_timer as f64 > nav.timeout_limit * 3.0 {
                nav.reset_stuck_timeout();
                nav.path = None;
            }
            nav.last_timeout_check = w.game_time;
        }
    }
    /// PathNavigation.isStableDestination for the mob's navigation.
    pub fn stable_destination(&self, w: &MobWorld, p: [i32; 3]) -> bool {
        if self.species() == super::Species::Bee {
            return super::bee::stable_destination(w, p);
        }
        if self.species() == super::Species::Turtle
            && let Some(stable) = super::turtle::stable_destination(self, w, p)
        {
            return stable;
        }
        // StriderPathNavigation: lava too.
        if self.kind.flags.lava_walker && w.block_name(p[0], p[1], p[2]) == "minecraft:lava" {
            return true;
        }
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => {
                w.facts
                    .get(w.world.block(p[0], p[1] - 1, p[2]))
                    .solid_render
            }
            NavKind::Water { .. } => !w.facts.get(w.world.block(p[0], p[1], p[2])).solid_render,
            NavKind::Flying => {
                w.data.state(w.world.block(p[0], p[1], p[2])).flags & super::data::STURDY_TOP != 0
            }
            NavKind::Amphibious { .. } => !w.facts.is_air(w.world.block(p[0], p[1] - 1, p[2])),
        }
    }
}

#[cfg(test)]
mod recompute_tests {
    use super::*;
    use crate::mobs::path::{Path, PathNode, PathType};

    fn walker() -> Mob {
        let kind = crate::mobs::sensor_scan_tests::kind();
        Mob::new(1, kind, Vec3::new(0.5, 0.0, 0.5), 0.0)
    }
    fn straight(len: i32) -> Path {
        Path {
            nodes: (0..len)
                .map(|x| PathNode {
                    x,
                    y: 0,
                    z: 0,
                    kind: PathType::Walkable,
                })
                .collect(),
            next: 0,
            target: [len - 1, 0, 0],
            dist_to_target: 0.0,
            reached: true,
        }
    }

    /// `PathNavigation.shouldRecomputePath`: a block within (nodes left) of the
    /// middle of the way to the path's end, and nothing while a recompute waits.
    #[test]
    fn a_change_near_the_middle_of_the_path_asks_for_a_recompute() {
        let mut m = walker();
        assert!(!m.should_recompute_path([5, 0, 0]), "no path");
        m.ai.nav.path = Some(straight(11));
        // The middle of (10, 0, 0) and (0.5, 0, 0.5) is about (5.25, 0, 0.25);
        // 11 nodes are left.
        assert!(m.should_recompute_path([5, 0, 0]));
        assert!(m.should_recompute_path([5, 5, 0]));
        assert!(!m.should_recompute_path([5, 12, 0]), "too far");
        assert!(!m.should_recompute_path([-9, 0, 0]), "behind the mob");
        m.ai.nav.delayed_recompute = true;
        assert!(
            !m.should_recompute_path([5, 0, 0]),
            "one is already waiting"
        );
        m.ai.nav.delayed_recompute = false;
        m.ai.nav.path.as_mut().unwrap().next = 11;
        assert!(!m.should_recompute_path([5, 0, 0]), "the path is done");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn surface_search_uses_the_world_build_range() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        assert!(crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, _| world.shape =
                crate::chunk::LevelShape::new(0, 128, 128, true, false).unwrap(),
            |world, types| {
                let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
                let mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
                assert_eq!(
                    mob.surface_position(world, [0, 2_000_000_000, 0]),
                    [0, 2_000_000_001, 0]
                );
                assert_eq!(
                    mob.surface_position(world, [0, -2_000_000_000, 0]),
                    [0, 128, 0]
                );
                assert_eq!(mob.surface_position(world, [0, 127, 0]), [0, 128, 0]);
                assert_eq!(mob.surface_position(world, [0, 0, 0]), [0, 128, 0]);
            },
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn surface_search_from_above_keeps_the_top_nonsolid_block() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        assert!(crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                world
                    .set(
                        0,
                        319,
                        0,
                        registry.default_state("minecraft:torch").unwrap(),
                    )
                    .unwrap();
            },
            |world, types| {
                let mob = Mob::new(
                    1,
                    std::sync::Arc::new(types.types["minecraft:zombie"].clone()),
                    Vec3::ZERO,
                    0.0,
                );
                assert_eq!(mob.surface_position(world, [0, 321, 0]), [0, 320, 0]);
                assert_eq!(mob.surface_position(world, [0, 319, 0]), [0, 319, 0]);
            },
        ));
    }

    /// `Frog.FrogPathNavigation.prefersShallowSwimming` has to reach the path
    /// finder, or the shallow-water malus never applies to a frog.
    #[test]
    fn the_amphibious_navigation_keeps_its_shallow_preference() {
        let mut nav = Navigation::default();
        assert!(
            matches!(nav.evaluator(), Evaluator::Walk),
            "ground by default"
        );
        nav.kind = NavKind::Amphibious {
            prefers_shallow: true,
        };
        assert!(matches!(
            nav.evaluator(),
            Evaluator::Amphibious {
                prefers_shallow: true
            }
        ));
        nav.kind = NavKind::Amphibious {
            prefers_shallow: false,
        };
        assert!(matches!(
            nav.evaluator(),
            Evaluator::Amphibious {
                prefers_shallow: false
            }
        ));
    }

    /// `Creeper.getMaxFallDistance` replaces the shared
    /// `health - a third` formula with its own `health - 1`: a creeper
    /// chasing something drops 22 blocks at full health and 6 at four,
    /// where the shared formula would have given 3.
    #[test]
    #[ignore = "daily"]
    fn a_creeper_falls_by_its_own_health_while_it_has_a_target() {
        // Skipped without the extracted vanilla data (CI's verify has none).
        crate::mobs::world::with_test_world(&[], |_, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:creeper"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            assert_eq!(m.max_fall_distance(), 3, "no target");
            m.ai.target = Some(crate::mobs::Who::Player);
            m.health = 20.0;
            assert_eq!(m.max_fall_distance(), 22, "full health, has a target");
            m.health = 4.0;
            assert_eq!(m.max_fall_distance(), 6, "hurt, has a target");
        });
    }

    /// `GroundPathNavigation.canUpdatePath`: `onGround() || isInLiquid() ||
    /// isPassenger()`. A pig riding a vehicle - a chicken jockey, a husk on a
    /// camel, a piglin on a hoglin - can still ask for a path and follow it.
    #[test]
    fn a_mob_riding_a_vehicle_keeps_its_ground_navigation() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        std::sync::Arc::make_mut(&mut kind).species = crate::mobs::Species::Pig;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.on_ground = true;
        assert!(m.can_update_path(), "standing on the ground");
        m.on_ground = false;
        assert!(
            !m.can_update_path(),
            "in the air with no vehicle: no path is made or followed"
        );
        m.extra.mount = Some(7);
        assert!(m.can_update_path(), "riding a vehicle: isPassenger()");
        m.extra.mount = None;
        m.fluids.in_water = true;
        assert!(m.can_update_path(), "in a liquid");
    }
}

#[cfg(test)]
mod supplied_path_tests {
    use super::*;
    use crate::mobs::world::{with_built_test_world, with_test_world};
    use horizon_core::geometry::Vec3;

    fn cow(types: &crate::mobs::MobTypes) -> std::sync::Arc<crate::mobs::MobType> {
        std::sync::Arc::new(types.types["minecraft:cow"].clone())
    }

    /// A supplied path is followed to its last node: positions are floored to
    /// their block, the speed becomes the path's modifier, and the path is in
    /// progress until its next-node index reaches the node count.
    #[test]
    #[ignore = "daily"]
    fn a_supplied_path_is_followed_and_finishes_at_its_node_count() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            assert!(m.ai.nav.is_done(), "no path");
            assert!(m.nav_move_to_supplied(
                w,
                &[[0.5, 64.0, 0.5], [1.9, 64.0, 0.5], [2.9, 64.0, 0.5]],
                0.75
            ));
            let path = m.ai.nav.path.as_ref().expect("a path was taken");
            assert_eq!(
                path.nodes
                    .iter()
                    .map(|n| (n.x, n.y, n.z))
                    .collect::<Vec<_>>(),
                vec![(0, 64, 0), (1, 64, 0), (2, 64, 0)],
                "each coordinate is floored to its block"
            );
            assert_eq!(path.target, [2, 64, 0]);
            assert_eq!(m.ai.nav.speed_modifier, 0.75);
            assert!(m.ai.nav.is_in_progress());
            assert!(!m.ai.nav.is_done());
            assert!(
                w.world.chunks.is_empty(),
                "classifying an unloaded node loads no chunk"
            );
            for (index, x) in [0.5, 1.5, 2.5].into_iter().enumerate() {
                m.position = Vec3::new(x, 64.0, 0.5);
                m.nav_tick(w);
                assert_eq!(m.ai.nav.path.as_ref().unwrap().next, index + 1);
            }
            assert_eq!(m.ai.nav.path.as_ref().unwrap().next, 3);
            assert_eq!(m.ai.nav.path.as_ref().unwrap().nodes.len(), 3);
            assert!(m.ai.nav.is_done(), "the follower reached the final node");
            assert!(!m.ai.nav.is_in_progress());
        });
    }

    /// Unloaded and out-of-world coordinates are accepted without loading chunks.
    #[test]
    #[ignore = "daily"]
    fn unloaded_and_out_of_world_nodes_are_accepted_without_loading_chunks() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            assert!(m.nav_move_to_supplied(w, &[[1.0e9, 1.0e9, 1.0e9]], 0.5));
            assert!(
                m.ai.nav.path.as_ref().is_some_and(|p| p.nodes[0]
                    == PathNode {
                        x: 1_000_000_000,
                        y: 1_000_000_000,
                        z: 1_000_000_000,
                        kind: p.nodes[0].kind,
                    }),
                "the out-of-world node is neither moved nor clamped"
            );
            assert!(w.world.chunks.is_empty(), "still no chunk was loaded");
            assert!(
                m.ai.nav.is_in_progress(),
                "the unloaded node is still ahead"
            );
        });
    }

    /// Classification is information, never a filter: a node the engine calls
    /// blocked is recorded as such and the path is taken whole.
    #[test]
    #[ignore = "daily"]
    fn a_node_the_engine_calls_blocked_is_recorded_not_refused() {
        with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                world.set(1, 64, 0, stone).unwrap();
            },
            |w, types| {
                let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.on_ground = true;
                assert!(m.nav_move_to_supplied(
                    w,
                    &[[0.5, 64.0, 0.5], [1.5, 64.0, 0.5], [2.5, 64.0, 0.5]],
                    0.5
                ));
                let path = m.ai.nav.path.as_ref().expect("the path is taken whole");
                assert_eq!(path.nodes.len(), 3);
                assert_eq!(
                    path.nodes[1].kind,
                    PathType::Blocked,
                    "the engine's classification is recorded"
                );
            },
        );
    }

    /// Every `moveTo` trims: a node standing in a cauldron is raised onto its
    /// rim at acceptance.
    #[test]
    #[ignore = "daily"]
    fn a_supplied_path_is_trimmed_like_any_move_to() {
        with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let cauldron = registry.default_state("minecraft:cauldron").unwrap();
                world.set(1, 63, 0, cauldron).unwrap();
            },
            |w, types| {
                let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 63.0, 0.5), 0.0);
                m.on_ground = true;
                assert!(m.nav_move_to_supplied(
                    w,
                    &[[0.5, 63.0, 0.5], [1.5, 63.0, 0.5], [2.5, 63.0, 0.5]],
                    0.5
                ));
                let path = m.ai.nav.path.as_ref().expect("a path was taken");
                assert_eq!(
                    path.nodes[0],
                    PathNode {
                        x: 0,
                        y: 63,
                        z: 0,
                        kind: path.nodes[0].kind
                    }
                );
                assert_eq!(
                    path.nodes[1].y, 64,
                    "the cauldron node is raised onto its rim"
                );
                assert_eq!(
                    path.nodes
                        .iter()
                        .map(|n| (n.x, n.y, n.z))
                        .collect::<Vec<_>>(),
                    [(0, 63, 0), (1, 64, 0), (2, 64, 0)],
                    "the successor is raised to the same rim"
                );
            },
        );
        with_built_test_world(
            &[],
            |world, _| {
                let mut chunk = crate::chunk::Chunk::empty(0, 0);
                let section = chunk.section_index(64).unwrap();
                let mut light = Box::new([0; 4096]);
                for x in [2, 3] {
                    light[crate::chunk::index(x, 64, 0)] = 0xF0;
                }
                std::sync::Arc::make_mut(&mut chunk.sections[section]).light =
                    crate::chunk::Light::Full(light);
                world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            },
            |w, types| {
                assert!(!w.can_see_sky(0, 64, 0));
                assert!(w.can_see_sky(2, 64, 0));
                let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.on_ground = true;
                m.ai.nav.avoid_sun = true;
                assert!(m.nav_move_to_supplied(
                    w,
                    &[
                        [0.5, 64.0, 0.5],
                        [1.5, 64.0, 0.5],
                        [2.5, 64.0, 0.5],
                        [3.5, 64.0, 0.5]
                    ],
                    0.5
                ));
                let path = m.ai.nav.path.as_ref().unwrap();
                assert_eq!(
                    path.nodes
                        .iter()
                        .map(|n| (n.x, n.y, n.z))
                        .collect::<Vec<_>>(),
                    [(0, 64, 0), (1, 64, 0)],
                    "avoid-sun keeps only the nodes before the first sky-lit node"
                );
            },
        );
    }

    /// The stricter rule: a block change on a supplied path drops it and no
    /// search runs in its place, leaving `done` true and in progress false.
    #[test]
    #[ignore = "daily"]
    fn a_block_change_drops_a_supplied_path_without_re_searching() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            let nodes: Vec<[f64; 3]> = (0..20).map(|x| [f64::from(x) + 0.5, 64.0, 0.5]).collect();
            assert!(m.nav_move_to_supplied(w, &nodes, 0.5));
            assert!(
                m.ai.nav.target_pos.is_none(),
                "a supplied path stores no recompute target"
            );
            // A block on the path changed: the tick asks for a recompute.
            m.queue_path_recompute();
            m.nav_tick(w);
            assert!(m.ai.nav.path.is_none(), "the path was dropped");
            assert!(m.ai.nav.is_done());
            assert!(!m.ai.nav.is_in_progress());
            assert!(!m.ai.nav.delayed_recompute);
            m.nav_tick(w);
            assert!(m.ai.nav.path.is_none(), "and nothing searched in its place");
        });
    }

    /// The stuck check stops a supplied path: afterwards `done` is true and
    /// `stuck` is true.
    #[test]
    #[ignore = "daily"]
    fn the_stuck_check_stops_a_supplied_path() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            m.ai.speed = 0.3;
            let nodes: Vec<[f64; 3]> = (0..200).map(|x| [f64::from(x) + 0.5, 64.0, 0.5]).collect();
            assert!(m.nav_move_to_supplied(w, &nodes, 0.5));
            for _ in 0..=100 {
                m.nav_tick(w);
            }
            assert!(m.ai.nav.stuck, "the 100-tick check found no progress");
            assert!(m.ai.nav.path.is_none(), "the stuck check dropped the path");
            assert!(m.ai.nav.is_done());
            assert!(!m.ai.nav.is_in_progress());
        });
    }

    /// The per-node timeout stops a supplied path too: afterwards `done` is
    /// true, in progress false, and `stuck` is false (the timeout resets it).
    #[test]
    #[ignore = "daily"]
    fn the_per_node_timeout_stops_a_supplied_path() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            m.ai.speed = 0.5;
            assert!(m.nav_move_to_supplied(w, &[[50.5, 64.0, 0.5], [51.5, 64.0, 0.5]], 0.5));
            let mob = Vec3::new(0.5, 64.0, 0.5);
            // The first sample sets the node's expected travel time.
            m.do_stuck_detection(w, mob);
            assert!(m.ai.nav.timeout_limit > 0.0);
            // Three times the expected travel time passes on the same node.
            m.ai.nav.last_timeout_check = w.game_time - (m.ai.nav.timeout_limit as i64 * 3 + 1);
            m.do_stuck_detection(w, mob);
            assert!(m.ai.nav.path.is_none(), "the timeout dropped the path");
            assert!(m.ai.nav.is_done());
            assert!(!m.ai.nav.is_in_progress());
            assert!(!m.ai.nav.stuck, "a timeout is not a stuck check");
        });
    }

    /// A walking node at the top of the `i32` range is classified and followed
    /// without overflowing: the floor under it takes the walk classifier
    /// through its neighbour scan, whose `x + 1` saturated badly in a checked
    /// build (arismoko/horizon#4443). A finite coordinate is never refused, and the node is
    /// neither moved nor clamped.
    #[test]
    #[ignore = "daily"]
    fn an_extreme_walking_node_is_classified_and_followed() {
        with_built_test_world(
            &[],
            |world, registry| {
                // `World::set` marks the sections around a write, which steps
                // past the `i32` range at the world's edge: the fence goes into
                // its chunk directly, the block the classifier reads.
                let (cx, cz) = (i32::MAX >> 4, i32::MAX >> 4);
                let fence = registry.default_state("minecraft:oak_fence").unwrap();
                let mut chunk = crate::chunk::Chunk::empty(cx, cz);
                chunk.set(i32::MAX & 15, 63, i32::MAX & 15, fence.0);
                world.chunks.insert((cx, cz), std::sync::Arc::new(chunk));
            },
            |w, types| {
                let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.on_ground = true;
                assert!(
                    m.nav_move_to_supplied(w, &[[i32::MAX as f64, 64.0, i32::MAX as f64]], 0.5),
                    "the extreme node is taken"
                );
                let path = m.ai.nav.path.as_ref().expect("a path was taken");
                assert_eq!(
                    (path.nodes[0].x, path.nodes[0].y, path.nodes[0].z),
                    (i32::MAX, 64, i32::MAX),
                    "the node stays where the mod put it"
                );
                assert_eq!(
                    path.nodes[0].kind,
                    PathType::Walkable,
                    "the fence below sent the classifier through the neighbour scan"
                );
                // The follow arithmetic reads the same extreme block: one tick
                // classifies nothing new and must not overflow either.
                m.nav_tick(w);
                assert!(m.ai.nav.path.is_some(), "the path survives its follow tick");
                assert!(!m.ai.nav.is_done(), "the node is still to reach");
            },
        );
    }

    /// A flying node in void air at the top of the `i32` range is classified
    /// and followed without overflowing: the fly classifier adds a neighbour
    /// (`x + 1`) to the node itself, the exact addition of arismoko/horizon#4443.
    #[test]
    #[ignore = "daily"]
    fn an_extreme_flying_node_is_classified_and_followed() {
        with_test_world(&[], |w, types| {
            let mut m = Mob::new(1, cow(types), Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.ai.nav.kind = NavKind::Flying;
            m.on_ground = true;
            assert!(
                m.nav_move_to_supplied(w, &[[i32::MAX as f64, 64.0, i32::MAX as f64]], 0.5),
                "void air at the saturation point is taken, not refused"
            );
            let path = m.ai.nav.path.as_ref().expect("a path was taken");
            assert_eq!(
                (path.nodes[0].x, path.nodes[0].y, path.nodes[0].z),
                (i32::MAX, 64, i32::MAX),
                "the node is neither moved nor clamped"
            );
            assert_eq!(
                path.nodes[0].kind,
                PathType::Open,
                "a fly classifier classified the extreme node itself"
            );
            m.nav_tick(w);
            assert!(m.ai.nav.path.is_some(), "the path survives its follow tick");
            assert!(!m.ai.nav.is_done(), "the node is still to reach");
        });
    }
}
