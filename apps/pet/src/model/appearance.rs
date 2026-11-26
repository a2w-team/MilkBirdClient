//! User-customizable appearance parameters for MilkBird ("奶鸟").
//!
//! Preserves core IP anchors (crown little flame + hollow green eyes + frog mouth)
//! while allowing users to freely customize hairstyle and body shape.

use serde::{Deserialize, Serialize};

/// Customizable hairstyle presets. Every style mounts the signature crown little flame IP anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HairStyle {
    /// Default sweeping flame crest with stepped mini embers (inspired by fiery crest silhouette).
    #[default]
    FlameSweep,
    /// Soft fluffy cloud-curl bangs topped by a lively center flame.
    FluffyCloud,
    /// Playful symmetric twin cheek-tufts framing a central flickering flame.
    TwinSparks,
    /// Neat rounded fringe with a cute teardrop flame perched on top.
    BangsTuft,
}

impl HairStyle {
    pub const ALL: [Self; 4] = [
        Self::FlameSweep,
        Self::FluffyCloud,
        Self::TwinSparks,
        Self::BangsTuft,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::FlameSweep => "自然呆毛",
            Self::FluffyCloud => "奶咖软卷",
            Self::TwinSparks => "俏皮双丸子",
            Self::BangsTuft => "元气齐刘海",
        }
    }
}

/// Customizable body silhouette presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BodyShape {
    /// Classic chubby round dumpling ball.
    #[default]
    RoundBall,
    /// Narrower head and chubby pear-shaped tummy (plump Nailong-like silhouette).
    PearChubby,
    /// Wide, low, ultra-squishy mochi daifuku body.
    MochiFlat,
    /// Slightly taller upright soft bean silhouette.
    TallBean,
}

impl BodyShape {
    pub const ALL: [Self; 4] = [
        Self::RoundBall,
        Self::PearChubby,
        Self::MochiFlat,
        Self::TallBean,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::RoundBall => "圆团子",
            Self::PearChubby => "梨形大肚",
            Self::MochiFlat => "大福麻薯",
            Self::TallBean => "长条软豆",
        }
    }
}

/// User-customizable appearance configuration (compact `Copy` struct, `< 32 bytes`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetAppearance {
    pub hair: HairStyle,
    pub body: BodyShape,
    /// Optional fine-tune horizontal body proportion (`0.85..=1.20`).
    pub body_width_scale: f32,
    /// Optional fine-tune vertical body proportion (`0.85..=1.20`).
    pub body_height_scale: f32,
    /// Optional crown little-flame size multiplier (`0.75..=1.35`).
    pub flame_scale: f32,
}

impl Default for PetAppearance {
    fn default() -> Self {
        Self {
            hair: HairStyle::FlameSweep,
            body: BodyShape::RoundBall,
            body_width_scale: 1.0,
            body_height_scale: 1.0,
            flame_scale: 1.0,
        }
    }
}

impl PetAppearance {
    pub fn sanitized(self) -> Self {
        Self {
            hair: self.hair,
            body: self.body,
            body_width_scale: self.body_width_scale.clamp(0.85, 1.20),
            body_height_scale: self.body_height_scale.clamp(0.85, 1.20),
            flame_scale: self.flame_scale.clamp(0.75, 1.35),
        }
    }

    pub fn with_hair(mut self, hair: HairStyle) -> Self {
        self.hair = hair;
        self
    }

    pub fn with_body(mut self, body: BodyShape) -> Self {
        self.body = body;
        self
    }
}
