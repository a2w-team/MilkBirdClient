//! Zero-jitter pre-allocated `tiny-skia` vector renderer and O(1) alpha HitMask
//! for the original "奶鸟 (MilkBird)" IP character.
//!
//! Core IP anchors (preserved across all user customizations):
//! 1. Living little flame ("小火苗") crowning the customizable hairstyle.
//! 2. Vacant-cute hollow green eyes ("空洞绿眼睛", Nailong-inspired emerald iris + void pupil).
//! 3. Wide expressive frog mouth ("蛙嘴": chill frog line, round "o", or joyful wide grin).
//!
//! User-customizable traits via [`PetAppearance`]:
//! - 4 hairstyles (`FlameSweep`, `FluffyCloud`, `TwinSparks`, `BangsTuft`)
//! - 4 body shapes (`RoundBall`, `PearChubby`, `MochiFlat`, `TallBean`) + fine-tune scales.

use crate::model::{
    BeakPose, BodyShape, EyeExpression, FootPose, HairStyle, LOGICAL_CANVAS_SIZE, Palette,
    PetAppearance, PetPose, PetRig, PropKind, Rgba,
};
use std::f32::consts::PI;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, Point, RadialGradient, SpreadMode, Stroke, Transform,
};

/// Alpha threshold (out of 255) for pixel-level pointer hit-testing.
const HIT_ALPHA_THRESHOLD: u8 = 18;

/// Geometry anchors derived from the selected [`BodyShape`] so hair, face, belly, and wings
/// always sit harmoniously on every custom body silhouette.
#[derive(Debug, Clone, Copy)]
struct BodyMetrics {
    crown_y: f32,
    head_rx: f32,
    eye_y: f32,
    mouth_y: f32,
    belly_y: f32,
    belly_scale_x: f32,
    belly_scale_y: f32,
    wing_y: f32,
    front_wing_x: f32,
    back_wing_x: f32,
    tail_x: f32,
    tail_y: f32,
    foot_y_offset: f32,
}

impl BodyMetrics {
    fn for_shape(shape: BodyShape) -> Self {
        match shape {
            BodyShape::RoundBall => Self {
                crown_y: -34.0,
                head_rx: 34.5,
                eye_y: -12.5,
                mouth_y: -2.0,
                belly_y: 16.5,
                belly_scale_x: 1.0,
                belly_scale_y: 0.96,
                wing_y: 5.5,
                front_wing_x: -23.0,
                back_wing_x: 31.0,
                tail_x: -33.5,
                tail_y: 15.0,
                foot_y_offset: 36.0,
            },
            BodyShape::PearChubby => Self {
                crown_y: -38.5,
                head_rx: 27.5,
                eye_y: -18.5,
                mouth_y: -8.0,
                belly_y: 14.0,
                belly_scale_x: 1.12,
                belly_scale_y: 1.08,
                wing_y: 2.0,
                front_wing_x: -21.5,
                back_wing_x: 29.0,
                tail_x: -36.5,
                tail_y: 17.5,
                foot_y_offset: 37.0,
            },
            BodyShape::MochiFlat => Self {
                crown_y: -27.5,
                head_rx: 39.5,
                eye_y: -9.0,
                mouth_y: 0.5,
                belly_y: 16.0,
                belly_scale_x: 1.16,
                belly_scale_y: 0.78,
                wing_y: 6.0,
                front_wing_x: -27.5,
                back_wing_x: 35.5,
                tail_x: -39.5,
                tail_y: 14.0,
                foot_y_offset: 32.0,
            },
            BodyShape::TallBean => Self {
                crown_y: -41.5,
                head_rx: 29.0,
                eye_y: -21.0,
                mouth_y: -10.5,
                belly_y: 14.5,
                belly_scale_x: 0.88,
                belly_scale_y: 1.15,
                wing_y: 0.5,
                front_wing_x: -19.5,
                back_wing_x: 27.5,
                tail_x: -29.0,
                tail_y: 17.0,
                foot_y_offset: 38.5,
            },
        }
    }
}

/// Pre-allocated CPU vector renderer. Reuses its internal `Pixmap` across frames.
pub struct PetRenderer {
    rig: PetRig,
    palette: Palette,
    appearance: PetAppearance,
    scale: f32,
    pixmap: Pixmap,
}

impl PetRenderer {
    /// Allocate a renderer for the given Retina scale factor (`1.0`, `2.0`, `3.0`).
    pub fn new(scale: f32) -> Self {
        Self::with_config(
            PetRig::default(),
            Palette::milkbird_original(),
            PetAppearance::default(),
            scale,
        )
    }

    pub fn with_config(
        rig: PetRig,
        palette: Palette,
        appearance: PetAppearance,
        scale: f32,
    ) -> Self {
        let scale = scale.clamp(1.0, 4.0);
        let px = (rig.canvas_size * scale).round().max(16.0) as u32;
        let pixmap = Pixmap::new(px, px).expect("valid pet pixmap dimensions");
        Self {
            rig,
            palette,
            appearance: appearance.sanitized(),
            scale,
            pixmap,
        }
    }

    pub fn rig(&self) -> PetRig {
        self.rig
    }

    pub fn palette(&self) -> Palette {
        self.palette
    }

    pub fn appearance(&self) -> PetAppearance {
        self.appearance
    }

    pub fn set_appearance(&mut self, appearance: PetAppearance) {
        self.appearance = appearance.sanitized();
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn pixel_size(&self) -> (u32, u32) {
        (self.pixmap.width(), self.pixmap.height())
    }

    /// Premultiplied RGBA8 frame buffer ready for direct macOS `CGImage` upload.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// O(1) pixel-level alpha hit test in logical `160x160` coordinates (`< 0.001 ms`).
    pub fn hit_test(&self, logical_x: f32, logical_y: f32) -> bool {
        if logical_x < 0.0
            || logical_y < 0.0
            || logical_x >= self.rig.canvas_size
            || logical_y >= self.rig.canvas_size
        {
            return false;
        }
        let px = (logical_x * self.scale) as u32;
        let py = (logical_y * self.scale) as u32;
        let w = self.pixmap.width();
        let h = self.pixmap.height();
        if px >= w || py >= h {
            return false;
        }
        let idx = ((py * w + px) * 4 + 3) as usize;
        self.pixmap.data()[idx] >= HIT_ALPHA_THRESHOLD
    }

    /// Render the original customizable MilkBird character into the pre-allocated `Pixmap`.
    pub fn render(&mut self, pose: &PetPose) -> &Pixmap {
        self.pixmap.data_mut().fill(0);

        let scale = self.scale;
        let anchor_x = self.rig.anchor_x + pose.offset_x;
        let ground_y = self.rig.ground_y;
        let facing = if pose.facing >= 0.0 { 1.0 } else { -1.0 };
        let metrics = BodyMetrics::for_shape(self.appearance.body);

        // 1. Soft ground shadow.
        self.draw_ground_shadow(anchor_x, ground_y, pose, scale);

        let breath_scale_x = 1.0 + 0.018 * pose.breath;
        let breath_scale_y = 1.0 + 0.022 * pose.breath;
        let sx = pose.scale_x * breath_scale_x * self.appearance.body_width_scale * facing;
        let sy = pose.scale_y * breath_scale_y * self.appearance.body_height_scale;

        let base_rest = match self.appearance.body {
            BodyShape::MochiFlat => 35.5,
            BodyShape::TallBean => 43.0,
            BodyShape::PearChubby => 41.5,
            BodyShape::RoundBall => 40.5,
        };
        let rest_height = (base_rest - 8.0 * pose.sleep_curl) * self.appearance.body_height_scale;
        let body_y = ground_y - pose.altitude - rest_height * pose.scale_y;

        let body_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(sx, sy);

        let body_rigid_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(facing, 1.0);

        // 2. Tail feathers behind body.
        self.draw_tail_feathers(body_tx, pose, &metrics);

        // 3. Back wing peeking cleanly on right contour.
        self.draw_back_wing(body_tx, pose, &metrics);

        // 4. Honey-apricot little feet.
        self.draw_feet(
            body_rigid_tx,
            anchor_x,
            ground_y,
            facing,
            pose,
            scale,
            &metrics,
        );

        // 5. Customizable body shape (RoundBall / PearChubby / MochiFlat / TallBean).
        self.draw_body_blob(body_tx);

        // 6. Clean creamy milk-foam belly fluff (zero logos!).
        self.draw_milk_belly(body_tx, &metrics);

        // 7. Front sleek two-tone wing.
        self.draw_front_wing(body_tx, pose, &metrics);

        // 8. Soft strawberry-milk cheek blushes.
        self.draw_cheeks(body_tx, pose, &metrics);

        // 9. IP Anchor #2a: Hollow green eyes (Nailong-inspired emerald iris + void pupil).
        self.draw_hollow_green_eyes(body_tx, pose, &metrics);

        // 10. IP Anchor #2b: Wide expressive frog mouth ("蛙嘴").
        self.draw_frog_mouth(body_tx, pose, &metrics);

        // 11. IP Anchor #1 + Custom Hair: Customizable hairstyle crowned with the living little flame!
        self.draw_hair_and_little_flame(body_tx, pose, &metrics);

        // 12. Floating micro-prop overlay (hearts, sleep Zzz, unread pill, sparkles, focus ripple).
        self.draw_prop(anchor_x, body_y, facing, pose, scale, &metrics);

        &self.pixmap
    }

    fn stroke_outline(&self, width: f32) -> Stroke {
        Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        }
    }

