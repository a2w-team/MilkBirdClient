//! 8-state behavioral FSM and interaction controller for MilkBird ("小奶鸟").

use crate::model::{BeakPose, EyeExpression, FootPose, PetPose, PropKind};
use crate::physics::{PetPhysics, PhysicsTargets};
use crate::scheduler::FpsTier;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// The 8 primary behavioral states of the MilkBird desktop pet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PetState {
    Idle,
    WalkHop,
    DragDangle,
    Petted,
    AlertMatch,
    Sleep,
    Perch,
    Suspended,
}

/// User-selectable autonomy mode from the tray/context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BehaviorMode {
    #[default]
    FreeRoam,
    StayPut,
    EdgePerch,
}

/// Screen edge for perching / peeking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScreenEdge {
    Left,
    Right,
    Bottom,
}

/// External events fed from the native window shell or the tray bridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PetEvent {
    /// Cursor position relative to pet center, normalized to roughly `-1.0..1.0`.
    PointerMove { rel_x: f32, rel_y: f32 },
    /// Left click / tap on the pet in local 160x160 coordinates.
    Click { local_x: f32, local_y: f32 },
    /// Gentle head rub over crown.
    StrokeHead { intensity: f32 },
    /// Drag lifecycle.
    DragStart,
    DragMove { dx: f32, dy: f32 },
    DragEnd { near_edge: Option<ScreenEdge> },
    /// Unread/demand state updates from `tray.sock`.
    TrayStatus {
        unread_count: u32,
        active_demands: u32,
        urgent_pulse: bool,
    },
    /// Change autonomous walking preference.
    SetBehaviorMode(BehaviorMode),
    /// Manual sleep / wake toggle.
    ForceSleep,
    WakeUp,
    /// Fullscreen app / screen lock / stealth mode (`0 fps`).
    SetSuspended(bool),
    /// Explicitly enter a state (for `--demo` and preview generation).
    DemoState(PetState),
}

/// Horizontal movement request returned to the window shell (in logical points).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionDelta {
    pub dx: f32,
    pub dy: f32,
}

/// Tiny deterministic xorshift64 PRNG (zero external `rand` dependency).
#[derive(Debug, Clone, Copy)]
struct MiniRng(u64);

impl MiniRng {
    const fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e3779b97f4a7c15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32)
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }
}

/// Complete behavioral controller driving `PetPhysics` and outputting `PetPose`.
#[derive(Debug, Clone)]
pub struct PetController {
    state: PetState,
    pre_suspend_state: PetState,
    mode: BehaviorMode,
    physics: PetPhysics,
    rng: MiniRng,

    clock: f32,
    state_elapsed: f32,
    inactivity_secs: f32,
    sleep_timeout_secs: f32,
    high_fps_cooldown: f32,

    // Natural blink scheduling.
    next_blink_in: f32,
    blink_remaining: f32,

    // Roam / walk state.
    next_wander_in: f32,
    walk_duration: f32,
    walk_dir: f32,
    walk_speed: f32,
    walk_phase: f32,

    // Interaction combo & head-petting.
    recent_clicks: u8,
    click_combo_window: f32,
    combo_celebration: f32,
    petted_remaining: f32,

    // Edge perch info.
    perch_edge: Option<ScreenEdge>,

    // Gaze target.
    gaze_target_x: f32,
    gaze_target_y: f32,
    facing_target: f32,

    // Club tray status signals.
    unread_count: u32,
    active_demands: u32,
    alert_burst_remaining: f32,
}

impl Default for PetController {
    fn default() -> Self {
        Self::new(0x4d69_6c6b_4269_7264)
    }
}

