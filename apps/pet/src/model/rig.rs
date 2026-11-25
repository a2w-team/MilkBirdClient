//! Parametric 11-layer vector rig and compact runtime pose for "小奶鸟 (MilkBird)".
//!
//! All geometry lives in normalized 160x160 logical coordinates so the renderer scales
//! crisply to any Retina factor (1x/2x/3x) with < 32 KB static memory footprint.

use serde::{Deserialize, Serialize};

/// Canonical logical design canvas size (square, in logical points).
pub const LOGICAL_CANVAS_SIZE: f32 = 160.0;

/// The 11 ordered character layers drawn back-to-front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerKind {
    GroundShadow,
    TailFeathers,
    BackWing,
    Feet,
    BodyBlob,
    Scarf,
    FrontWing,
    Cheeks,
    Eyes,
    Beak,
    Ahoge,
    PropOverlay,
}

/// Complete back-to-front Z-order (BackWing + FrontWing form the wing pair across depth).
pub const LAYER_ORDER: [LayerKind; 12] = [
    LayerKind::GroundShadow,
    LayerKind::TailFeathers,
    LayerKind::BackWing,
    LayerKind::Feet,
    LayerKind::BodyBlob,
    LayerKind::Scarf,
    LayerKind::FrontWing,
    LayerKind::Cheeks,
    LayerKind::Eyes,
    LayerKind::Beak,
    LayerKind::Ahoge,
    LayerKind::PropOverlay,
];

/// 7 expressive vector eye states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EyeExpression {
    Normal,
    Blink,
    Happy,
    Drowsy,
    Surprised,
    Focused,
    Sleeping,
}

/// Beak articulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BeakPose {
    Closed,
    Chirp,
    HappyOpen,
}

/// Foot posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FootPose {
    Planted,
    Walking,
    Dangling,
    Tucked,
}

/// Lightweight micro-prop floating around the pet (zero text-shaping overhead).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PropKind {
    None,
    Hearts { phase: f32, count: u8 },
    SleepZzz { phase: f32 },
    UnreadBadge { count: u32, pulse: f32 },
    Sparkles { phase: f32 },
    FocusRipple { phase: f32 },
}

/// Static normalized proportions of the MilkBird rig in 160x160 logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PetRig {
    pub canvas_size: f32,
    pub anchor_x: f32,
    pub ground_y: f32,
    pub body_radius_x: f32,
    pub body_radius_y: f32,
    pub body_center_offset_y: f32,
    pub outline_width: f32,
    pub eye_offset_x: f32,
    pub eye_spacing: f32,
    pub eye_offset_y: f32,
    pub eye_radius: f32,
    pub beak_offset_x: f32,
    pub beak_offset_y: f32,
    pub scarf_offset_y: f32,
}

impl Default for PetRig {
    fn default() -> Self {
        Self {
            canvas_size: LOGICAL_CANVAS_SIZE,
            anchor_x: 80.0,
            ground_y: 136.0,
            body_radius_x: 37.0,
            body_radius_y: 35.0,
            body_center_offset_y: -41.0,
            outline_width: 2.35,
            eye_offset_x: 7.5,
            eye_spacing: 14.5,
            eye_offset_y: -11.5,
            eye_radius: 4.5,
            beak_offset_x: 15.0,
            beak_offset_y: -4.5,
            scarf_offset_y: 9.5,
        }
    }
}

