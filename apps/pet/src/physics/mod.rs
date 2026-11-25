//! 8-DOF coupled spring-damper physics for the MilkBird character.

pub mod spring;

pub use spring::Spring1D;

/// Targets fed into the physics solver from the behavior state machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsTargets {
    pub squash: f32,
    pub tilt_rad: f32,
    pub ahoge_angle: f32,
    pub scarf_sway: f32,
    pub scarf_lift: f32,
    pub wing_flap: f32,
    pub tail_angle: f32,
    pub sleep_curl: f32,
    pub gaze_x: f32,
    pub gaze_y: f32,
    pub facing: f32,
    /// If `true`, vertical altitude is governed by gravity & ground bounce; otherwise held aloft.
    pub gravity_enabled: bool,
    /// Target altitude when `gravity_enabled == false` (e.g., held while dragging).
    pub held_altitude: f32,
}

impl Default for PhysicsTargets {
    fn default() -> Self {
        Self {
            squash: 0.0,
            tilt_rad: 0.0,
            ahoge_angle: 0.0,
            scarf_sway: 0.0,
            scarf_lift: 0.0,
            wing_flap: 0.0,
            tail_angle: 0.0,
            sleep_curl: 0.0,
            gaze_x: 0.0,
            gaze_y: 0.0,
            facing: 1.0,
            gravity_enabled: true,
            held_altitude: 0.0,
        }
    }
}

/// Physical simulation state of the MilkBird rig (zero heap allocation).
#[derive(Debug, Clone, Copy)]
pub struct PetPhysics {
    /// Volume-preserving vertical squash (`+` = stretched tall, `-` = squashed wide).
    pub squash: Spring1D,
    /// Body tilt in radians.
    pub tilt: Spring1D,
    /// Altitude above ground (`>= 0.0`) + vertical velocity for hops & drops.
    pub altitude: f32,
    pub vertical_vel: f32,
    /// High-elasticity milk-foam ahoge bend in radians.
    pub ahoge: Spring1D,
    /// Scarf ribbon horizontal sway in radians.
    pub scarf_sway: Spring1D,
    /// Scarf vertical flutter (`-1..1`).
    pub scarf_lift: Spring1D,
    /// Wing flap spring in radians.
    pub wing: Spring1D,
    /// Tail wag spring in radians.
    pub tail: Spring1D,
    /// Smooth sleep-curl transition (`0..1`).
    pub sleep_curl: Spring1D,
    /// Smooth eye gaze follower.
    pub gaze_x: f32,
    pub gaze_y: f32,
    /// Smooth facing sign (`-1.0..1.0`, never stuck near 0).
    pub facing: f32,
}

impl Default for PetPhysics {
    fn default() -> Self {
        Self {
            // Snappy bouncy mochi squash-and-stretch.
            squash: Spring1D::new(0.0, 260.0, 15.0),
            // Gentle body lean.
            tilt: Spring1D::new(0.0, 140.0, 14.0),
            altitude: 0.0,
            vertical_vel: 0.0,
            // Playful springy milk-foam tuft!
            ahoge: Spring1D::new(0.0, 210.0, 9.5),
            // Flowing secondary scarf ribbon.
            scarf_sway: Spring1D::new(0.0, 95.0, 8.0),
            scarf_lift: Spring1D::new(0.0, 120.0, 10.5),
            // Crisp wing flaps.
            wing: Spring1D::new(0.0, 240.0, 16.0),
            // Cheerful tail wag.
            tail: Spring1D::new(0.0, 175.0, 12.0),
            // Cozy sleep curl transition.
            sleep_curl: Spring1D::new(0.0, 55.0, 12.0),
            gaze_x: 0.0,
            gaze_y: 0.0,
            facing: 1.0,
        }
    }
}

impl PetPhysics {
    /// Gravity in logical points / s^2.
    const GRAVITY: f32 = 980.0;

    /// Apply a jump impulse (upward launch + stretch + ahoge whip).
    pub fn hop(&mut self, launch_speed: f32) {
        self.vertical_vel = launch_speed.max(0.0);
        self.squash.impulse(2.6);
        self.ahoge.impulse(-3.2);
        self.scarf_lift.impulse(-2.4);
    }

    /// Apply a soft squish impulse (e.g., head pat / click).
    pub fn pat_squish(&mut self, strength: f32) {
        let s = strength.clamp(0.2, 2.0);
        self.squash.impulse(-3.4 * s);
        self.ahoge.impulse(2.8 * s);
        self.tail.impulse(2.6 * s);
        self.wing.impulse(1.8 * s);
    }

    /// Feed horizontal/vertical drag delta into inertial lag channels.
    pub fn apply_drag_delta(&mut self, dx: f32, dy: f32) {
        let clamped_dx = dx.clamp(-60.0, 60.0);
        let clamped_dy = dy.clamp(-60.0, 60.0);
        // Body lags behind motion like being gently picked up by the scruff.
        self.tilt.impulse((-clamped_dx * 0.032).clamp(-1.8, 1.8));
        self.ahoge.impulse((-clamped_dx * 0.075).clamp(-4.5, 4.5));
        self.scarf_sway
            .impulse((-clamped_dx * 0.085).clamp(-4.8, 4.8));
        self.scarf_lift
            .impulse((clamped_dy * 0.055).clamp(-3.0, 3.0));
        self.squash
            .impulse((clamped_dy.abs() * 0.025).clamp(0.0, 1.5));
    }