impl PetController {
    pub fn new(seed: u64) -> Self {
        let mut rng = MiniRng::new(seed);
        let next_blink_in = rng.range(2.8, 5.0);
        let next_wander_in = rng.range(7.0, 14.0);
        Self {
            state: PetState::Idle,
            pre_suspend_state: PetState::Idle,
            mode: BehaviorMode::FreeRoam,
            physics: PetPhysics::default(),
            rng,
            clock: 0.0,
            state_elapsed: 0.0,
            inactivity_secs: 0.0,
            sleep_timeout_secs: 180.0,
            high_fps_cooldown: 0.0,
            next_blink_in,
            blink_remaining: 0.0,
            next_wander_in,
            walk_duration: 0.0,
            walk_dir: 1.0,
            walk_speed: 36.0,
            walk_phase: 0.0,
            recent_clicks: 0,
            click_combo_window: 0.0,
            combo_celebration: 0.0,
            petted_remaining: 0.0,
            perch_edge: None,
            gaze_target_x: 0.0,
            gaze_target_y: 0.0,
            facing_target: 1.0,
            unread_count: 0,
            active_demands: 0,
            alert_burst_remaining: 0.0,
        }
    }

    pub fn state(&self) -> PetState {
        self.state
    }

    pub fn mode(&self) -> BehaviorMode {
        self.mode
    }

    pub fn unread_count(&self) -> u32 {
        self.unread_count
    }

    pub fn active_demands(&self) -> u32 {
        self.active_demands
    }

    pub fn physics(&self) -> &PetPhysics {
        &self.physics
    }

    pub fn set_sleep_timeout_secs(&mut self, secs: f32) {
        self.sleep_timeout_secs = secs.max(5.0);
    }

    /// Flip horizontal roaming direction (called by window shell upon hitting a screen bound).
    pub fn reverse_walk_direction(&mut self) {
        self.walk_dir = -self.walk_dir;
        self.facing_target = self.walk_dir;
        self.boost_fps(1.0);
    }

    fn boost_fps(&mut self, seconds: f32) {
        self.high_fps_cooldown = self.high_fps_cooldown.max(seconds);
    }

    fn transition(&mut self, next: PetState) {
        if self.state != next {
            self.state = next;
            self.state_elapsed = 0.0;
        }
    }