    fn solid_paint(rgba: Rgba) -> Paint<'static> {
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(rgba.to_skia());
        paint
    }

    fn draw_ground_shadow(&mut self, anchor_x: f32, ground_y: f32, pose: &PetPose, scale: f32) {
        let alt_factor = (1.0 - pose.altitude / 90.0).clamp(0.35, 1.0);
        let shape_w = match self.appearance.body {
            BodyShape::MochiFlat => 1.15,
            BodyShape::PearChubby => 1.06,
            BodyShape::TallBean => 0.88,
            BodyShape::RoundBall => 1.0,
        };
        let rx = (32.5 + 4.0 * pose.sleep_curl)
            * pose.scale_x
            * self.appearance.body_width_scale
            * shape_w
            * alt_factor;
        let ry = 7.2 * alt_factor;
        if let Some(path) = ellipse_path(anchor_x, ground_y + 1.5, rx, ry) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            let a = (self.palette.shadow.a as f32 * alt_factor).round() as u8;
            paint.set_color(Color::from_rgba8(
                self.palette.shadow.r,
                self.palette.shadow.g,
                self.palette.shadow.b,
                a,
            ));
            let tx = Transform::from_scale(scale, scale);
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, tx, None);
        }
    }

    fn draw_tail_feathers(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let wag_deg = pose.tail_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.45;

        let lower_tx = body_tx
            .pre_translate(m.tail_x, m.tail_y + 2.0)
            .pre_rotate(18.0 + wag_deg * 0.85);
        if let Some(path) = feather_path(20.0, 6.2) {
            let fill = Self::solid_paint(self.palette.tail_base);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, lower_tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.88),
                lower_tx,
                None,
            );
        }

        let upper_tx = body_tx
            .pre_translate(m.tail_x + 1.5, m.tail_y - 3.5)
            .pre_rotate(-14.0 + wag_deg);
        if let Some(path) = feather_path(23.0, 6.8) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(-23.0, 0.0),
                vec![
                    GradientStop::new(0.0, self.palette.tail_base.to_skia()),
                    GradientStop::new(0.65, self.palette.flame_outer.to_skia()),
                    GradientStop::new(1.0, self.palette.tail_tip.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.tail_tip.to_skia());
            }
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, upper_tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                upper_tx,
                None,
            );
        }
    }

    fn draw_back_wing(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let lift_deg = -12.0 - pose.wing_flap.to_degrees() * 1.05 - pose.wing_spread * 24.0;
        let tx = body_tx
            .pre_translate(
                m.back_wing_x + pose.wing_spread * 2.5,
                m.wing_y - 1.0 - pose.wing_spread * 2.5,
            )
            .pre_rotate(lift_deg);
        if let Some(path) = sculpted_wing_path(1.0) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, -4.0),
                Point::from_xy(16.0, 15.0),
                vec![
                    GradientStop::new(0.0, self.palette.wing_shade.to_skia()),
                    GradientStop::new(0.68, self.palette.wing_base.to_skia()),
                    GradientStop::new(1.0, self.palette.wing_tip.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.wing_shade.to_skia());
            }
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                tx,
                None,
            );
        }
    }

    fn draw_front_wing(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let flap_deg = -2.0 + pose.wing_flap.to_degrees() * 1.15 + pose.wing_spread * 28.0;
        let tx = body_tx
            .pre_translate(
                m.front_wing_x - pose.wing_spread * 2.5,
                m.wing_y - pose.wing_spread * 2.0,
            )
            .pre_rotate(flap_deg);
        if let Some(path) = sculpted_wing_path(-1.0) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(2.0, -4.0),
                Point::from_xy(-17.0, 16.0),
                vec![
                    GradientStop::new(0.0, self.palette.body_highlight.to_skia()),
                    GradientStop::new(0.58, self.palette.wing_base.to_skia()),
                    GradientStop::new(1.0, self.palette.wing_tip.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.wing_base.to_skia());
            }
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, tx, None);

            let mut fe = PathBuilder::new();
            fe.move_to(-10.5, 8.5);
            fe.quad_to(-14.0, 11.2, -11.5, 13.8);
            if let Some(streak) = fe.finish() {
                let sp = Self::solid_paint(self.palette.belly_highlight.with_alpha(0.55));
                self.pixmap
                    .stroke_path(&streak, &sp, &self.stroke_outline(1.8), tx, None);
            }

            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.92),
                tx,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_feet(
        &mut self,
        body_rigid_tx: Transform,
        anchor_x: f32,
        ground_y: f32,
        facing: f32,
        pose: &PetPose,
        scale: f32,
        m: &BodyMetrics,
    ) {
        if pose.foot_pose == FootPose::Tucked && pose.sleep_curl > 0.65 {
            let tx = Transform::from_scale(scale, scale)
                .pre_translate(anchor_x, ground_y - 3.5)
                .pre_scale(facing, 1.0);
            self.draw_single_foot(tx, -10.5, 0.0, 0.0);
            self.draw_single_foot(tx, 11.5, 0.0, 0.0);
            return;
        }

        match pose.foot_pose {
            FootPose::Planted | FootPose::Tucked => {
                let tx = Transform::from_scale(scale, scale)
                    .pre_translate(anchor_x, ground_y - 4.0 - pose.altitude)
                    .pre_scale(facing, 1.0);
                self.draw_single_foot(tx, -10.5, 0.0, 0.0);
                self.draw_single_foot(tx, 11.5, 0.0, 0.0);
            }
            FootPose::Walking => {
                let step = pose.foot_phase.sin();
                let lift_l = step.max(0.0) * 6.5;
                let lift_r = (-step).max(0.0) * 6.5;
                let stride_l = step * 6.2;
                let stride_r = -step * 6.2;
                let tx = Transform::from_scale(scale, scale)
                    .pre_translate(anchor_x, ground_y - 4.0 - pose.altitude)
                    .pre_scale(facing, 1.0);
                self.draw_single_foot(tx, -10.5 + stride_l, -lift_l, step * 18.0);
                self.draw_single_foot(tx, 11.5 + stride_r, -lift_r, -step * 18.0);
            }
            FootPose::Dangling => {
                let kick = (pose.foot_phase * 1.4).sin();
                self.draw_single_foot(body_rigid_tx, -10.5, m.foot_y_offset, -22.0 + kick * 28.0);
                self.draw_single_foot(body_rigid_tx, 11.5, m.foot_y_offset, 22.0 - kick * 28.0);
            }
        }
    }

    fn draw_single_foot(&mut self, base_tx: Transform, x: f32, y: f32, rot_deg: f32) {
        let tx = base_tx.pre_translate(x, y).pre_rotate(rot_deg);
        let mut pb = PathBuilder::new();
        pb.move_to(-2.4, -7.0);
        pb.line_to(2.4, -7.0);
        pb.line_to(2.7, -0.6);
        pb.quad_to(8.2, -0.2, 8.4, 3.2);
        pb.quad_to(6.2, 5.5, 2.6, 4.4);
        pb.quad_to(0.0, 5.9, -2.6, 4.4);
        pb.quad_to(-6.2, 5.3, -6.4, 2.4);
        pb.quad_to(-5.6, -0.2, -2.4, -0.6);
        pb.close();

        if let Some(path) = pb.finish() {
            let fill = Self::solid_paint(self.palette.foot_base);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, tx, None);
            self.pixmap
                .stroke_path(&path, &outline, &self.stroke_outline(1.85), tx, None);
        }
    }

    /// Customizable body silhouettes (`RoundBall`, `PearChubby`, `MochiFlat`, `TallBean`).
    fn draw_body_blob(&mut self, body_tx: Transform) {
        let mut pb = PathBuilder::new();
        match self.appearance.body {
            BodyShape::RoundBall => {
                pb.move_to(0.0, -34.5);
                pb.cubic_to(23.5, -34.5, 39.0, -17.0, 38.2, 4.2);
                pb.cubic_to(37.5, 23.0, 23.8, 35.0, 0.0, 35.0);
                pb.cubic_to(-23.8, 35.0, -38.5, 23.0, -38.8, 4.2);
                pb.cubic_to(-39.0, -17.0, -23.5, -34.5, 0.0, -34.5);
            }
            BodyShape::PearChubby => {
                // Cute narrower round head smoothly flaring into a chubby Nailong-like pear belly!
                pb.move_to(0.0, -39.0);
                pb.cubic_to(19.0, -39.0, 28.5, -26.0, 29.0, -10.0);
                pb.cubic_to(29.5, -1.0, 41.5, 7.0, 40.5, 19.5);
                pb.cubic_to(39.5, 31.0, 24.5, 36.5, 0.0, 36.5);
                pb.cubic_to(-24.5, 36.5, -40.0, 31.0, -40.8, 19.5);
                pb.cubic_to(-41.5, 7.0, -29.5, -1.0, -29.0, -10.0);
                pb.cubic_to(-28.5, -26.0, -19.0, -39.0, 0.0, -39.0);
            }
            BodyShape::MochiFlat => {
                // Wide, low, ultra-squishy mochi daifuku silhouette!
                pb.move_to(0.0, -28.0);
                pb.cubic_to(28.5, -28.0, 45.0, -13.5, 44.5, 6.5);
                pb.cubic_to(44.0, 23.0, 29.0, 31.0, 0.0, 31.0);
                pb.cubic_to(-29.0, 31.0, -44.5, 23.0, -45.0, 6.5);
                pb.cubic_to(-45.5, -13.5, -28.5, -28.0, 0.0, -28.0);
            }
            BodyShape::TallBean => {
                // Soft upright capsule/penguin-bean silhouette!
                pb.move_to(0.0, -42.0);
                pb.cubic_to(21.0, -42.0, 31.5, -26.0, 31.5, -3.0);
                pb.cubic_to(31.5, 22.0, 22.5, 37.5, 0.0, 37.5);
                pb.cubic_to(-22.5, 37.5, -32.0, 22.0, -32.0, -3.0);
                pb.cubic_to(-32.0, -26.0, -21.0, -42.0, 0.0, -42.0);
            }
        }
        pb.close();

        let Some(body_path) = pb.finish() else {
            return;
        };

        let mut grad_paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(8.0, -16.0),
            Point::from_xy(4.0, -4.0),
            56.0,
            vec![
                GradientStop::new(0.0, self.palette.body_highlight.to_skia()),
                GradientStop::new(0.60, self.palette.body_base.to_skia()),
                GradientStop::new(1.0, self.palette.body_shade.to_skia()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            grad_paint.shader = shader;
        } else {
            grad_paint.set_color(self.palette.body_base.to_skia());
        }
        self.pixmap
            .fill_path(&body_path, &grad_paint, FillRule::Winding, body_tx, None);

        // Soft warm highlight on forehead crown.
        let m = BodyMetrics::for_shape(self.appearance.body);
        if let Some(crown) = ellipse_path(5.0, m.crown_y + 10.5, 13.5, 4.8) {
            let sheen = Self::solid_paint(Rgba::new(255, 178, 164, 68));
            self.pixmap
                .fill_path(&crown, &sheen, FillRule::Winding, body_tx, None);
        }

        let outline = Self::solid_paint(self.palette.outline);
        self.pixmap.stroke_path(
            &body_path,
            &outline,
            &self.stroke_outline(self.rig.outline_width),
            body_tx,
            None,
        );
    }

    /// Pure creamy milk-foam cloud-tufted belly (zero logos, adapts to body shape).
    fn draw_milk_belly(&mut self, body_tx: Transform, m: &BodyMetrics) {
        let bx = 5.5;
        let by = m.belly_y;
        let sx = m.belly_scale_x;
        let sy = m.belly_scale_y;
        let tx = body_tx.pre_translate(bx, by).pre_scale(sx, sy);

        let mut pb = PathBuilder::new();
        pb.move_to(-18.5, -5.0);
        pb.cubic_to(-17.0, -13.0, -9.5, -15.2, -5.0, -11.8);
        pb.cubic_to(-2.2, -16.8, 4.8, -16.8, 7.2, -11.6);
        pb.cubic_to(11.8, -14.8, 18.5, -11.6, 19.4, -4.0);
        pb.cubic_to(20.6, 8.2, 12.0, 17.0, 0.0, 17.2);
        pb.cubic_to(-12.4, 17.2, -20.2, 8.4, -18.5, -5.0);
        pb.close();

        if let Some(belly) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(2.0, -5.0),
                Point::from_xy(0.0, 1.0),
                24.0,
                vec![
                    GradientStop::new(0.0, self.palette.belly_highlight.to_skia()),
                    GradientStop::new(0.72, self.palette.belly_base.to_skia()),
                    GradientStop::new(1.0, self.palette.belly_shade.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.belly_base.to_skia());
            }
            self.pixmap
                .fill_path(&belly, &paint, FillRule::Winding, tx, None);

            if let Some(glow) = ellipse_path(2.5, -2.0, 9.2, 5.8) {
                let p = Self::solid_paint(self.palette.belly_highlight.with_alpha(0.65));
                self.pixmap
                    .fill_path(&glow, &p, FillRule::Winding, tx, None);
            }
        }
    }

    fn draw_cheeks(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let alpha = pose.blush.clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let paint = Self::solid_paint(self.palette.cheek.with_alpha(alpha * 0.65));
        let cy = m.mouth_y - 1.5;
        for &(cx, rx, ry) in &[(-13.5, 5.8, 3.3), (24.5, 5.4, 3.1)] {
            if let Some(cheek) = ellipse_path(cx, cy, rx, ry) {
                self.pixmap
                    .fill_path(&cheek, &paint, FillRule::Winding, body_tx, None);
            }
        }
    }

    /// IP Anchor #2a: Nailong-faithful hollow olive-green eyes!
    /// Soft white eye socket (sclera visible on outer edge, no harsh cartoon ring),
    /// deep moss-green outer iris + luminous olive-emerald lower glow + large glossy
    /// dark-forest hollow pupil occupying the upper-center, with iconic 4-point star
    /// catchlights when happy/excited and sloped eyelids when focused.
    fn draw_hollow_green_eyes(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let ey = m.eye_y;
        // (center_x, center_y, inward_bias) — iris hugs slightly inward so white sclera peeks
        // cleanly on the outer corners just like Nailong in reference image 2!
        let eye_specs = [(-5.8f32, ey, 1.35f32), (18.8f32, ey, -1.35f32)];
        let sclera = Self::solid_paint(self.palette.eye_sclera);
        let pupil = Self::solid_paint(self.palette.eye_pupil);
        let sparkle = Self::solid_paint(self.palette.eye_sparkle);
        let socket_shadow = Self::solid_paint(self.palette.outline.with_alpha(0.35));
        let lid_lash = Self::solid_paint(self.palette.eye_pupil.with_alpha(0.85));

        let gx = pose.gaze_x.clamp(-1.0, 1.0) * 1.4;
        let gy = pose.gaze_y.clamp(-1.0, 1.0) * 1.1;

        for &(ex, ey, inward) in &eye_specs {
            let ix = ex + inward + gx * 0.45;
            let iy = ey + gy * 0.45;
            match pose.eyes {
                EyeExpression::Normal => {
                    // 1. Subtle recessed socket shadow + soft white sclera (no hard black circle!).
                    if let Some(sh) = ellipse_path(ex, ey - 0.4, 7.6, 7.2) {
                        self.pixmap
                            .fill_path(&sh, &socket_shadow, FillRule::Winding, body_tx, None);
                    }
                    if let Some(white) = ellipse_path(ex, ey + 0.2, 7.2, 6.7) {
                        self.pixmap
                            .fill_path(&white, &sclera, FillRule::Winding, body_tx, None);
                    }
                    // 2. Glossy Nailong olive-emerald hollow iris + large dark upper-center pupil.
                    self.draw_nailong_green_iris(body_tx, ix, iy + 0.2, 5.5, 5.7);
                    // 3. Soft dark upper lash curve (top only, authentic 3D plush/Nailong feel).
                    let mut lash = PathBuilder::new();
                    lash.move_to(ex - 6.2, ey - 2.6);
                    lash.quad_to(ex, ey - 7.8, ex + 6.2, ey - 2.6);
                    if let Some(lp) = lash.finish() {
                        self.pixmap
                            .stroke_path(&lp, &lid_lash, &self.stroke_outline(1.55), body_tx, None);
                    }
                    // 4. Tiny vacant upper-left pinpoint highlight (keeps center deeply hollow!).
                    if let Some(pin) = ellipse_path(ix - 1.9, iy - 1.9, 1.15, 1.15) {
                        self.pixmap
                            .fill_path(&pin, &sparkle, FillRule::Winding, body_tx, None);
                    }
                }
                EyeExpression::Blink => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.4, ey + 0.6);
                    pb.quad_to(ex, ey + 3.4, ex + 5.4, ey + 0.4);
                    if let Some(arc) = pb.finish() {
                        let rim = Self::solid_paint(self.palette.eye_sclera.with_alpha(0.7));
                        self.pixmap
                            .stroke_path(&arc, &rim, &self.stroke_outline(3.8), body_tx, None);
                        self.pixmap
                            .stroke_path(&arc, &pupil, &self.stroke_outline(2.3), body_tx, None);
                    }
                }
                EyeExpression::Happy => {
                    // Exact Nailong left-pose joyful starry olive-green eyes!
                    if let Some(sh) = ellipse_path(ex, ey - 0.3, 7.8, 7.4) {
                        self.pixmap
                            .fill_path(&sh, &socket_shadow, FillRule::Winding, body_tx, None);
                    }
                    if let Some(white) = ellipse_path(ex, ey + 0.2, 7.4, 6.9) {
                        self.pixmap
                            .fill_path(&white, &sclera, FillRule::Winding, body_tx, None);
                    }
                    self.draw_nailong_green_iris(body_tx, ix, iy + 0.1, 5.7, 5.9);
                    // Big glowing 4-point white star + mini upper-right star just like ref image 2!
                    if let Some(star_big) = star4_path(ix - 0.5, iy - 0.2, 4.1) {
                        self.pixmap
                            .fill_path(&star_big, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    if let Some(star_sm) = star4_path(ix + 2.8, iy - 2.3, 1.7) {
                        self.pixmap
                            .fill_path(&star_sm, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let mut lash = PathBuilder::new();
                    lash.move_to(ex - 6.4, ey - 2.6);
                    lash.quad_to(ex, ey - 8.0, ex + 6.4, ey - 2.6);
                    if let Some(lp) = lash.finish() {
                        self.pixmap
                            .stroke_path(&lp, &lid_lash, &self.stroke_outline(1.5), body_tx, None);
                    }
                }
                EyeExpression::Drowsy => {
                    if let Some(white) = ellipse_path(ex, ey + 0.8, 7.0, 5.4) {
                        self.pixmap
                            .fill_path(&white, &sclera, FillRule::Winding, body_tx, None);
                    }
                    self.draw_nailong_green_iris(body_tx, ix, iy + 1.1, 5.1, 4.2);
                    let cover = Self::solid_paint(self.palette.body_base);
                    if let Some(lid_cover) = ellipse_path(ex, ey - 2.6, 8.0, 3.6) {
                        self.pixmap
                            .fill_path(&lid_cover, &cover, FillRule::Winding, body_tx, None);
                    }
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 6.6, ey - 0.8);
                    pb.line_to(ex + 6.6, ey - 0.4);
                    if let Some(lid) = pb.finish() {
                        self.pixmap
                            .stroke_path(&lid, &pupil, &self.stroke_outline(2.1), body_tx, None);
                    }
                }
                EyeExpression::Surprised => {
                    // Wide open white socket + smaller hollow Nailong green iris (hilarious shock!).
                    if let Some(sh) = ellipse_path(ex, ey - 0.3, 8.3, 8.2) {
                        self.pixmap
                            .fill_path(&sh, &socket_shadow, FillRule::Winding, body_tx, None);
                    }
                    if let Some(white) = ellipse_path(ex, ey + 0.1, 7.9, 7.8) {
                        self.pixmap
                            .fill_path(&white, &sclera, FillRule::Winding, body_tx, None);
                    }
                    self.draw_nailong_green_iris(body_tx, ex + inward * 0.5, ey, 4.9, 5.1);
                    if let Some(pin) = ellipse_path(ex + inward * 0.5 - 1.6, ey - 1.7, 1.15, 1.15) {
                        self.pixmap
                            .fill_path(&pin, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 3.8, ey - 10.2);
                    brow.quad_to(ex, ey - 12.2, ex + 3.8, ey - 10.2);
                    if let Some(b) = brow.finish() {
                        self.pixmap
                            .stroke_path(&b, &lid_lash, &self.stroke_outline(1.75), body_tx, None);
                    }
                }
                EyeExpression::Focused => {
                    // Exact Nailong middle/right-pose grumpy/focused sloped upper eyelid over green eye!
                    if let Some(white) = ellipse_path(ex, ey + 0.4, 7.2, 6.4) {
                        self.pixmap
                            .fill_path(&white, &sclera, FillRule::Winding, body_tx, None);
                    }
                    self.draw_nailong_green_iris(body_tx, ix, iy + 0.6, 5.4, 5.2);
                    if let Some(pin) = ellipse_path(ix - 1.7, iy - 1.2, 1.1, 1.1) {
                        self.pixmap
                            .fill_path(&pin, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    // Sloped coral-red upper eyelid clipping top-inner eye just like Nailong!
                    let slant = if ex < 5.0 { 3.2 } else { -3.2 };
                    let mut cover = PathBuilder::new();
                    cover.move_to(ex - 8.5, ey - 9.5);
                    cover.line_to(ex + 8.5, ey - 9.5);
                    cover.line_to(ex + 8.5, ey - 3.8 + slant);
                    cover.line_to(ex - 8.5, ey - 3.8 - slant);
                    cover.close();
                    if let Some(cp) = cover.finish() {
                        let body_p = Self::solid_paint(self.palette.body_base);
                        self.pixmap
                            .fill_path(&cp, &body_p, FillRule::Winding, body_tx, None);
                    }
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 6.8, ey - 3.8 - slant);
                    brow.line_to(ex + 6.8, ey - 3.8 + slant);
                    if let Some(b) = brow.finish() {
                        self.pixmap
                            .stroke_path(&b, &lid_lash, &self.stroke_outline(2.15), body_tx, None);
                    }
                }
                EyeExpression::Sleeping => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.4, ey - 0.4);
                    pb.quad_to(ex, ey + 4.2, ex + 5.4, ey - 0.4);
                    if let Some(arc) = pb.finish() {
                        let rim = Self::solid_paint(self.palette.eye_sclera.with_alpha(0.7));
                        self.pixmap
                            .stroke_path(&arc, &rim, &self.stroke_outline(3.8), body_tx, None);
                        self.pixmap
                            .stroke_path(&arc, &pupil, &self.stroke_outline(2.3), body_tx, None);
                    }
                }
            }
        }
    }

    /// Nailong-faithful glossy olive-emerald iris with large deep dark-forest upper-center pupil.
    /// The bright translucent green ring glows primarily around the lower & side crescent, while
    /// the top/center melts into a deep hollow void pupil (zero heap allocation).
    fn draw_nailong_green_iris(&mut self, body_tx: Transform, cx: f32, cy: f32, rx: f32, ry: f32) {
        // 1. Deep moss-green outer iris rim.
        if let Some(rim) = ellipse_path(cx, cy, rx, ry) {
            let paint = Self::solid_paint(self.palette.eye_green_outer);
            self.pixmap
                .fill_path(&rim, &paint, FillRule::Winding, body_tx, None);
        }
        // 2. Luminous olive-emerald green glow shifted slightly downward to form Nailong's green lower crescent!
        if let Some(glow) = ellipse_path(cx, cy + ry * 0.14, rx * 0.85, ry * 0.80) {
            let paint = Self::solid_paint(self.palette.eye_green_inner);
            self.pixmap
                .fill_path(&glow, &paint, FillRule::Winding, body_tx, None);
        }
        // 3. Deep hollow dark-forest pupil occupying upper-center ~74% of the iris!
        if let Some(pup) = ellipse_path(cx, cy - ry * 0.10, rx * 0.72, ry * 0.74) {
            let paint = Self::solid_paint(self.palette.eye_pupil);
            self.pixmap
                .fill_path(&pup, &paint, FillRule::Winding, body_tx, None);
        }
    }

    /// IP Anchor #2b: Expressive wide frog mouth ("蛙嘴")!
    fn draw_frog_mouth(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let mx = 6.5;
        let my = m.mouth_y;
        let line = Self::solid_paint(self.palette.mouth_line);

        match pose.beak {
            BeakPose::Closed => {
                // Iconic wide deadpan-chill frog mouth with gentle upward frog-lip dip & tucked corners!
                let mut pb = PathBuilder::new();
                pb.move_to(mx - 9.2, my - 0.6);
                pb.cubic_to(mx - 4.2, my + 3.4, mx - 1.2, my + 0.2, mx, my + 0.8);
                pb.cubic_to(mx + 1.2, my + 0.2, mx + 4.2, my + 3.4, mx + 9.2, my - 0.6);
                if let Some(path) = pb.finish() {
                    self.pixmap
                        .stroke_path(&path, &line, &self.stroke_outline(2.35), body_tx, None);
                }
                // Cute tiny corner dimple ticks typical of a frog grin.
                let mut corners = PathBuilder::new();
                corners.move_to(mx - 10.2, my - 1.4);
                corners.line_to(mx - 8.6, my - 0.2);
                corners.move_to(mx + 10.2, my - 1.4);
                corners.line_to(mx + 8.6, my - 0.2);
                if let Some(cp) = corners.finish() {
                    self.pixmap
                        .stroke_path(&cp, &line, &self.stroke_outline(1.75), body_tx, None);
                }
            }
            BeakPose::Chirp => {
                // Adorable startled/curious little dark oval "o" mouth (just like Nailong right pose!).
                if let Some(o_mouth) = ellipse_path(mx, my + 1.2, 3.6, 4.6) {
                    let cavity = Self::solid_paint(self.palette.mouth_cavity);
                    self.pixmap
                        .fill_path(&o_mouth, &cavity, FillRule::Winding, body_tx, None);
                    self.pixmap.stroke_path(
                        &o_mouth,
                        &line,
                        &self.stroke_outline(1.9),
                        body_tx,
                        None,
                    );
                }
            }
            BeakPose::HappyOpen => {
                // Big joyful wide frog grin open mouth with rosy tongue inside!
                let mut pb = PathBuilder::new();
                pb.move_to(mx - 9.6, my - 1.6);
                pb.quad_to(mx, my - 0.4, mx + 9.6, my - 1.6);
                pb.cubic_to(mx + 9.2, my + 7.2, mx + 4.5, my + 9.5, mx, my + 9.5);
                pb.cubic_to(mx - 4.5, my + 9.5, mx - 9.2, my + 7.2, mx - 9.6, my - 1.6);
                pb.close();
                if let Some(mouth) = pb.finish() {
                    let cavity = Self::solid_paint(self.palette.mouth_cavity);
                    self.pixmap
                        .fill_path(&mouth, &cavity, FillRule::Winding, body_tx, None);

                    // Rosy frog tongue nestled inside lower grin.
                    if let Some(tongue) = ellipse_path(mx, my + 5.8, 5.4, 2.8) {
                        let tp = Self::solid_paint(self.palette.mouth_tongue);
                        self.pixmap
                            .fill_path(&tongue, &tp, FillRule::Winding, body_tx, None);
                    }

                    self.pixmap.stroke_path(
                        &mouth,
                        &line,
                        &self.stroke_outline(2.05),
                        body_tx,
                        None,
                    );
                }
            }
        }
    }

    /// IP Anchor #1 + Custom Hairstyle: Renders 4 genuinely distinct, head-hugging chibi hairstyles
    /// (seamless ahoge / milk-tea fluffy curls / twin buns / cocoa bob bangs) with the signature
    /// reference-image-1 Little Flame ("小火苗") floating cleanly in mid-air above the hairstyle!
    fn draw_hair_and_little_flame(&mut self, body_tx: Transform, pose: &PetPose, m: &BodyMetrics) {
        let sway_deg = pose.ahoge_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.35;
        let crown_tx = body_tx.pre_translate(0.0, m.crown_y);
        let head_scale_x = m.head_rx / 34.5;
        let outline = Self::solid_paint(self.palette.outline);

        // Step A: Draw natural head-hugging customizable hairstyle.
        // Returns local clearance `hair_top_y` so the floating little flame hovers cleanly above it.
        let hair_top_y: f32 = match self.appearance.hair {
            HairStyle::FlameSweep => {
                // Style 1: "自然呆毛" — Clean, seamless curved twin-strand bird ahoge sprouting
                // organically from the crown with NO horizontal seam across the forehead!
                let tx = crown_tx.pre_translate(1.0, 1.0).pre_rotate(sway_deg * 0.55);
                let mut pb = PathBuilder::new();
                // Tall slender curved strand + delicate small side curl
                pb.move_to(-3.5, 1.5);
                pb.cubic_to(-7.5, -4.5, -4.5, -13.0, 3.8, -13.5);
                pb.cubic_to(-0.5, -9.8, 1.0, -5.0, 1.8, -1.2);
                pb.cubic_to(4.8, -6.5, 10.5, -7.2, 10.2, -2.5);
                pb.cubic_to(7.2, -2.8, 4.8, -0.4, 3.8, 1.8);
                pb.close();
                if let Some(p) = pb.finish() {
                    let fill = Self::solid_paint(self.palette.hair_base);
                    self.pixmap
                        .fill_path(&p, &fill, FillRule::Winding, tx, None);
                    self.pixmap.stroke_path(
                        &p,
                        &outline,
                        &self.stroke_outline(self.rig.outline_width * 0.85),
                        tx,
                        None,
                    );
                    // Seamlessly blend the root into the coral forehead so it grows naturally out of the head!
                    if let Some(seam) = ellipse_path(0.3, 2.5, 5.2, 2.6) {
                        self.pixmap
                            .fill_path(&seam, &fill, FillRule::Winding, tx, None);
                    }
                }
                -11.5
            }
            HairStyle::FluffyCloud => {
                // Style 2: "奶咖软卷" — Cozy creamy milk-tea curly hair wrapping smoothly across the
                // upper dome & framing both temples with soft airy bangs above the green eyes!
                let cap_tx = crown_tx.pre_scale(head_scale_x, 1.0);
                let mut pb = PathBuilder::new();
                // Outer fluffy curved crown hugging the upper head cleanly
                pb.move_to(-34.5, 19.5);
                pb.cubic_to(-36.5, 4.0, -24.0, -6.8, 0.0, -6.8);
                pb.cubic_to(24.0, -6.8, 36.5, 4.0, 34.5, 19.5);
                // Soft curly side locks & airy scalloped curls framing forehead (clears eyes cleanly!)
                pb.quad_to(30.0, 22.0, 27.5, 15.0);
                pb.quad_to(23.5, 18.0, 18.5, 13.0);
                pb.quad_to(11.5, 17.5, 4.5, 12.2);
                pb.quad_to(-2.5, 17.8, -10.5, 12.5);
                pb.quad_to(-18.0, 17.5, -24.5, 13.2);
                pb.quad_to(-29.5, 20.5, -34.5, 19.5);
                pb.close();

                if let Some(hair) = pb.finish() {
                    let mut paint = Paint {
                        anti_alias: true,
                        ..Paint::default()
                    };
                    if let Some(shader) = LinearGradient::new(
                        Point::from_xy(0.0, -7.0),
                        Point::from_xy(0.0, 20.0),
                        vec![
                            GradientStop::new(0.0, self.palette.belly_highlight.to_skia()),
                            GradientStop::new(0.45, self.palette.hair_cream.to_skia()),
                            GradientStop::new(1.0, self.palette.hair_cream_shade.to_skia()),
                        ],
                        SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        paint.shader = shader;
                    } else {
                        paint.set_color(self.palette.hair_cream.to_skia());
                    }
                    self.pixmap
                        .fill_path(&hair, &paint, FillRule::Winding, cap_tx, None);

                    // Delicate glossy curl shine band across the crown.
                    if let Some(hi) = ellipse_path(2.0, 1.8, 14.0, 2.8) {
                        let shine = Self::solid_paint(self.palette.belly_highlight.with_alpha(0.6));
                        self.pixmap
                            .fill_path(&hi, &shine, FillRule::Winding, cap_tx, None);
                    }

                    self.pixmap.stroke_path(
                        &hair,
                        &outline,
                        &self.stroke_outline(self.rig.outline_width * 0.85),
                        cap_tx,
                        None,
                    );
                }
                -7.0
            }
            HairStyle::TwinSparks => {
                // Style 3: "俏皮双丸子" — Adorable chibi twin buns (odango) with gold hair-ties +
                // neat cocoa side-parted bangs hugging the upper forehead, leaving center open for the flame!
                let cap_tx = crown_tx.pre_scale(head_scale_x, 1.0);
                let bun_fill = Self::solid_paint(self.palette.hair_cocoa);
                let bun_hi = Self::solid_paint(self.palette.hair_cocoa_hi);
                let tie_gold = Self::solid_paint(self.palette.hair_tie_gold);

                // Left & right cute round chibi hair buns perched on the upper crown corners!
                for &side in &[-1.0f32, 1.0f32] {
                    let bx = side * 24.5;
                    let by = 1.5;
                    let btx = cap_tx
                        .pre_translate(bx, by)
                        .pre_rotate(side * (8.0 + sway_deg * 0.3));
                    if let Some(bun) = ellipse_path(0.0, 0.0, 10.5, 9.8) {
                        self.pixmap
                            .fill_path(&bun, &bun_fill, FillRule::Winding, btx, None);
                        if let Some(sh) = ellipse_path(-side * 2.2, -3.2, 4.2, 2.4) {
                            self.pixmap
                                .fill_path(&sh, &bun_hi, FillRule::Winding, btx, None);
                        }
                        self.pixmap.stroke_path(
                            &bun,
                            &outline,
                            &self.stroke_outline(self.rig.outline_width * 0.85),
                            btx,
                            None,
                        );
                    }
                    // Cute little golden hair-tie collar at the base of each bun!
                    let mut tie = PathBuilder::new();
                    tie.move_to(-side * 8.2, 5.2);
                    tie.quad_to(-side * 3.0, 8.8, side * 1.5, 8.5);
                    if let Some(tp) = tie.finish() {
                        self.pixmap
                            .stroke_path(&tp, &tie_gold, &self.stroke_outline(3.2), btx, None);
                    }
                }

                // Sleek cocoa hair cap with cute center-parted curtain bangs hugging the forehead!
                let mut pb = PathBuilder::new();
                pb.move_to(-32.5, 17.5);
                pb.cubic_to(-33.5, 3.0, -21.5, -4.5, 0.0, -4.5);
                pb.cubic_to(21.5, -4.5, 33.5, 3.0, 32.5, 17.5);
                // Side-parted curtain bangs sweeping gently outward above the green eyes
                pb.quad_to(24.0, 13.5, 13.5, 12.2);
                pb.quad_to(6.0, 11.2, 2.0, 7.2);
                pb.quad_to(-3.5, 11.5, -13.5, 12.2);
                pb.quad_to(-24.0, 13.5, -32.5, 17.5);
                pb.close();

                if let Some(hair) = pb.finish() {
                    self.pixmap
                        .fill_path(&hair, &bun_fill, FillRule::Winding, cap_tx, None);
                    if let Some(sh) = ellipse_path(0.0, 1.5, 12.0, 2.2) {
                        self.pixmap
                            .fill_path(&sh, &bun_hi, FillRule::Winding, cap_tx, None);
                    }
                    self.pixmap.stroke_path(
                        &hair,
                        &outline,
                        &self.stroke_outline(self.rig.outline_width * 0.85),
                        cap_tx,
                        None,
                    );
                }
                -5.0
            }
            HairStyle::BangsTuft => {
                // Style 4: "元气齐刘海" — Classic glossy warm-cocoa chibi bob wrapping the full head
                // with feathered air-bangs neatly framing the eyes and cute cheek side-locks!
                let cap_tx = crown_tx.pre_scale(head_scale_x, 1.0);
                let mut pb = PathBuilder::new();
                // Full smooth bob dome + cheek-framing side locks
                pb.move_to(-36.2, 23.5);
                pb.cubic_to(-38.0, 4.5, -24.5, -5.8, 0.0, -5.8);
                pb.cubic_to(24.5, -5.8, 38.0, 4.5, 36.2, 23.5);
                // Right cheek lock curving inward
                pb.quad_to(32.5, 25.5, 30.2, 16.5);
                // Clean straight-ish airy feathered bangs with 2 delicate parting notches!
                pb.line_to(24.0, 13.0);
                pb.line_to(18.5, 14.2);
                pb.line_to(14.0, 10.2);
                pb.line_to(10.0, 14.5);
                pb.line_to(-4.5, 14.5);
                pb.line_to(-8.5, 10.4);
                pb.line_to(-13.0, 14.2);
                pb.line_to(-24.0, 13.0);
                // Left cheek lock curving inward
                pb.line_to(-30.2, 16.5);
                pb.quad_to(-32.5, 25.5, -36.2, 23.5);
                pb.close();

                if let Some(hair) = pb.finish() {
                    let mut paint = Paint {
                        anti_alias: true,
                        ..Paint::default()
                    };
                    if let Some(shader) = LinearGradient::new(
                        Point::from_xy(0.0, -6.0),
                        Point::from_xy(0.0, 24.0),
                        vec![
                            GradientStop::new(0.0, self.palette.hair_cocoa_hi.to_skia()),
                            GradientStop::new(0.45, self.palette.hair_cocoa.to_skia()),
                            GradientStop::new(1.0, self.palette.outline.to_skia()),
                        ],
                        SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        paint.shader = shader;
                    } else {
                        paint.set_color(self.palette.hair_cocoa.to_skia());
                    }
                    self.pixmap
                        .fill_path(&hair, &paint, FillRule::Winding, cap_tx, None);

                    // Anime-style glossy hair highlight halo arcs across the bob!
                    let shine = Self::solid_paint(self.palette.hair_cocoa_hi.with_alpha(0.75));
                    for &(sx, sy, rx, ry) in &[(-11.0, 3.0, 7.5, 2.0), (11.5, 3.0, 8.0, 2.0)] {
                        if let Some(arc) = ellipse_path(sx, sy, rx, ry) {
                            self.pixmap
                                .fill_path(&arc, &shine, FillRule::Winding, cap_tx, None);
                        }
                    }

                    self.pixmap.stroke_path(
                        &hair,
                        &outline,
                        &self.stroke_outline(self.rig.outline_width * 0.85),
                        cap_tx,
                        None,
                    );
                }
                -6.0
            }
        };

        // Step B: IP Anchor #1 — The Reference-Image-1 Little Flame ("小火苗") FLOATING cleanly above the hair!
        // Deliberate floating air gap + gentle vertical bob + soft radial fade-to-zero glow + tiny ember dot.
        let hover_bob = -1.6 * pose.breath - 1.2 * pose.badge_glow;
        let float_y = hair_top_y - 5.2 + hover_bob;
        let flicker = 1.0 + 0.04 * pose.breath + 0.07 * pose.badge_glow;
        let fscale = self.appearance.flame_scale * 0.78 * flicker;
        let flame_tx = crown_tx
            .pre_translate(1.5, float_y)
            .pre_rotate(sway_deg * 1.15)
            .pre_scale(fscale, fscale);

        // True radial glow fading smoothly to 0% alpha (zero hard bubble edge on light or dark desktops).
        if let Some(halo) = ellipse_path(1.0, -7.5, 18.0, 16.0) {
            let mut glow = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(1.0, -7.5),
                Point::from_xy(1.0, -7.5),
                17.0,
                vec![
                    GradientStop::new(0.0, self.palette.flame_core.with_alpha(0.30).to_skia()),
                    GradientStop::new(0.55, self.palette.flame_mid.with_alpha(0.10).to_skia()),
                    GradientStop::new(1.0, self.palette.flame_mid.with_alpha(0.0).to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                glow.shader = shader;
                self.pixmap
                    .fill_path(&halo, &glow, FillRule::Winding, flame_tx, None);
            }
        }

        // Exact reference image 1 silhouette: sleek S-curved left flame blade + 3 sharp stepped spikes!
        let mut flame = PathBuilder::new();
        flame.move_to(-6.2, 2.4);
        flame.cubic_to(-8.6, -3.5, -3.2, -10.8, -11.4, -24.5);
        flame.cubic_to(-3.8, -19.0, -0.2, -11.4, -1.6, -4.6);
        flame.line_to(1.8, -10.5);
        flame.line_to(2.8, -4.8);
        flame.line_to(7.8, -10.3);
        flame.line_to(7.2, -4.0);
        flame.line_to(13.8, -8.8);
        flame.cubic_to(13.2, -3.0, 11.4, 0.8, 8.6, 2.4);
        flame.quad_to(1.2, 5.0, -6.2, 2.4);
        flame.close();

        if let Some(flame_path) = flame.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(-10.0, -24.5),
                Point::from_xy(6.0, 3.5),
                vec![
                    GradientStop::new(0.0, self.palette.flame_core.to_skia()),
                    GradientStop::new(0.42, self.palette.flame_mid.to_skia()),
                    GradientStop::new(1.0, self.palette.flame_outer.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.flame_mid.to_skia());
            }
            self.pixmap
                .fill_path(&flame_path, &paint, FillRule::Winding, flame_tx, None);

            let flame_rim = Self::solid_paint(self.palette.outline.with_alpha(0.72));
            self.pixmap.stroke_path(
                &flame_path,
                &flame_rim,
                &self.stroke_outline(1.4),
                flame_tx,
                None,
            );
        }

        // Tiny floating spark dot hovering beside the flame tip to emphasize airy levitation!
        if let Some(ember) = ellipse_path(12.2, -14.5 + hover_bob * 0.6, 1.8, 1.8) {
            let ep = Self::solid_paint(self.palette.flame_core.with_alpha(0.9));
            self.pixmap
                .fill_path(&ember, &ep, FillRule::Winding, flame_tx, None);
        }
    }

    fn draw_prop(
        &mut self,
        anchor_x: f32,
        body_y: f32,
        facing: f32,
        pose: &PetPose,
        scale: f32,
        m: &BodyMetrics,
    ) {
        let base_tx = Transform::from_scale(scale, scale);
        let top_y = body_y + m.crown_y;
        match pose.prop {
            PropKind::None => {}
            PropKind::Hearts { phase, count } => {
                for i in 0..count.min(2) {
                    let p = (phase + i as f32 * 0.45).fract();
                    let fade = (1.0 - p).clamp(0.0, 1.0);
                    let hx =
                        anchor_x + facing * (30.0 + i as f32 * 13.0) + (p * PI * 2.0).sin() * 3.0;
                    let hy = top_y + 8.0 - p * 22.0 - i as f32 * 7.0;
                    let size = (6.8 - i as f32 * 1.4) * (0.8 + 0.25 * (p * PI).sin());
                    if let Some(heart) = heart_path(hx, hy, size) {
                        let fill = Self::solid_paint(self.palette.heart.with_alpha(fade));
                        self.pixmap
                            .fill_path(&heart, &fill, FillRule::Winding, base_tx, None);
                    }
                }
            }
            PropKind::SleepZzz { phase } => {
                for i in 0..2 {
                    let p = (phase + i as f32 * 0.5).fract();
                    let fade = ((p * 3.0).min(1.0) * (1.0 - p)).clamp(0.0, 1.0);
                    let zx = anchor_x + facing * (27.0 + p * 14.0 + i as f32 * 6.0);
                    let zy = top_y + 14.0 - p * 26.0 - i as f32 * 6.0;
                    let s = 5.0 + i as f32 * 2.4 + p * 1.5;
                    let mut pb = PathBuilder::new();
                    pb.move_to(zx - s * 0.5, zy - s * 0.5);
                    pb.line_to(zx + s * 0.5, zy - s * 0.5);
                    pb.line_to(zx - s * 0.5, zy + s * 0.5);
                    pb.line_to(zx + s * 0.5, zy + s * 0.5);
                    if let Some(z_path) = pb.finish() {
                        let paint =
                            Self::solid_paint(self.palette.sleep_zzz.with_alpha(fade * 0.95));
                        self.pixmap.stroke_path(
                            &z_path,
                            &paint,
                            &self.stroke_outline(2.1),
                            base_tx,
                            None,
                        );
                    }
                }
            }
            PropKind::UnreadBadge { count, pulse } => {
                let bx = anchor_x + facing * 36.0;
                let by = top_y + 3.0 - pulse * 2.5;
                let halo_r = 12.2 + pulse * 4.5;
                if let Some(halo) = ellipse_path(bx, by, halo_r, halo_r) {
                    let halo_paint = Self::solid_paint(
                        self.palette.badge_alert.with_alpha((1.0 - pulse) * 0.38),
                    );
                    self.pixmap
                        .fill_path(&halo, &halo_paint, FillRule::Winding, base_tx, None);
                }
                if let Some(pill) = ellipse_path(bx, by, 10.8, 10.8) {
                    let fill = Self::solid_paint(self.palette.badge_alert);
                    let ring = Self::solid_paint(self.palette.eye_sparkle);
                    self.pixmap
                        .fill_path(&pill, &fill, FillRule::Winding, base_tx, None);
                    self.pixmap.stroke_path(
                        &pill,
                        &ring,
                        &self.stroke_outline(2.0),
                        base_tx,
                        None,
                    );
                }
                let mut env = PathBuilder::new();
                env.move_to(bx - 5.0, by - 3.0);
                env.line_to(bx + 5.0, by - 3.0);
                env.line_to(bx + 5.0, by + 3.4);
                env.line_to(bx - 5.0, by + 3.4);
                env.close();
                env.move_to(bx - 4.8, by - 2.6);
                env.line_to(bx, by + 0.8);
                env.line_to(bx + 4.8, by - 2.6);
                if let Some(env_path) = env.finish() {
                    let white = Self::solid_paint(self.palette.eye_sparkle);
                    self.pixmap.stroke_path(
                        &env_path,
                        &white,
                        &self.stroke_outline(1.5),
                        base_tx,
                        None,
                    );
                }
                if count > 1
                    && let Some(dot) = ellipse_path(bx + 8.5, by - 7.5, 3.2, 3.2)
                {
                    let gold = Self::solid_paint(self.palette.sparkle);
                    self.pixmap
                        .fill_path(&dot, &gold, FillRule::Winding, base_tx, None);
                }
            }
            PropKind::Sparkles { phase } => {
                let coords = [(-36.0, -29.0, 7.2), (39.0, -34.0, 8.4), (43.0, -8.0, 5.6)];
                for (i, &(ox, oy, r)) in coords.iter().enumerate() {
                    let p = (phase + i as f32 * 0.33).fract();
                    let twinkle = (p * PI).sin();
                    let sr = r * (0.45 + 0.55 * twinkle);
                    if let Some(star) = star4_path(anchor_x + ox * facing, body_y + oy, sr) {
                        let fill = Self::solid_paint(
                            self.palette.sparkle.with_alpha(0.35 + 0.65 * twinkle),
                        );
                        self.pixmap
                            .fill_path(&star, &fill, FillRule::Winding, base_tx, None);
                    }
                }
            }
            PropKind::FocusRipple { phase } => {
                let cx = anchor_x + facing * 14.0;
                let cy = top_y - 11.0;
                for i in 0..2 {
                    let p = (phase + i as f32 * 0.5).fract();
                    let r = 6.0 + p * 12.0;
                    let fade = (1.0 - p) * 0.65;
                    let mut pb = PathBuilder::new();
                    pb.move_to(cx - r * 0.6, cy - r * 0.45);
                    pb.quad_to(cx, cy - r * 0.95, cx + r * 0.6, cy - r * 0.45);
                    if let Some(arc) = pb.finish() {
                        let paint = Self::solid_paint(self.palette.badge_ok.with_alpha(fade));
                        self.pixmap.stroke_path(
                            &arc,
                            &paint,
                            &self.stroke_outline(1.8),
                            base_tx,
                            None,
                        );
                    }
                }
            }
        }
    }
}

