//! Original "奶鸟 (MilkBird)" procedural vector model definitions and customization types.

pub mod appearance;
pub mod palette;
pub mod rig;

pub use appearance::{BodyShape, HairStyle, PetAppearance};
pub use palette::{Palette, Rgba};
pub use rig::{
    BeakPose, EyeExpression, FootPose, LAYER_ORDER, LOGICAL_CANVAS_SIZE, LayerKind, PetPose,
    PetRig, PropKind,
};
