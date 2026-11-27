//! Original "小奶鸟 (MilkBird)" procedural vector model definitions.

pub mod palette;
pub mod rig;

pub use palette::{Palette, Rgba};
pub use rig::{
    BeakPose, EyeExpression, FootPose, LAYER_ORDER, LOGICAL_CANVAS_SIZE, LayerKind, PetPose,
    PetRig, PropKind,
};
