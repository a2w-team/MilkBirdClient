//! Zero-jitter pre-allocated `tiny-skia` vector renderer and O(1) alpha HitMask
//! for the original "小奶鸟 (MilkBird)" character.

use crate::model::{
    BeakPose, EyeExpression, FootPose, LOGICAL_CANVAS_SIZE, Palette, PetPose, PetRig, PropKind,
    Rgba,
};
use std::f32::consts::PI;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, Point, RadialGradient, SpreadMode, Stroke, Transform,
};

/// Alpha threshold (out of 255) for pixel-level pointer hit-testing.
const HIT_ALPHA_THRESHOLD: u8 = 18;

/// Pre-allocated CPU vector renderer. Reuses its internal `Pixmap` across frames.
pub struct PetRenderer {
    rig: PetRig,
    palette: Palette,
    scale: f32,
    pixmap: Pixmap,
}

impl PetRenderer {
    /// Allocate a renderer for the given Retina scale factor (`1.0`, `2.0`, `3.0`).
    pub fn new(scale: f32) -> Self {
        Self::with_config(PetRig::default(), Palette::milkbird_day(), scale)
    }

    pub fn with_config(rig: PetRig, palette: Palette, scale: f32) -> Self {
        let scale = scale.clamp(1.0, 4.0);
        let px = (rig.canvas_size * scale).round().max(16.0) as u32;
        let pixmap = Pixmap::new(px, px).expect("valid pet pixmap dimensions");
        Self {
            rig,
            palette,
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

    /// Render the complete 11-layer MilkBird character into the pre-allocated `Pixmap`.
    pub fn render(&mut self, pose: &PetPose) -> &Pixmap {
        self.pixmap.data_mut().fill(0);

        let scale = self.scale;
        let anchor_x = self.rig.anchor_x + pose.offset_x;
        let ground_y = self.rig.ground_y;
        let facing = if pose.facing >= 0.0 { 1.0 } else { -1.0 };

        // 1. Ground shadow (stays on ground, shrinks softly as bird hops/dangles).
        self.draw_ground_shadow(anchor_x, ground_y, pose, scale);

        // Compute world transform for the bird body (pivot at bottom-center contact point,
        // lifted by altitude, tilted, squashed/stretched, and mirrored by facing).
        let breath_scale_x = 1.0 + 0.018 * pose.breath;
        let breath_scale_y = 1.0 + 0.022 * pose.breath;
        let sx = pose.scale_x * breath_scale_x * facing;
        let sy = pose.scale_y * breath_scale_y;

        // Body center sits above ground_y; sleep curl lowers it closer to the ground.
        let rest_height = 40.0 - 7.5 * pose.sleep_curl;
        let body_y = ground_y - pose.altitude - rest_height * pose.scale_y;

        let body_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(sx, sy);

        // Un-squashed coordinate frame attached to body center for props/feet that shouldn't distort weirdly.
        let body_rigid_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(facing, 1.0);

        // 2. Tail feathers (behind body).
        self.draw_tail_feathers(body_tx, pose);

        // 3. Back wing (peeking behind front contour when flapping/alert).
        self.draw_back_wing(body_tx, pose);

        // 4. Feet (planted, walking, dangling, or tucked).
        self.draw_feet(body_rigid_tx, anchor_x, ground_y, facing, pose, scale);

        // 5. Plump chubby MilkBird body + soft milk gradient + fluffy chest glow + warm outline.
        self.draw_body_blob(body_tx);

        // 6. Signature MilkBird Club cobalt scarf + flowing twin ribbon tails + club badge.
        self.draw_scarf(body_tx, pose);

        // 7. Front oat-latte wing tucked at flank or fluttering excitedly.
        self.draw_front_wing(body_tx, pose);

        // 8. Soft peach cheek blushes.
        self.draw_cheeks(body_tx, pose);

        // 9. Expressive twin eyes (7 expressions + gaze tracking + crisp dual catchlights).
        self.draw_eyes(body_tx, pose);

        // 10. Cute apricot beak (closed / chirp / happy open).
        self.draw_beak(body_tx, pose);

        // 11. Playful double-curl milk-foam ahoge sprout on crown!
        self.draw_ahoge(body_tx, pose);

        // 12. Floating micro-prop overlay (hearts, sleep Zzz, unread badge, sparkles, radar).
        self.draw_prop(anchor_x, body_y, facing, pose, scale);

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
        let mut paint = Paint::default();
        paint.set_color(rgba.to_skia());
        paint.anti_alias = true;
        paint
    }

    fn draw_ground_shadow(&mut self, anchor_x: f32, ground_y: f32, pose: &PetPose, scale: f32) {
        let alt_factor = (1.0 - pose.altitude / 90.0).clamp(0.35, 1.0);
        let rx = (33.0 + 4.0 * pose.sleep_curl) * pose.scale_x * alt_factor;
        let ry = 7.5 * alt_factor;
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

    fn draw_tail_feathers(&mut self, body_tx: Transform, pose: &PetPose) {
        let pivot_x = -31.0;
        let pivot_y = 14.0;
        let wag_deg = pose.tail_angle.to_degrees();

        // Lower accent feather.
        let lower_tx = body_tx
            .pre_translate(pivot_x, pivot_y)
            .pre_rotate(16.0 + wag_deg * 0.85);
        if let Some(path) = feather_path(22.0, 7.2) {
            let fill = Self::solid_paint(self.palette.tail_accent);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, lower_tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                lower_tx,
                None,
            );
        }

        // Upper main feather.
        let upper_tx = body_tx
            .pre_translate(pivot_x + 1.5, pivot_y - 4.0)
            .pre_rotate(-14.0 + wag_deg);
        if let Some(path) = feather_path(25.0, 8.2) {
            let fill = Self::solid_paint(self.palette.tail_base);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, upper_tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.92),
                upper_tx,
                None,
            );
        }
    }

