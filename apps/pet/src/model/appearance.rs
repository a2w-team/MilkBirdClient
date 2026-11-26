//! User-customizable appearance parameters for MilkBird ("奶鸟").
//!
//! Preserves core IP anchors (floating crown little flame + hollow green eyes + frog mouth)
//! while allowing users to freely customize hairstyle (8 styles including Shamate!) and body shape.

use serde::{Deserialize, Serialize};

/// Customizable hairstyle presets. Every style floats the signature little flame IP anchor above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HairStyle {
    /// Default seamless twin-strand natural ahoge.
    #[default]
    NaturalAhoge,
    /// Energetic twin-tails with golden hair ties and airy bangs.
    TwinTails,
    /// Glossy slicked-back boss hair.
    SlickBack,
    /// Round fluffy mushroom / bowl bob cut.
    MushroomBob,
    /// Soft milk-tea caramel curly bangs with bouncy side curls.
    CurlyBangs,
    /// Cute symmetric chibi twin buns (odango).
    TwinBuns,
    /// Wild spiky Shamate sea-urchin hair with electric-cyan highlight bangs!
    ShamateSpike,
    /// Dramatic silver-white & neon-magenta asymmetric Shamate wolf-tail!
    ShamateFlame,
}

impl HairStyle {
    pub const ALL: [Self; 8] = [
        Self::NaturalAhoge,
        Self::TwinTails,
        Self::SlickBack,
        Self::MushroomBob,
        Self::CurlyBangs,
        Self::TwinBuns,
        Self::ShamateSpike,
        Self::ShamateFlame,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::NaturalAhoge => "自然呆毛",
            Self::TwinTails => "元气双马尾",
            Self::SlickBack => "霸总大背头",
            Self::MushroomBob => "憨萌蘑菇头",
            Self::CurlyBangs => "奶咖卷刘海",
            Self::TwinBuns => "俏皮双丸子",
            Self::ShamateSpike => "葬爱爆炸头",
            Self::ShamateFlame => "银粉杀马特",
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

/// User-customizable appearance configuration (compact `Copy` struct, `16 bytes`).
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
            hair: HairStyle::NaturalAhoge,
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