    /// Release from drag with natural drop velocity and bounce.
    pub fn release_drop(&mut self, drop_altitude: f32) {
        self.altitude = drop_altitude.clamp(0.0, 40.0);
        self.vertical_vel = 0.0;
        self.squash.impulse(1.4);
    }

    /// Step all 8 coupled channels over `dt` seconds. Returns `true` if a ground landing occurred.
    pub fn step(&mut self, targets: &PhysicsTargets, dt: f32) -> bool {
        let dt = dt.clamp(0.0, 0.1);
        let mut landed = false;

        if targets.gravity_enabled {
            if self.altitude > 0.0 || self.vertical_vel > 0.0 {
                self.vertical_vel -= Self::GRAVITY * dt;
                self.altitude += self.vertical_vel * dt;
                if self.altitude <= 0.0 {
                    let impact = (-self.vertical_vel).min(520.0);
                    self.altitude = 0.0;
                    if impact > 95.0 {
                        // Gentle secondary bounce on hard drop.
                        self.vertical_vel = impact * 0.22;
                    } else {
                        self.vertical_vel = 0.0;
                    }
                    // Landing squash + fluffy ahoge/scarf rebound.
                    let norm = (impact / 280.0).clamp(0.15, 1.4);
                    self.squash.impulse(-4.0 * norm);
                    self.ahoge.impulse(3.5 * norm);
                    self.scarf_lift.impulse(2.8 * norm);
                    self.wing.impulse(1.5 * norm);
                    landed = true;
                }
            }
        } else {
            // Smoothly pull toward held altitude while dragged.
            let alpha = 1.0 - (-18.0 * dt).exp();
            self.altitude += (targets.held_altitude - self.altitude) * alpha;
            self.vertical_vel = 0.0;
        }

        self.squash.step(targets.squash, dt);
        self.tilt.step(targets.tilt_rad, dt);
        self.ahoge.step(targets.ahoge_angle, dt);
        self.scarf_sway.step(targets.scarf_sway, dt);
        self.scarf_lift.step(targets.scarf_lift, dt);
        self.wing.step(targets.wing_flap, dt);
        self.tail.step(targets.tail_angle, dt);
        self.sleep_curl.step(targets.sleep_curl, dt);

        // Smooth eye gaze low-pass filter.
        let gaze_alpha = 1.0 - (-10.0 * dt).exp();
        self.gaze_x += (targets.gaze_x.clamp(-1.0, 1.0) - self.gaze_x) * gaze_alpha;
        self.gaze_y += (targets.gaze_y.clamp(-1.0, 1.0) - self.gaze_y) * gaze_alpha;

        // Snappy horizontal turn.
        let target_facing = if targets.facing >= 0.0 { 1.0 } else { -1.0 };
        let face_alpha = 1.0 - (-16.0 * dt).exp();
        self.facing += (target_facing - self.facing) * face_alpha;
        if (self.facing - target_facing).abs() < 0.01 {
            self.facing = target_facing;
        }

        landed
    }

    /// Volume-preserving squash & stretch scales (`scale_x * scale_y ≈ 1`).
    pub fn body_scales(&self) -> (f32, f32) {
        let curl = self.sleep_curl.value.clamp(0.0, 1.0);
        let raw_y = (1.0 + self.squash.value.clamp(-0.24, 0.28)) * (1.0 - 0.16 * curl);
        let scale_y = raw_y.clamp(0.74, 1.28);
        let scale_x = ((1.0 / scale_y.sqrt()) * (1.0 + 0.10 * curl)).clamp(0.78, 1.30);
        (scale_x, scale_y)
    }

    /// Returns `true` when all dynamic springs have converged within `< 0.05px` equivalent.
    pub fn is_settled(&self, targets: &PhysicsTargets) -> bool {
        let airborne = targets.gravity_enabled
            && (self.altitude > 0.05 || self.vertical_vel.abs() > 0.5);
        if airborne {
            return false;
        }
        let held_delta = !targets.gravity_enabled && (self.altitude - targets.held_altitude).abs() > 0.05;
        if held_delta {
            return false;
        }
        let eps_p = 0.002;
        let eps_v = 0.01;
        self.squash.is_settled(targets.squash, eps_p, eps_v)
            && self.tilt.is_settled(targets.tilt_rad, eps_p, eps_v)
            && self.ahoge.is_settled(targets.ahoge_angle, eps_p, eps_v)
            && self.scarf_sway.is_settled(targets.scarf_sway, eps_p, eps_v)
            && self.scarf_lift.is_settled(targets.scarf_lift, eps_p, eps_v)
            && self.wing.is_settled(targets.wing_flap, eps_p, eps_v)
            && self.tail.is_settled(targets.tail_angle, eps_p, eps_v)
            && self.sleep_curl.is_settled(targets.sleep_curl, eps_p, eps_v)
            && (self.gaze_x - targets.gaze_x).abs() <= 0.01
            && (self.gaze_y - targets.gaze_y).abs() <= 0.01
    }
}