/// Build an axis-aligned smooth 4-cubic-bezier ellipse path.
fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Path> {
    if rx <= 0.01 || ry <= 0.01 {
        return None;
    }
    const K: f32 = 0.552_284_8;
    let kx = rx * K;
    let ky = ry * K;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - ry);
    pb.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    pb.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    pb.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    pb.close();
    pb.finish()
}

/// Rounded tail feather extending to the left from `(0, 0)`.
fn feather_path(len: f32, half_w: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(2.0, -half_w * 0.55);
    pb.cubic_to(-len * 0.45, -half_w * 1.15, -len * 0.95, -half_w * 0.85, -len, 0.0);
    pb.cubic_to(-len * 0.95, half_w * 0.85, -len * 0.45, half_w * 1.15, 2.0, half_w * 0.55);
    pb.close();
    pb.finish()
}

/// Sleek sculpted two-tone feathered bird wing pivoted near `(0, 0)`.
fn sculpted_wing_path(dir: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, -4.5);
    pb.cubic_to(dir * 9.5, -6.5, dir * 19.5, 0.5, dir * 18.0, 11.5);
    pb.quad_to(dir * 16.5, 16.5, dir * 11.5, 15.8);
    pb.quad_to(dir * 10.5, 19.5, dir * 5.2, 17.8);
    pb.cubic_to(dir * 0.5, 16.0, -dir * 3.5, 9.5, -dir * 2.5, 2.0);
    pb.cubic_to(-dir * 3.0, -2.0, -dir * 1.5, -4.0, 0.0, -4.5);
    pb.close();
    pb.finish()
}