/// Compact, zero-allocation frame pose produced by the state machine + physics solver.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PetPose {
    /// Horizontal facing: `+1.0` = facing right, `-1.0` = facing left.
    pub facing: f32,
    /// Height above ground in logical points (`>= 0.0`).
    pub altitude: f32,
    /// Local horizontal micro-offset (used for perch peek or wiggle).
    pub offset_x: f32,
    /// Body tilt in radians (`+` leans clockwise).
    pub tilt_rad: f32,
    /// Volume-preserving horizontal stretch (`~0.78..1.28`).
    pub scale_x: f32,
    /// Volume-preserving vertical stretch (`~0.78..1.28`).
    pub scale_y: f32,
    /// Subtle chest breathing expansion (`-1.0..1.0`).
    pub breath: f32,
    /// Sleep curl factor (`0.0` upright, `1.0` cozy flattened dumpling).
    pub sleep_curl: f32,
    /// Wing flap angle in radians (`0.0` tucked, positive lifts upward).
    pub wing_flap: f32,
    /// Wing outward spread (`0.0` cozy tucked, `1.0` wide excited).
    pub wing_spread: f32,
    /// Tail feather wag angle in radians.
    pub tail_angle: f32,
    /// Milk-foam ahoge bend angle in radians.
    pub ahoge_angle: f32,
    /// Secondary scarf ribbon sway in radians.
    pub scarf_sway: f32,
    /// Scarf tail flutter lift (`-1.0..1.0`).
    pub scarf_lift: f32,
    /// Normalized pupil gaze offset (`-1.0..1.0`).
    pub gaze_x: f32,
    pub gaze_y: f32,
    /// Cheek blush opacity (`0.0..1.0`).
    pub blush: f32,
    /// Walk cycle or dangle kick phase in radians.
    pub foot_phase: f32,
    /// Foot articulation mode.
    pub foot_pose: FootPose,
    /// Eye expression.
    pub eyes: EyeExpression,
    /// Beak expression.
    pub beak: BeakPose,
    /// Club scarf badge highlight mode (`0.0` calm, `1.0` alert glow).
    pub badge_glow: f32,
    /// Optional floating micro-prop.
    pub prop: PropKind,
}

impl Default for PetPose {
    fn default() -> Self {
        Self {
            facing: 1.0,
            altitude: 0.0,
            offset_x: 0.0,
            tilt_rad: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            breath: 0.0,
            sleep_curl: 0.0,
            wing_flap: 0.0,
            wing_spread: 0.0,
            tail_angle: 0.0,
            ahoge_angle: 0.0,
            scarf_sway: 0.0,
            scarf_lift: 0.0,
            gaze_x: 0.0,
            gaze_y: 0.0,
            blush: 0.34,
            foot_phase: 0.0,
            foot_pose: FootPose::Planted,
            eyes: EyeExpression::Normal,
            beak: BeakPose::Closed,
            badge_glow: 0.0,
            prop: PropKind::None,
        }
    }
}

impl PetPose {
    /// Quantized hash fingerprint used by the adaptive scheduler to skip visually identical frames.
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x100000001b3);
        };
        let q = |x: f32, step: f32| -> u64 { (x * step).round() as i32 as u32 as u64 };

        mix(q(self.facing, 100.0));
        mix(q(self.altitude, 20.0));
        mix(q(self.offset_x, 20.0));
        mix(q(self.tilt_rad, 400.0));
        mix(q(self.scale_x, 400.0));
        mix(q(self.scale_y, 400.0));
        mix(q(self.breath, 200.0));
        mix(q(self.sleep_curl, 200.0));
        mix(q(self.wing_flap, 300.0));
        mix(q(self.wing_spread, 200.0));
        mix(q(self.tail_angle, 300.0));
        mix(q(self.ahoge_angle, 300.0));
        mix(q(self.scarf_sway, 300.0));
        mix(q(self.scarf_lift, 200.0));
        mix(q(self.gaze_x, 100.0));
        mix(q(self.gaze_y, 100.0));
        mix(q(self.blush, 100.0));
        mix(q(self.foot_phase, 100.0));
        mix(self.foot_pose as u64);
        mix(self.eyes as u64);
        mix(self.beak as u64);
        mix(q(self.badge_glow, 100.0));
        match self.prop {
            PropKind::None => mix(0),
            PropKind::Hearts { phase, count } => {
                mix(1);
                mix(q(phase, 60.0));
                mix(u64::from(count));
            }
            PropKind::SleepZzz { phase } => {
                mix(2);
                mix(q(phase, 40.0));
            }
            PropKind::UnreadBadge { count, pulse } => {
                mix(3);
                mix(u64::from(count));
                mix(q(pulse, 40.0));
            }
            PropKind::Sparkles { phase } => {
                mix(4);
                mix(q(phase, 60.0));
            }
            PropKind::FocusRipple { phase } => {
                mix(5);
                mix(q(phase, 40.0));
            }
        }
        h
    }
}