    /// Handle an incoming user, window, or tray-bridge event.
    pub fn handle_event(&mut self, event: PetEvent) {
        match event {
            PetEvent::PointerMove { rel_x, rel_y } => {
                if self.state == PetState::Suspended || self.state == PetState::Sleep {
                    return;
                }
                let nx = rel_x.clamp(-1.0, 1.0);
                let ny = rel_y.clamp(-1.0, 1.0);
                // Keep subtle gaze tracking without waking 60fps unnecessarily unless moving briskly.
                let moved = (nx - self.gaze_target_x).abs() + (ny - self.gaze_target_y).abs();
                self.gaze_target_x = nx;
                self.gaze_target_y = ny;
                if moved > 0.25 && self.state == PetState::Idle {
                    self.boost_fps(0.35);
                }
            }
            PetEvent::Click { local_x, local_y } => {
                self.inactivity_secs = 0.0;
                if self.state == PetState::Suspended {
                    return;
                }
                if self.click_combo_window > 0.0 {
                    self.recent_clicks = self.recent_clicks.saturating_add(1);
                } else {
                    self.recent_clicks = 1;
                }
                self.click_combo_window = 0.85;

                // Clicking gently turns toward side tapped.
                if (local_x - 80.0).abs() > 18.0 {
                    self.facing_target = if local_x >= 80.0 { 1.0 } else { -1.0 };
                }

                // Rapid 3-click combo triggers joyful spin/hop easter egg!
                if self.recent_clicks >= 3 {
                    self.recent_clicks = 0;
                    self.combo_celebration = 1.6;
                    self.petted_remaining = 2.0;
                    self.physics.hop(260.0);
                    self.physics.pat_squish(1.3);
                    self.transition(PetState::Petted);
                } else {
                    let head_tap = local_y < 92.0;
                    self.petted_remaining = if head_tap { 1.9 } else { 1.3 };
                    self.physics.pat_squish(if head_tap { 1.15 } else { 0.85 });
                    if self.state == PetState::Sleep {
                        self.physics.hop(140.0);
                    }
                    self.transition(PetState::Petted);
                }
                self.boost_fps(2.0);
            }
            PetEvent::StrokeHead { intensity } => {
                self.inactivity_secs = 0.0;
                if self.state == PetState::Suspended || self.state == PetState::DragDangle {
                    return;
                }
                self.petted_remaining = 1.6;
                self.physics.pat_squish(0.45 * intensity.clamp(0.4, 1.8));
                self.transition(PetState::Petted);
                self.boost_fps(1.6);
            }
            PetEvent::DragStart => {
                self.inactivity_secs = 0.0;
                self.perch_edge = None;
                self.transition(PetState::DragDangle);
                self.physics.squash.impulse(2.2);
                self.physics.ahoge.impulse(-3.0);
                self.boost_fps(2.0);
            }
            PetEvent::DragMove { dx, dy } => {
                self.inactivity_secs = 0.0;
                if self.state != PetState::DragDangle {
                    self.transition(PetState::DragDangle);
                }
                if dx.abs() > 1.5 {
                    self.facing_target = if dx > 0.0 { 1.0 } else { -1.0 };
                }
                self.physics.apply_drag_delta(dx, dy);
                self.boost_fps(1.6);
            }
            PetEvent::DragEnd { near_edge } => {
                self.inactivity_secs = 0.0;
                self.physics.release_drop(18.0);
                if let Some(edge) = near_edge {
                    self.perch_edge = Some(edge);
                    if edge == ScreenEdge::Left {
                        self.facing_target = 1.0;
                    } else if edge == ScreenEdge::Right {
                        self.facing_target = -1.0;
                    }
                    self.transition(PetState::Perch);
                } else if self.unread_count > 0 {
                    self.transition(PetState::AlertMatch);
                } else {
                    self.transition(PetState::Idle);
                }
                self.boost_fps(1.8);
            }
            PetEvent::TrayStatus {
                unread_count,
                active_demands,
                urgent_pulse,
            } => {
                let increased = unread_count > self.unread_count;
                self.unread_count = unread_count;
                self.active_demands = active_demands;
                if self.state == PetState::Suspended {
                    return;
                }
                if unread_count > 0 && (increased || urgent_pulse) {
                    self.inactivity_secs = 0.0;
                    self.alert_burst_remaining = 3.2;
                    if self.state != PetState::DragDangle {
                        self.physics.hop(215.0);
                        self.transition(PetState::AlertMatch);
                    }
                    self.boost_fps(2.4);
                } else if unread_count == 0 && self.state == PetState::AlertMatch {
                    self.alert_burst_remaining = 0.0;
                    self.transition(PetState::Idle);
                    self.boost_fps(1.0);
                }
            }
            PetEvent::SetBehaviorMode(mode) => {
                self.mode = mode;
                self.inactivity_secs = 0.0;
                match mode {
                    BehaviorMode::StayPut => {
                        if self.state == PetState::WalkHop {
                            self.transition(PetState::Idle);
                        }
                    }
                    BehaviorMode::EdgePerch => {
                        self.perch_edge.get_or_insert(ScreenEdge::Right);
                        self.transition(PetState::Perch);
                    }
                    BehaviorMode::FreeRoam => {
                        if self.state == PetState::Perch {
                            self.perch_edge = None;
                            self.transition(PetState::Idle);
                        }
                    }
                }
                self.boost_fps(1.0);
            }
            PetEvent::ForceSleep => {
                if self.state != PetState::Suspended {
                    self.transition(PetState::Sleep);
                    self.boost_fps(1.0);
                }
            }
            PetEvent::WakeUp => {
                self.inactivity_secs = 0.0;
                if self.state == PetState::Sleep {
                    self.physics.hop(150.0);
                    self.transition(if self.unread_count > 0 {
                        PetState::AlertMatch
                    } else {
                        PetState::Idle
                    });
                    self.boost_fps(1.5);
                }
            }
            PetEvent::SetSuspended(suspended) => {
                if suspended && self.state != PetState::Suspended {
                    self.pre_suspend_state = self.state;
                    self.transition(PetState::Suspended);
                    self.high_fps_cooldown = 0.0;
                } else if !suspended && self.state == PetState::Suspended {
                    let restored = self.pre_suspend_state;
                    self.transition(if restored == PetState::Suspended {
                        PetState::Idle
                    } else {
                        restored
                    });
                    self.boost_fps(1.0);
                }
            }
            PetEvent::DemoState(target) => {
                self.inactivity_secs = 0.0;
                self.high_fps_cooldown = 1.6;
                match target {
                    PetState::Idle => {
                        self.unread_count = 0;
                        self.perch_edge = None;
                        self.transition(PetState::Idle);
                    }
                    PetState::WalkHop => {
                        self.walk_duration = 3.0;
                        self.walk_dir = 1.0;
                        self.facing_target = 1.0;
                        self.physics.hop(145.0);
                        self.transition(PetState::WalkHop);
                    }
                    PetState::DragDangle => {
                        self.transition(PetState::DragDangle);
                        self.physics.apply_drag_delta(22.0, -8.0);
                    }
                    PetState::Petted => {
                        self.petted_remaining = 2.5;
                        self.combo_celebration = 0.0;
                        self.physics.pat_squish(1.1);
                        self.transition(PetState::Petted);
                    }
                    PetState::AlertMatch => {
                        self.unread_count = self.unread_count.max(2);
                        self.alert_burst_remaining = 3.0;
                        self.physics.hop(200.0);
                        self.transition(PetState::AlertMatch);
                    }
                    PetState::Sleep => {
                        self.transition(PetState::Sleep);
                    }
                    PetState::Perch => {
                        self.perch_edge = Some(ScreenEdge::Right);
                        self.facing_target = -1.0;
                        self.transition(PetState::Perch);
                    }
                    PetState::Suspended => {
                        self.transition(PetState::Suspended);
                        self.high_fps_cooldown = 0.0;
                    }
                }
            }
        }
    }

