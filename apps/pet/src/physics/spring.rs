//! Unconditionally stable sub-stepped second-order damped spring solver.

/// Maximum integration sub-step (120 Hz) to guarantee stability even under OS scheduling stalls.
const MAX_SUBSTEP_SECS: f32 = 1.0 / 120.0;

/// Single-channel damped harmonic oscillator: `a = k*(target - x) - c*v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring1D {
    pub value: f32,
    pub velocity: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring1D {
    pub const fn new(initial: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            value: initial,
            velocity: 0.0,
            stiffness,
            damping,
        }
    }

    /// Add an instantaneous velocity kick (e.g., landing impact, head pat, or flick).
    pub fn impulse(&mut self, delta_v: f32) {
        self.velocity += delta_v;
    }

    /// Snap directly to a value with zero velocity.
    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.velocity = 0.0;
    }

    /// Advance the spring toward `target` over `dt` seconds using symplectic sub-stepping.
    pub fn step(&mut self, target: f32, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        if dt <= 0.0 {
            return;
        }
        let steps = ((dt / MAX_SUBSTEP_SECS).ceil() as u32).max(1);
        let h = dt / steps as f32;
        for _ in 0..steps {
            let accel = self.stiffness * (target - self.value) - self.damping * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }
        if !self.value.is_finite() || !self.velocity.is_finite() {
            self.value = target;
            self.velocity = 0.0;
        }
    }

    /// Returns `true` when position and velocity are within rest thresholds.
    pub fn is_settled(&self, target: f32, pos_eps: f32, vel_eps: f32) -> bool {
        (self.value - target).abs() <= pos_eps && self.velocity.abs() <= vel_eps
    }
}