/// Smooth vector heart centered at `(cx, cy)`.
fn heart_path(cx: f32, cy: f32, size: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy + size * 0.75);
    pb.cubic_to(
        cx - size * 1.15,
        cy + size * 0.05,
        cx - size * 0.95,
        cy - size * 0.95,
        cx,
        cy - size * 0.28,
    );
    pb.cubic_to(
        cx + size * 0.95,
        cy - size * 0.95,
        cx + size * 1.15,
        cy + size * 0.05,
        cx,
        cy + size * 0.75,
    );
    pb.close();
    pb.finish()
}

/// 4-point sparkle star centered at `(cx, cy)`.
fn star4_path(cx: f32, cy: f32, r: f32) -> Option<Path> {
    let inner = r * 0.26;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.quad_to(cx + inner, cy - inner, cx + r, cy);
    pb.quad_to(cx + inner, cy + inner, cx, cy + r);
    pb.quad_to(cx - inner, cy + inner, cx - r, cy);
    pb.quad_to(cx - inner, cy - inner, cx, cy - r);
    pb.close();
    pb.finish()
}

/// Compose a multi-pose studio showcase contact sheet (4 columns x N rows) on split light/dark tiles.
pub fn render_showcase_sheet(poses: &[(&str, PetPose)], scale: f32) -> Pixmap {
    let items: Vec<(&str, PetAppearance, PetPose)> = poses
        .iter()
        .map(|&(name, pose)| (name, PetAppearance::default(), pose))
        .collect();
    render_variant_sheet(&items, scale)
}