    /// Advance the FSM and spring physics by `dt` seconds.
    pub fn tick(&mut self, dt: f32) -> (PetPose, MotionDelta) {
        if self.state == PetState::Suspended {
            return (self.compose_pose(), MotionDelta::default());
        }

        let dt = dt.clamp(0.0, 0.1);
        self.clock += dt;
        self.state_elapsed += dt;
        self.high_fps_cooldown = (self.high_fps_cooldown - dt).max(0.0);
        self.click_combo_window = (self.click_combo_window - dt).max(0.0);
        if self.click_combo_window == 0.0 {
            self.recent_clicks = 0;
        }
        self.combo_celebration = (self.combo_celebration - dt).max(0.0);
        self.alert_burst_remaining = (self.alert_burst_remaining - dt).max(0.0);

        // Natural blinking timer.
        if self.blink_remaining > 0.0 {
            self.blink_remaining = (self.blink_remaining - dt).max(0.0);
        } else {
            self.next_blink_in -= dt;
            if self.next_blink_in <= 0.0 {
                self.blink_remaining = 0.14;
                self.next_blink_in = self.rng.range(2.8, 5.4);
                if matches!(self.state, PetState::Idle | PetState::Perch) {
                    self.boost_fps(0.22);
                }
            }
        }

        let mut motion = MotionDelta::default();

        // State-specific transitions and autonomous behavior.
        match self.state {
            PetState::Idle => {
                self.inactivity_secs += dt;
                if self.unread_count > 0 {
                    self.transition(PetState::AlertMatch);
                } else if self.inactivity_secs >= self.sleep_timeout_secs {
                    self.transition(PetState::Sleep);
                } else if self.mode == BehaviorMode::FreeRoam {
                    self.next_wander_in -= dt;
                    if self.next_wander_in <= 0.0 {
                        self.walk_duration = self.rng.range(2.2, 4.4);
                        self.walk_dir = if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
                        self.facing_target = self.walk_dir;
                        self.next_wander_in = self.rng.range(8.0, 18.0);
                        self.transition(PetState::WalkHop);
                        self.physics.hop(120.0);
                        self.boost_fps(self.walk_duration + 0.8);
                    }
                }
            }
            PetState::WalkHop => {
                self.walk_duration -= dt;
                self.walk_phase += dt * 10.5;
                motion.dx = self.walk_dir * self.walk_speed * dt;
                // Periodic playful little hops during walks.
                if self.physics.altitude <= 0.01 && (self.walk_phase % TAU) < 0.25 {
                    self.physics.hop(110.0);
                }
                if self.walk_duration <= 0.0 {
                    self.transition(PetState::Idle);
                }
            }
            PetState::DragDangle => {
                self.walk_phase += dt * 15.0;
            }
            PetState::Petted => {
                self.petted_remaining -= dt;
                if self.combo_celebration > 0.0 && self.physics.altitude <= 0.01 {
                    self.physics.hop(175.0);
                }
                if self.petted_remaining <= 0.0 {
                    self.transition(if self.unread_count > 0 {
                        PetState::AlertMatch
                    } else if self.mode == BehaviorMode::EdgePerch {
                        PetState::Perch
                    } else {
                        PetState::Idle
                    });
                }
            }
            PetState::AlertMatch => {
                if self.unread_count == 0 {
                    self.transition(PetState::Idle);
                } else if self.alert_burst_remaining > 0.0 && self.physics.altitude <= 0.01 {
                    self.physics.hop(165.0);
                    self.boost_fps(1.2);
                }
            }
            PetState::Sleep => {
                // Slow peaceful breathing only; wakes on click/drag/unread.
            }
            PetState::Perch => {
                self.inactivity_secs += dt;
                if self.inactivity_secs >= self.sleep_timeout_secs * 1.5 {
                    self.transition(PetState::Sleep);
                }
            }
            PetState::Suspended => {}
        }

        let targets = self.compute_physics_targets();
        let landed = self.physics.step(&targets, dt);
        if landed {
            self.boost_fps(0.8);
        }

        (self.compose_pose(), motion)
    }

