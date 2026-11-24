use super::*;

impl PlayerMotion {
    /// `Player.tick`'s jump-trigger countdown and `LocalPlayer.aiStep`'s
    /// creative-flight toggle. `vehicle_jumpable` is vanilla's
    /// `getVehicle() == null || jumpableVehicle() != null`: a rider only
    /// toggles flight on a jumpable mount. Returns the jump key last tick,
    /// whether flight changed, and whether the toggle jumped from the ground
    /// (the caller runs `jumpFromGround`).
    pub fn creative_flight_toggle(
        &mut self,
        jumping: bool,
        vehicle_jumpable: bool,
    ) -> (bool, bool, bool) {
        if self.jump_trigger_time > 0 {
            self.jump_trigger_time -= 1;
        }
        let was_jumping = std::mem::replace(&mut self.was_jumping, jumping);
        let flying_before = self.flying;
        let mut jumped = false;
        if self.spectator {
            self.flying = true;
            self.ground.on_ground = false;
        } else if self.may_fly && !was_jumping && jumping && !self.was_auto_jump {
            // `LocalPlayer.aiStep`: mayfly gates the flight toggle, not the
            // flying state independently supplied by the server's abilities.
            if self.jump_trigger_time == 0 {
                self.jump_trigger_time = 7;
            } else if vehicle_jumpable && !self.swimming {
                self.flying = !self.flying;
                if self.flying && self.ground.on_ground {
                    jumped = true;
                }
                self.jump_trigger_time = 0;
            }
        }
        (was_jumping, flying_before != self.flying, jumped)
    }

    /// Explicit identity allocation for deterministic reference replay.
    pub fn with_attribute_identities(
        resources: &MotionResources,
        position: Vec3,
        hashes: &[i32],
        next: Box<dyn horizon_world::attributes::live::Identities>,
    ) -> Result<Self> {
        let mut attributes = LiveAttributes::new(
            resources.attributes.clone(),
            "minecraft:player",
            hashes,
            next,
        )
        .map_err(attribute_error)?;
        Ok(Self {
            random: crate::mobs::entity_random(0, position),
            health: crate::health::PlayerHealth::new(&resources.health, &mut attributes)?,
            status: crate::effects::StatusFlags::default(),
            impulse: ImpulseContext::default(),
            spawn_extra_particles_on_fall: false,
            position,
            velocity: Vec3::ZERO,
            ground: GroundState::default(),
            collision: CollisionFlags::default(),
            history: MovementHistory::default(),
            fall_distance: 0.0,
            block_landing: None,
            stuck_multiplier: Vec3::ZERO,
            piston: Default::default(),
            yaw: 0.0,
            head_yaw: 0.0,
            jumping: false,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            may_fly: false,
            mobility_restricted: false,
            invulnerable: false,
            spectator: false,
            flying: false,
            flying_speed: 0.05,
            was_jumping: false,
            auto_jump_enabled: false,
            auto_jump_time: 0,
            was_auto_jump: false,
            move_vector: [0.0, 0.0],
            applied_input: [0.0, 0.0],
            glider: false,
            pitch: 0.0,
            fall_flying: false,
            sleeping: false,
            fall_fly_ticks: 0,
            glide_ending: false,
            server_owns_glide: false,
            glider_wear: 0,
            location_effects: LocationEffects::default(),
            auto_spin_attack_ticks: 0,
            auto_spin_attack_dmg: 0.0,
            auto_spin_attack_item: None,
            spin_targets: Vec::new(),
            spin_hit: None,
            sprinting: false,
            sprint_input: crate::sprint::SprintState::default(),
            shift_key_down: false,
            local_crouching: false,
            pose: crate::pose::Pose::Standing,
            dimensions: crate::pose::Pose::Standing.dimensions(1.0),
            size_answer: None,
            scale: 1.0,
            swimming: false,
            is_passenger: false,
            fire_ticks: -20,
            air: 300,
            respiration: 0,
            speed: 0.0,
            item_use_speed: None,
            item_use_slows_sprint: false,
            nudge: [None; 2],
            last_climbable: None,
            on_climbable: false,
            steps: MovementDistances {
                next_step: 1.0,
                ..MovementDistances::default()
            },
            crystal_sound_intensity: 0.0,
            last_crystal_sound_play_tick: 0,
            needs_sync: false,
            sync_position: false,
            teleported: false,
            teleported_delta: [false; 3],
            events: Vec::new(),
            attributes,
            fluids: PlayerFluids::default(),
            in_powder_snow: false,
            was_in_powder_snow: false,
            ticks_frozen: 0,
            leather_boots: false,
            freeze_immune: false,
            burn_from_stepping_immune: false,
            old_position: position,
            server_copy: false,
            replaying: false,
            wall_hit_events: None,
            client_side: false,
            entity_id: 0,
            client_loaded: true,
            block_effects: true,
            hurt_log: Vec::new(),
            attack_log: Vec::new(),
            border: None,
            collision_exclusions: Default::default(),
        })
    }
}