/// Compose a 4-column studio showcase sheet where each tile can use a custom [`PetAppearance`].
pub fn render_variant_sheet(items: &[(&str, PetAppearance, PetPose)], scale: f32) -> Pixmap {
    let cols = 4u32;
    let rows = ((items.len() as u32).saturating_add(cols - 1) / cols).max(1);
    let cell = (LOGICAL_CANVAS_SIZE * scale).round() as u32;
    let pad = (16.0 * scale).round() as u32;
    let width = cols * cell + (cols + 1) * pad;
    let height = rows * cell + (rows + 1) * pad;

    let mut sheet = Pixmap::new(width, height).expect("valid showcase sheet size");
    sheet.fill(Color::from_rgba8(229, 231, 235, 255));

    let mut renderer = PetRenderer::new(scale);
    let light_bg = Color::from_rgba8(255, 255, 255, 255);
    let dark_bg = Color::from_rgba8(30, 41, 59, 255);

    for (idx, (_, appearance, pose)) in items.iter().enumerate() {
        let col = (idx as u32) % cols;
        let row = (idx as u32) / cols;
        let ox = pad + col * (cell + pad);
        let oy = pad + row * (cell + pad);

        let half = cell / 2;
        if let Some(rect) =
            tiny_skia::Rect::from_xywh(ox as f32, oy as f32, half as f32, cell as f32)
        {
            let mut p = Paint::default();
            p.set_color(light_bg);
            sheet.fill_rect(rect, &p, Transform::identity(), None);
        }
        if let Some(rect) = tiny_skia::Rect::from_xywh(
            (ox + half) as f32,
            oy as f32,
            (cell - half) as f32,
            cell as f32,
        ) {
            let mut p = Paint::default();
            p.set_color(dark_bg);
            sheet.fill_rect(rect, &p, Transform::identity(), None);
        }

        renderer.set_appearance(*appearance);
        let frame = renderer.render(pose);
        sheet.draw_pixmap(
            ox as i32,
            oy as i32,
            frame.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    sheet
}