    fn compute_physics_targets(&self) -> PhysicsTargets {
        let t = self.clock;
        let mut targets = PhysicsTargets {
            gaze_x: self.gaze_target_x,
            gaze_y: self.gaze_target_y,
            facing: self.facing_target,
            ..PhysicsTargets::default()
        };

        match self.state {
            PetState::Idle => {
                // Rest targets are zero so transient springs settle cleanly and let the
                // scheduler drop from 60 Hz to 15 Hz IdleBreeze. Ambient breeze & breathing
                // are layered additively in `compose_pose()`.
            }
            PetState::WalkHop => {
                targets.tilt_rad = 0.08 * self.walk_dir * (self.walk_phase).sin();
                targets.wing_flap = 0.28 + 0.20 * (self.walk_phase).sin().abs();
                targets.tail_angle = 0.22 * (self.walk_phase).sin();
                targets.ahoge_angle = -0.20 * self.walk_dir + 0.14 * (self.walk_phase).cos();
                targets.scarf_sway = -0.35 * self.walk_dir + 0.15 * (self.walk_phase).sin();
                targets.scarf_lift = 0.25;
            }
            PetState::DragDangle => {
                targets.gravity_enabled = false;
                targets.held_altitude = 16.0;
                targets.squash = 0.10; // Gently elongated while held up.
                targets.wing_flap = 0.52 + 0.34 * (t * 14.0).sin();
                targets.tail_angle = 0.24 * (t * 10.0).sin();
                targets.ahoge_angle = 0.24 * (t * 7.0).sin();
                targets.scarf_lift = -0.30;
            }
            PetState::Petted => {
                if self.combo_celebration > 0.0 {
                    targets.tilt_rad = 0.18 * (t * 11.0).sin();
                    targets.wing_flap = 0.72 + 0.28 * (t * 16.0).sin();
                    targets.tail_angle = 0.32 * (t * 14.0).sin();
                    targets.ahoge_angle = 0.26 * (t * 12.0).cos();
                    targets.scarf_lift = 0.45;
                } else {
                    targets.squash = -0.07; // Cozy happy squish into hand!
                    targets.tilt_rad = 0.07 * (t * 4.5).sin();
                    targets.wing_flap = 0.22 + 0.12 * (t * 6.0).sin();
                    targets.tail_angle = 0.24 * (t * 8.0).sin();
                    targets.ahoge_angle = 0.18 * (t * 6.0).sin();
                }
            }
            PetState::AlertMatch => {
                let excited = if self.alert_burst_remaining > 0.0 {
                    1.0
                } else {
                    0.0
                };
                targets.wing_flap = excited * (0.45 + 0.42 * (t * 12.0).sin().abs());
                targets.tail_angle = excited * 0.28 * (t * 10.0).sin();
                targets.ahoge_angle = excited * 0.25 * (t * 9.0).cos();
                targets.scarf_sway = excited * 0.18 * (t * 5.0).sin();
                targets.scarf_lift = 0.32 * excited;
            }
            PetState::Sleep => {
                targets.sleep_curl = 1.0;
                targets.ahoge_angle = 0.16;
                targets.tail_angle = -0.10;
                targets.gaze_x = 0.0;
                targets.gaze_y = 0.0;
            }
            PetState::Perch => {
                let lean = match self.perch_edge {
                    Some(ScreenEdge::Left) => 0.12,
                    Some(ScreenEdge::Right) => -0.12,
                    _ => 0.0,
                };
                targets.tilt_rad = lean;
            }
            PetState::Suspended => {}
        }

        targets
    }