    fn draw_back_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        let lift_deg = -16.0 - pose.wing_flap.to_degrees() * 0.95 - pose.wing_spread * 20.0;
        let tx = body_tx
            .pre_translate(27.0 + pose.wing_spread * 3.0, 3.0 - pose.wing_spread * 3.0)
            .pre_rotate(lift_deg);
        if let Some(path) = wing_path(1.0) {
            let fill = Self::solid_paint(self.palette.wing_shade);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                tx,
                None,
            );
        }
    }

    fn draw_feet(
        &mut self,
        body_rigid_tx: Transform,
        anchor_x: f32,
        ground_y: f32,
        facing: f32,
        pose: &PetPose,
        scale: f32,
    ) {
        if pose.foot_pose == FootPose::Tucked && pose.sleep_curl > 0.65 {
            // Tiny cozy peek of toes under sleeping dumpling.
            let tx = Transform::from_scale(scale, scale)
                .pre_translate(anchor_x, ground_y - 3.5)
                .pre_scale(facing, 1.0);
            self.draw_single_foot(tx, -9.0, 0.0, 0.0);
            self.draw_single_foot(tx, 10.0, 0.0, 0.0);
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
                let stride_l = step * 6.0;
                let stride_r = -step * 6.0;
                let tx = Transform::from_scale(scale, scale)
                    .pre_translate(anchor_x, ground_y - 4.0 - pose.altitude)
                    .pre_scale(facing, 1.0);
                self.draw_single_foot(tx, -10.5 + stride_l, -lift_l, step * 18.0);
                self.draw_single_foot(tx, 11.5 + stride_r, -lift_r, -step * 18.0);
            }
            FootPose::Dangling => {
                let kick = (pose.foot_phase * 1.4).sin();
                self.draw_single_foot(body_rigid_tx, -10.5, 37.0, -24.0 + kick * 28.0);
                self.draw_single_foot(body_rigid_tx, 11.5, 37.0, 24.0 - kick * 28.0);
            }
        }
    }

    fn draw_single_foot(&mut self, base_tx: Transform, x: f32, y: f32, rot_deg: f32) {
        let tx = base_tx.pre_translate(x, y).pre_rotate(rot_deg);
        let mut pb = PathBuilder::new();
        // Cute stubby leg + rounded 3-toe footpad.
        pb.move_to(-2.2, -7.0);
        pb.line_to(2.2, -7.0);
        pb.line_to(2.5, -0.5);
        pb.quad_to(8.0, 0.0, 8.2, 3.2);
        pb.quad_to(6.2, 5.4, 2.6, 4.4);
        pb.quad_to(0.0, 5.8, -2.5, 4.4);
        pb.quad_to(-6.0, 5.2, -6.2, 2.4);
        pb.quad_to(-5.5, 0.0, -2.2, -0.5);
        pb.close();
        if let Some(path) = pb.finish() {
            let fill = Self::solid_paint(self.palette.foot_base);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(1.85),
                tx,
                None,
            );
        }
    }

    fn draw_body_blob(&mut self, body_tx: Transform) {
        // Plump chubby milk-dumpling shima-enaga silhouette!
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, -34.0);
        pb.cubic_to(23.5, -34.0, 38.5, -16.5, 37.5, 4.5);
        pb.cubic_to(36.8, 22.8, 23.0, 34.5, 0.0, 34.5);
        pb.cubic_to(-23.5, 34.5, -38.0, 22.8, -38.0, 4.5);
        pb.cubic_to(-38.0, -16.5, -23.5, -34.0, 0.0, -34.0);
        pb.close();

        let Some(body_path) = pb.finish() else {
            return;
        };

        // Warm creamy radial gradient giving soft plump 3D volume.
        let mut grad_paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(9.0, -12.0),
            Point::from_xy(4.0, -4.0),
            52.0,
            vec![
                GradientStop::new(0.0, self.palette.body_highlight.to_skia()),
                GradientStop::new(0.62, self.palette.body_base.to_skia()),
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

        // Soft fluffy tummy highlight.
        if let Some(belly) = ellipse_path(8.0, 14.0, 18.0, 13.0) {
            let paint = Self::solid_paint(self.palette.belly_glow.with_alpha(0.55));
            self.pixmap
                .fill_path(&belly, &paint, FillRule::Winding, body_tx, None);
        }

        // Subtle fluffy crown highlight sheen.
        if let Some(crown) = ellipse_path(6.0, -23.5, 13.0, 5.5) {
            let paint = Self::solid_paint(self.palette.belly_glow.with_alpha(0.65));
            self.pixmap
                .fill_path(&crown, &paint, FillRule::Winding, body_tx, None);
        }

        // Crisp warm cocoa contour so white bird pops cleanly on bright AND dark desktops.
        let outline = Self::solid_paint(self.palette.outline);
        self.pixmap.stroke_path(
            &body_path,
            &outline,
            &self.stroke_outline(self.rig.outline_width),
            body_tx,
            None,
        );
    }

    fn draw_scarf(&mut self, body_tx: Transform, pose: &PetPose) {
        let sway = pose.scarf_sway.to_degrees();
        let lift = pose.scarf_lift * 7.0;

        // 1. Two flowing ribbon tails anchored at the side knot (-18, 13).
        let knot_x = -17.5;
        let knot_y = 13.0;

        // Back ribbon tail (drapes downward & back behind the body).
        let tail_b_tx = body_tx
            .pre_translate(knot_x - 1.0, knot_y + 2.0)
            .pre_rotate(-18.0 + sway * 0.9 + lift);
        if let Some(tail_b) = scarf_tail_path(21.0, 6.4) {
            let fill = Self::solid_paint(self.palette.scarf_shade);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&tail_b, &fill, FillRule::Winding, tail_b_tx, None);
            self.pixmap.stroke_path(
                &tail_b,
                &outline,
                &self.stroke_outline(1.8),
                tail_b_tx,
                None,
            );
        }

        // Front ribbon tail draping cleanly downward with a cute cream campus stripe.
        let tail_f_tx = body_tx
            .pre_translate(knot_x + 2.0, knot_y + 3.0)
            .pre_rotate(-44.0 + sway * 1.15 + lift * 0.7);
        if let Some(tail_f) = scarf_tail_path(23.0, 7.2) {
            let fill = Self::solid_paint(self.palette.scarf_base);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&tail_f, &fill, FillRule::Winding, tail_f_tx, None);
            // Tiny cream stripe near ribbon tip.
            let mut sp = PathBuilder::new();
            sp.move_to(-16.5, -2.8);
            sp.line_to(-16.5, 2.8);
            if let Some(stripe) = sp.finish() {
                let stripe_paint = Self::solid_paint(self.palette.badge_idle.with_alpha(0.92));
                self.pixmap.stroke_path(
                    &stripe,
                    &stripe_paint,
                    &self.stroke_outline(2.1),
                    tail_f_tx,
                    None,
                );
            }
            self.pixmap.stroke_path(
                &tail_f,
                &outline,
                &self.stroke_outline(1.85),
                tail_f_tx,
                None,
            );
        }

        // 2. Main curved neck scarf band hugging the plump chest contour cleanly.
        let mut band = PathBuilder::new();
        band.move_to(-34.8, 7.8);
        band.quad_to(2.0, 16.5, 35.4, 6.8);
        band.cubic_to(36.2, 10.2, 35.4, 14.4, 33.4, 17.0);
        band.quad_to(2.0, 26.8, -32.6, 17.5);
        band.cubic_to(-35.0, 14.8, -35.8, 10.8, -34.8, 7.8);
        band.close();

        if let Some(band_path) = band.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, 7.0),
                Point::from_xy(0.0, 26.0),
                vec![
                    GradientStop::new(0.0, self.palette.scarf_highlight.to_skia()),
                    GradientStop::new(0.45, self.palette.scarf_base.to_skia()),
                    GradientStop::new(1.0, self.palette.scarf_shade.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.scarf_base.to_skia());
            }
            self.pixmap
                .fill_path(&band_path, &paint, FillRule::Winding, body_tx, None);

            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap.stroke_path(
                &band_path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                body_tx,
                None,
            );
        }

        // 3. Cute MilkBird Club circular pin badge on the scarf knot!
        let badge_r = 4.6 + 0.6 * pose.badge_glow;
        if let Some(pin) = ellipse_path(knot_x + 3.0, knot_y + 2.0, badge_r, badge_r) {
            let badge_color = if pose.badge_glow > 0.4 {
                self.palette.badge_alert
            } else if pose.badge_glow > 0.1 {
                self.palette.badge_ok
            } else {
                self.palette.badge_idle
            };
            let fill = Self::solid_paint(badge_color);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&pin, &fill, FillRule::Winding, body_tx, None);
            self.pixmap.stroke_path(
                &pin,
                &outline,
                &self.stroke_outline(1.6),
                body_tx,
                None,
            );
            // Tiny glossy highlight dot on the badge.
            if let Some(dot) = ellipse_path(knot_x + 1.8, knot_y + 0.7, 1.3, 1.3) {
                let gloss = Self::solid_paint(self.palette.eye_sparkle.with_alpha(0.85));
                self.pixmap
                    .fill_path(&dot, &gloss, FillRule::Winding, body_tx, None);
            }
        }
    }

    fn draw_front_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        // Pivots cozily at left flank (-19, 5); flaps gracefully upward when excited.
        let flap_deg = -4.0 + pose.wing_flap.to_degrees() * 1.15 + pose.wing_spread * 28.0;
        let tx = body_tx
            .pre_translate(-19.5 - pose.wing_spread * 2.0, 5.5 - pose.wing_spread * 2.0)
            .pre_rotate(flap_deg);
        if let Some(path) = wing_path(-1.0) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, -6.0),
                Point::from_xy(-14.0, 18.0),
                vec![
                    GradientStop::new(0.0, self.palette.wing_tip.to_skia()),
                    GradientStop::new(0.55, self.palette.wing_base.to_skia()),
                    GradientStop::new(1.0, self.palette.wing_shade.to_skia()),
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

    fn draw_cheeks(&mut self, body_tx: Transform, pose: &PetPose) {
        let alpha = pose.blush.clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let paint = Self::solid_paint(self.palette.cheek.with_alpha(alpha * 0.72));
        for &(cx, cy, rx, ry) in &[(-9.5, -2.0, 6.0, 3.6), (27.5, -2.0, 5.4, 3.4)] {
            if let Some(cheek) = ellipse_path(cx, cy, rx, ry) {
                self.pixmap
                    .fill_path(&cheek, &paint, FillRule::Winding, body_tx, None);
            }
        }
    }

    fn draw_eyes(&mut self, body_tx: Transform, pose: &PetPose) {
        let eye_positions = [(-2.0, -10.5), (21.5, -10.5)];
        let ink = Self::solid_paint(self.palette.eye_ink);
        let sparkle = Self::solid_paint(self.palette.eye_sparkle);
        let gx = pose.gaze_x.clamp(-1.0, 1.0) * 1.5;
        let gy = pose.gaze_y.clamp(-1.0, 1.0) * 1.2;

        for &(ex, ey) in &eye_positions {
            match pose.eyes {
                EyeExpression::Normal => {
                    let r = self.rig.eye_radius;
                    if let Some(eye) = ellipse_path(ex + gx * 0.45, ey + gy * 0.45, r, r * 1.06) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    // Primary bright catchlight + tiny lower sparkle = super lively eyes!
                    if let Some(hi1) =
                        ellipse_path(ex + gx * 0.7 - 1.4, ey + gy * 0.7 - 1.6, 1.75, 1.75)
                    {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi2) =
                        ellipse_path(ex + gx * 0.5 + 1.4, ey + gy * 0.5 + 1.5, 0.85, 0.85)
                    {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, body_tx, None);
                    }
                }
                EyeExpression::Blink => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 4.8, ey + 0.5);
                    pb.quad_to(ex, ey + 2.8, ex + 4.8, ey + 0.2);
                    if let Some(arc) = pb.finish() {
                        self.pixmap.stroke_path(
                            &arc,
                            &ink,
                            &self.stroke_outline(2.4),
                            body_tx,
                            None,
                        );
                    }
                }
                EyeExpression::Happy => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.0, ey + 1.8);
                    pb.quad_to(ex, ey - 4.4, ex + 5.0, ey + 1.8);
                    if let Some(arc) = pb.finish() {
                        self.pixmap.stroke_path(
                            &arc,
                            &ink,
                            &self.stroke_outline(2.6),
                            body_tx,
                            None,
                        );
                    }
                }
                EyeExpression::Drowsy => {
                    // Half-closed sleepy eye.
                    if let Some(eye) = ellipse_path(ex, ey + 1.2, 4.3, 2.8) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    // Heavy eyelid cover in body color + lid line.
                    let cover = Self::solid_paint(self.palette.body_base);
                    if let Some(lid_cover) = ellipse_path(ex, ey - 1.5, 5.4, 2.6) {
                        self.pixmap
                            .fill_path(&lid_cover, &cover, FillRule::Winding, body_tx, None);
                    }
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.2, ey - 0.6);
                    pb.line_to(ex + 5.2, ey - 0.2);
                    if let Some(lid) = pb.finish() {
                        self.pixmap.stroke_path(
                            &lid,
                            &ink,
                            &self.stroke_outline(2.1),
                            body_tx,
                            None,
                        );
                    }
                }
                EyeExpression::Surprised => {
                    // Wide bright startled bean eyes + tiny raised brow ticks.
                    if let Some(eye) = ellipse_path(ex, ey, 5.1, 5.4) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi1) = ellipse_path(ex - 1.5, ey - 1.8, 2.0, 2.0) {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi2) = ellipse_path(ex + 1.7, ey + 1.6, 1.0, 1.0) {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 3.2, ey - 8.2);
                    brow.quad_to(ex, ey - 10.2, ex + 3.2, ey - 8.2);
                    if let Some(b) = brow.finish() {
                        self.pixmap.stroke_path(
                            &b,
                            &ink,
                            &self.stroke_outline(1.7),
                            body_tx,
                            None,
                        );
                    }
                }
                EyeExpression::Focused => {
                    // Bright attentive eyes with crisp subtle focused brows.
                    if let Some(eye) = ellipse_path(ex + gx * 0.5, ey + gy * 0.5, 4.4, 4.4) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi1) =
                        ellipse_path(ex + gx * 0.7 - 1.3, ey + gy * 0.7 - 1.4, 1.8, 1.8)
                    {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let slant = if ex < 5.0 { 1.4 } else { -1.4 };
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 4.4, ey - 6.8 - slant * 0.5);
                    brow.line_to(ex + 4.4, ey - 6.8 + slant * 0.5);
                    if let Some(b) = brow.finish() {
                        self.pixmap.stroke_path(
                            &b,
                            &ink,
                            &self.stroke_outline(1.9),
                            body_tx,
                            None,
                        );
                    }
                }
                EyeExpression::Sleeping => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 4.8, ey - 0.6);
                    pb.quad_to(ex, ey + 3.8, ex + 4.8, ey - 0.6);
                    if let Some(arc) = pb.finish() {
                        self.pixmap.stroke_path(
                            &arc,
                            &ink,
                            &self.stroke_outline(2.4),
                            body_tx,
                            None,
                        );
                    }
                }
            }
        }
    }

    fn draw_beak(&mut self, body_tx: Transform, pose: &PetPose) {
        let bx = 10.5;
        let by = -4.2;
        let outline = Self::solid_paint(self.palette.outline);

        match pose.beak {
            BeakPose::Closed => {
                let mut pb = PathBuilder::new();
                pb.move_to(bx - 5.2, by - 0.5);
                pb.quad_to(bx + 0.5, by - 4.4, bx + 6.2, by - 0.4);
                pb.quad_to(bx + 0.5, by + 4.6, bx - 5.2, by - 0.5);
                pb.close();
                if let Some(path) = pb.finish() {
                    let fill = Self::solid_paint(self.palette.beak_base);
                    self.pixmap
                        .fill_path(&path, &fill, FillRule::Winding, body_tx, None);
                    self.pixmap.stroke_path(
                        &path,
                        &outline,
                        &self.stroke_outline(1.85),
                        body_tx,
                        None,
                    );
                }
                // Warm amber highlight dot on upper beak.
                if let Some(hi) = ellipse_path(bx + 0.2, by - 1.4, 2.3, 1.0) {
                    let p = Self::solid_paint(self.palette.beak_top.with_alpha(0.8));
                    self.pixmap
                        .fill_path(&hi, &p, FillRule::Winding, body_tx, None);
                }
            }
            BeakPose::Chirp | BeakPose::HappyOpen => {
                let open = if pose.beak == BeakPose::HappyOpen {
                    4.6
                } else {
                    2.8
                };
                // Open inner rosy mouth.
                if let Some(mouth) = ellipse_path(bx + 0.3, by + 1.0, 4.2, open * 0.72) {
                    let inner = Self::solid_paint(self.palette.beak_inner);
                    self.pixmap
                        .fill_path(&mouth, &inner, FillRule::Winding, body_tx, None);
                    self.pixmap.stroke_path(
                        &mouth,
                        &outline,
                        &self.stroke_outline(1.75),
                        body_tx,
                        None,
                    );
                }
                // Upper beak dome.
                let mut top = PathBuilder::new();
                top.move_to(bx - 5.5, by - 0.4);
                top.quad_to(bx + 0.5, by - 5.2, bx + 6.5, by - 0.4);
                top.quad_to(bx + 0.5, by + 1.4, bx - 5.5, by - 0.4);
                top.close();
                if let Some(path) = top.finish() {
                    let fill = Self::solid_paint(self.palette.beak_top);
                    self.pixmap
                        .fill_path(&path, &fill, FillRule::Winding, body_tx, None);
                    self.pixmap.stroke_path(
                        &path,
                        &outline,
                        &self.stroke_outline(1.85),
                        body_tx,
                        None,
                    );
                }
            }
        }
    }

    fn draw_ahoge(&mut self, body_tx: Transform, pose: &PetPose) {
        // Signature bouncy double-curl milk-foam tuft atop crown!
        let tx = body_tx
            .pre_translate(2.0, -32.8)
            .pre_rotate(pose.ahoge_angle.to_degrees());

        let mut pb = PathBuilder::new();
        // Main plump curling milk-foam drop + cute side sprout.
        pb.move_to(-4.5, 1.8);
        pb.cubic_to(-9.5, -5.5, -5.0, -16.5, 4.2, -16.0);
        pb.cubic_to(-1.2, -11.5, 1.5, -6.0, 2.5, -1.5);
        pb.cubic_to(5.5, -7.8, 12.2, -8.5, 12.0, -3.5);
        pb.cubic_to(9.0, -4.0, 6.2, -1.2, 5.2, 2.0);
        pb.close();

        if let Some(path) = pb.finish() {
            let fill = Self::solid_paint(self.palette.body_highlight);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.9),
                tx,
                None,
            );
            // Softly blend the root seam into the white head!
            if let Some(seam) = ellipse_path(0.5, 2.4, 5.5, 2.4) {
                let blend = Self::solid_paint(self.palette.body_highlight);
                self.pixmap
                    .fill_path(&seam, &blend, FillRule::Winding, tx, None);
            }
        }
    }

    fn draw_prop(
        &mut self,
        anchor_x: f32,
        body_y: f32,
        facing: f32,
        pose: &PetPose,
        scale: f32,
    ) {
        let base_tx = Transform::from_scale(scale, scale);
        match pose.prop {
            PropKind::None => {}
            PropKind::Hearts { phase, count } => {
                for i in 0..count.min(2) {
                    let p = (phase + i as f32 * 0.45).fract();
                    let fade = (1.0 - p).clamp(0.0, 1.0);
                    let hx = anchor_x + facing * (28.0 + i as f32 * 13.0) + (p * PI * 2.0).sin() * 3.0;
                    let hy = body_y - 24.0 - p * 22.0 - i as f32 * 7.0;
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
                    let zx = anchor_x + facing * (26.0 + p * 14.0 + i as f32 * 6.0);
                    let zy = body_y - 16.0 - p * 26.0 - i as f32 * 6.0;
                    let s = 5.0 + i as f32 * 2.4 + p * 1.5;
                    let mut pb = PathBuilder::new();
                    pb.move_to(zx - s * 0.5, zy - s * 0.5);
                    pb.line_to(zx + s * 0.5, zy - s * 0.5);
                    pb.line_to(zx - s * 0.5, zy + s * 0.5);
                    pb.line_to(zx + s * 0.5, zy + s * 0.5);
                    if let Some(z_path) = pb.finish() {
                        let paint = Self::solid_paint(self.palette.sleep_zzz.with_alpha(fade * 0.95));
                        self.pixmap.stroke_path(
                            &z_path,
                            &paint,
                            &self.stroke_outline(2.0),
                            base_tx,
                            None,
                        );
                    }
                }
            }
            PropKind::UnreadBadge { count, pulse } => {
                let bx = anchor_x + facing * 34.0;
                let by = body_y - 31.0 - pulse * 2.5;
                let halo_r = 12.0 + pulse * 4.5;
                if let Some(halo) = ellipse_path(bx, by, halo_r, halo_r) {
                    let halo_paint =
                        Self::solid_paint(self.palette.badge_alert.with_alpha((1.0 - pulse) * 0.35));
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
                // Crisp vector mini-envelope inside badge (plus second dot if count > 1).
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
                let coords = [(-35.0, -28.0, 7.0), (38.0, -34.0, 8.2), (42.0, -8.0, 5.5)];
                for (i, &(ox, oy, r)) in coords.iter().enumerate() {
                    let p = (phase + i as f32 * 0.33).fract();
                    let twinkle = (p * PI).sin();
                    let sr = r * (0.45 + 0.55 * twinkle);
                    if let Some(star) = star4_path(anchor_x + ox * facing, body_y + oy, sr) {
                        let fill =
                            Self::solid_paint(self.palette.sparkle.with_alpha(0.35 + 0.65 * twinkle));
                        self.pixmap
                            .fill_path(&star, &fill, FillRule::Winding, base_tx, None);
                    }
                }
            }
            PropKind::FocusRipple { phase } => {
                let cx = anchor_x + facing * 14.0;
                let cy = body_y - 44.0;
                for i in 0..2 {
                    let p = (phase + i as f32 * 0.5).fract();
                    let r = 6.0 + p * 12.0;
                    let fade = (1.0 - p) * 0.55;
                    let mut pb = PathBuilder::new();
                    pb.move_to(cx - r * 0.6, cy - r * 0.45);
                    pb.quad_to(cx, cy - r * 0.95, cx + r * 0.6, cy - r * 0.45);
                    if let Some(arc) = pb.finish() {
                        let paint = Self::solid_paint(self.palette.scarf_highlight.with_alpha(fade));
                        self.pixmap.stroke_path(
                            &arc,
                            &paint,
                            &self.stroke_outline(1.7),
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

/// Soft sculpted teardrop bird wing pivoted near `(0, 0)`. `dir = -1` extends back-down, `+1` extends forward-down.
fn wing_path(dir: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, -3.5);
    pb.cubic_to(dir * 9.0, -5.5, dir * 18.5, 1.5, dir * 16.0, 13.5);
    pb.cubic_to(dir * 13.0, 20.5, dir * 3.5, 17.5, -dir * 1.5, 9.0);
    pb.cubic_to(-dir * 4.0, 3.2, -dir * 2.8, -2.0, 0.0, -3.5);
    pb.close();
    pb.finish()
}

/// Flowing scarf ribbon tail extending leftward from `(0, 0)`.
fn scarf_tail_path(len: f32, half_w: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, -half_w * 0.52);
    pb.quad_to(-len * 0.5, -half_w * 0.72, -len, -half_w * 0.95);
    pb.quad_to(-len * 1.08, 0.0, -len, half_w * 0.95);
    pb.quad_to(-len * 0.5, half_w * 0.72, 0.0, half_w * 0.52);
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

/// Compose a multi-pose studio showcase contact sheet (4 columns x 2 rows) on split light/dark tiles
/// so silhouette legibility can be verified visually on both bright and dark macOS desktops.
pub fn render_showcase_sheet(poses: &[(&str, PetPose)], scale: f32) -> Pixmap {
    let cols = 4u32;
    let rows = ((poses.len() as u32).saturating_add(cols - 1) / cols).max(1);
    let cell = (LOGICAL_CANVAS_SIZE * scale).round() as u32;
    let pad = (16.0 * scale).round() as u32;
    let width = cols * cell + (cols + 1) * pad;
    let height = rows * cell + (rows + 1) * pad;

    let mut sheet = Pixmap::new(width, height).expect("valid showcase sheet size");
    // Neutral studio backdrop (#E5E7EB).
    sheet.fill(Color::from_rgba8(229, 231, 235, 255));

    let mut renderer = PetRenderer::new(scale);
    let light_bg = Color::from_rgba8(255, 255, 255, 255);
    let dark_bg = Color::from_rgba8(30, 41, 59, 255);

    for (idx, (_, pose)) in poses.iter().enumerate() {
        let col = (idx as u32) % cols;
        let row = (idx as u32) / cols;
        let ox = pad + col * (cell + pad);
        let oy = pad + row * (cell + pad);

        // Paint split light (left) / dark (right) tile behind each bird to prove contrast on both!
        let half = cell / 2;
        if let Some(rect) = tiny_skia::Rect::from_xywh(ox as f32, oy as f32, half as f32, cell as f32)
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