    /// Suggested adaptive frame-rate tier for the scheduler.
    pub fn suggested_fps_tier(&self) -> FpsTier {
        if self.state == PetState::Suspended {
            return FpsTier::Suspended;
        }
        let targets = self.compute_physics_targets();
        if self.high_fps_cooldown > 0.0
            || matches!(
                self.state,
                PetState::WalkHop | PetState::DragDangle | PetState::Petted
            )
            || (self.state == PetState::AlertMatch && self.alert_burst_remaining > 0.0)
            || !self.physics.is_settled(&targets)
        {
            return FpsTier::ActiveSmooth;
        }
        if self.state == PetState::Sleep {
            return FpsTier::SleepLow;
        }
        FpsTier::IdleBreeze
    }

    /// Synthesize the compact renderable `PetPose` from current state and physics.
    pub fn compose_pose(&self) -> PetPose {
        let (scale_x, scale_y) = self.physics.body_scales();
        let t = self.clock;

        let breath = match self.state {
            PetState::Sleep => (t * 1.35).sin() * 1.0,
            PetState::Suspended => 0.0,
            _ => (t * 2.1).sin() * 0.65,
        };

        let eyes = match self.state {
            PetState::Sleep => EyeExpression::Sleeping,
            PetState::Petted => EyeExpression::Happy,
            PetState::DragDangle => EyeExpression::Surprised,
            PetState::AlertMatch => {
                if self.alert_burst_remaining > 0.0 {
                    EyeExpression::Happy
                } else if self.blink_remaining > 0.0 {
                    EyeExpression::Blink
                } else {
                    EyeExpression::Focused
                }
            }
            PetState::Idle | PetState::Perch => {
                if self.blink_remaining > 0.0 {
                    EyeExpression::Blink
                } else if self.inactivity_secs > self.sleep_timeout_secs * 0.72 {
                    EyeExpression::Drowsy
                } else if self.active_demands > 0 {
                    EyeExpression::Focused
                } else {
                    EyeExpression::Normal
                }
            }
            PetState::WalkHop => {
                if self.blink_remaining > 0.0 {
                    EyeExpression::Blink
                } else {
                    EyeExpression::Happy
                }
            }
            PetState::Suspended => EyeExpression::Sleeping,
        };

        let beak = match self.state {
            PetState::Petted => BeakPose::HappyOpen,
            PetState::AlertMatch => {
                if self.alert_burst_remaining > 0.0 && (t * 8.0).sin() > 0.0 {
                    BeakPose::HappyOpen
                } else {
                    BeakPose::Chirp
                }
            }
            PetState::DragDangle => BeakPose::Chirp,
            _ => BeakPose::Closed,
        };

        let foot_pose = match self.state {
            PetState::WalkHop => FootPose::Walking,
            PetState::DragDangle => FootPose::Dangling,
            PetState::Sleep => FootPose::Tucked,
            _ => {
                if self.physics.altitude > 2.5 {
                    FootPose::Dangling
                } else {
                    FootPose::Planted
                }
            }
        };

        let blush = match self.state {
            PetState::Petted => 0.88,
            PetState::AlertMatch => 0.62,
            PetState::Sleep => 0.45,
            _ => 0.34,
        };

        let wing_spread = match self.state {
            PetState::DragDangle => 0.85,
            PetState::AlertMatch => {
                if self.alert_burst_remaining > 0.0 {
                    0.9
                } else {
                    0.45
                }
            }
            PetState::Petted => {
                if self.combo_celebration > 0.0 {
                    0.95
                } else {
                    0.35
                }
            }
            PetState::WalkHop => 0.3,
            _ => 0.0,
        };

        let prop = match self.state {
            PetState::Petted => {
                if self.combo_celebration > 0.0 {
                    PropKind::Sparkles {
                        phase: (t * 1.4).fract(),
                    }
                } else {
                    PropKind::Hearts {
                        phase: (self.state_elapsed * 0.65).fract(),
                        count: 2,
                    }
                }
            }
            PetState::Sleep => PropKind::SleepZzz {
                phase: (t * 0.32).fract(),
            },
            PetState::AlertMatch => PropKind::UnreadBadge {
                count: self.unread_count.max(1),
                pulse: (t * 2.4).sin() * 0.5 + 0.5,
            },
            PetState::Idle if self.active_demands > 0 => PropKind::FocusRipple {
                phase: (t * 0.45).fract(),
            },
            _ => PropKind::None,
        };

        let offset_x = match (self.state, self.perch_edge) {
            (PetState::Perch, Some(ScreenEdge::Left)) => -10.0,
            (PetState::Perch, Some(ScreenEdge::Right)) => 10.0,
            _ => 0.0,
        };

        let badge_glow = if self.unread_count > 0 {
            0.55 + 0.45 * (t * 3.5).sin()
        } else if self.active_demands > 0 {
            0.25
        } else {
            0.0
        };

        // Steady-state ambient breeze / breathing modulation (does not prevent spring settling!).
        let (amb_tilt, amb_ahoge, amb_scarf, amb_tail, amb_wing) = match self.state {
            PetState::Idle => (
                if self.active_demands > 0 {
                    0.045 * (t * 0.8).sin()
                } else {
                    0.0
                },
                0.08 * (t * 1.35).sin(),
                0.07 * (t * 1.1 - 0.5).sin(),
                0.07 * (t * 1.6).sin(),
                0.0,
            ),
            PetState::Perch => (
                0.03 * (t * 1.1).sin(),
                0.09 * (t * 1.4).sin(),
                0.06 * (t * 1.1).sin(),
                0.04 * (t * 1.3).sin(),
                0.0,
            ),
            PetState::Sleep => (
                0.0,
                0.035 * (t * 0.8).sin(),
                0.02 * (t * 0.8).sin(),
                0.0,
                0.0,
            ),
            PetState::AlertMatch if self.alert_burst_remaining <= 0.0 => (
                0.0,
                0.10 * (t * 2.2).sin(),
                0.10 * (t * 2.0).sin(),
                0.12 * (t * 2.4).sin(),
                0.14 * (t * 2.8).sin().abs(),
            ),
            _ => (0.0, 0.0, 0.0, 0.0, 0.0),
        };

        PetPose {
            facing: if self.physics.facing >= 0.0 { 1.0 } else { -1.0 },
            altitude: self.physics.altitude,
            offset_x,
            tilt_rad: self.physics.tilt.value + amb_tilt,
            scale_x,
            scale_y,
            breath,
            sleep_curl: self.physics.sleep_curl.value.clamp(0.0, 1.0),
            wing_flap: self.physics.wing.value + amb_wing,
            wing_spread,
            tail_angle: self.physics.tail.value + amb_tail,
            ahoge_angle: self.physics.ahoge.value + amb_ahoge,
            scarf_sway: self.physics.scarf_sway.value + amb_scarf,
            scarf_lift: self.physics.scarf_lift.value,
            gaze_x: self.physics.gaze_x,
            gaze_y: self.physics.gaze_y,
            blush,
            foot_phase: self.walk_phase,
            foot_pose,
            eyes,
            beak,
            badge_glow,
            prop,
        }
    }
}
