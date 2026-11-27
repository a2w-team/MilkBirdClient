//! Zero-jitter pre-allocated `tiny-skia` vector renderer and O(1) alpha HitMask
//! for the original "奶鸟 (MilkBird)" character (coral-red plumage + pure milk-cream belly,
//! mango-gold twin flame-foam crest, peach-tipped wings, starry espresso eyes, zero logos).

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
        Self::with_config(PetRig::default(), Palette::milkbird_original(), scale)
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

    /// Render the original 11-layer MilkBird character into the pre-allocated `Pixmap`.
    pub fn render(&mut self, pose: &PetPose) -> &Pixmap {
        self.pixmap.data_mut().fill(0);

        let scale = self.scale;
        let anchor_x = self.rig.anchor_x + pose.offset_x;
        let ground_y = self.rig.ground_y;
        let facing = if pose.facing >= 0.0 { 1.0 } else { -1.0 };

        // 1. Soft ground shadow.
        self.draw_ground_shadow(anchor_x, ground_y, pose, scale);

        let breath_scale_x = 1.0 + 0.018 * pose.breath;
        let breath_scale_y = 1.0 + 0.022 * pose.breath;
        let sx = pose.scale_x * breath_scale_x * facing;
        let sy = pose.scale_y * breath_scale_y;

        let rest_height = 40.5 - 8.0 * pose.sleep_curl;
        let body_y = ground_y - pose.altitude - rest_height * pose.scale_y;

        let body_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(sx, sy);

        let body_rigid_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(facing, 1.0);

        // 2. Forked coral + mango-gold tail feathers behind body.
        self.draw_tail_feathers(body_tx, pose);

        // 3. Back wing peeking cleanly on right contour.
        self.draw_back_wing(body_tx, pose);

        // 4. Warm honey-apricot little bird feet.
        self.draw_feet(body_rigid_tx, anchor_x, ground_y, facing, pose, scale);

        // 5. Plump pear-round coral-red body + warm contour.
        self.draw_body_blob(body_tx);

        // 6. Pure creamy milk-foam cloud-tufted belly (completely clean, zero logos!).
        self.draw_milk_belly(body_tx);

        // 7. Sleek two-tone coral-to-peach-cream front wing.
        self.draw_front_wing(body_tx, pose);

        // 8. Soft strawberry-milk cheek blushes.
        self.draw_cheeks(body_tx, pose);

        // 9. Expressive starry espresso-jewel eyes (7 expressions + gaze tracking).
        self.draw_eyes(body_tx, pose);

        // 10. Cute rounded honey-apricot beak.
        self.draw_beak(body_tx, pose);

        // 11. Original twin-plume "flame + milk-foam" bouncy crown sprout!
        self.draw_crown_plume(body_tx, pose);

        // 12. Floating micro-prop overlay (hearts, sleep Zzz, unread badge, sparkles, watch ripple).
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
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(rgba.to_skia());
        paint
    }

    fn draw_ground_shadow(&mut self, anchor_x: f32, ground_y: f32, pose: &PetPose, scale: f32) {
        let alt_factor = (1.0 - pose.altitude / 90.0).clamp(0.35, 1.0);
        let rx = (32.5 + 4.0 * pose.sleep_curl) * pose.scale_x * alt_factor;
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

    fn draw_tail_feathers(&mut self, body_tx: Transform, pose: &PetPose) {
        let pivot_x = -33.0;
        let pivot_y = 14.5;
        let wag_deg = pose.tail_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.45;

        // Lower coral tail feather.
        let lower_tx = body_tx
            .pre_translate(pivot_x, pivot_y + 2.0)
            .pre_rotate(18.0 + wag_deg * 0.85);
        if let Some(path) = feather_path(21.0, 6.4) {
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

        // Upper gradient coral-to-mango-gold tail feather!
        let upper_tx = body_tx
            .pre_translate(pivot_x + 1.5, pivot_y - 3.5)
            .pre_rotate(-14.0 + wag_deg);
        if let Some(path) = feather_path(24.5, 7.2) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(-24.0, 0.0),
                vec![
                    GradientStop::new(0.0, self.palette.tail_base.to_skia()),
                    GradientStop::new(0.65, self.palette.crest_base.to_skia()),
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

    fn draw_back_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        let lift_deg = -12.0 - pose.wing_flap.to_degrees() * 1.05 - pose.wing_spread * 24.0;
        let tx = body_tx
            .pre_translate(30.5 + pose.wing_spread * 2.5, 4.5 - pose.wing_spread * 2.5)
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

    fn draw_front_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        // Sleek two-tone wing resting cozily along the left flank, sweeping up happily on flap!
        let flap_deg = -2.0 + pose.wing_flap.to_degrees() * 1.15 + pose.wing_spread * 28.0;
        let tx = body_tx
            .pre_translate(-22.5 - pose.wing_spread * 2.5, 5.5 - pose.wing_spread * 2.0)
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

            // Subtle soft feather-tip highlight streak for airy illustration charm.
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
                self.draw_single_foot(body_rigid_tx, -10.5, 36.5, -22.0 + kick * 28.0);
                self.draw_single_foot(body_rigid_tx, 11.5, 36.5, 22.0 - kick * 28.0);
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

    fn draw_body_blob(&mut self, body_tx: Transform) {
        // Plump chubby pear-round designer little bird silhouette.
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, -34.5);
        pb.cubic_to(23.5, -34.5, 39.0, -17.0, 38.2, 4.2);
        pb.cubic_to(37.5, 23.0, 23.8, 35.0, 0.0, 35.0);
        pb.cubic_to(-23.8, 35.0, -38.5, 23.0, -38.8, 4.2);
        pb.cubic_to(-39.0, -17.0, -23.5, -34.5, 0.0, -34.5);
        pb.close();

        let Some(body_path) = pb.finish() else {
            return;
        };

        // Lively coral-red radial gradient with warm sunny top-right light.
        let mut grad_paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(8.0, -14.0),
            Point::from_xy(4.0, -4.0),
            53.0,
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
        if let Some(crown) = ellipse_path(6.0, -24.0, 14.5, 5.4) {
            let sheen = Self::solid_paint(Rgba::new(255, 178, 164, 72));
            self.pixmap
                .fill_path(&crown, &sheen, FillRule::Winding, body_tx, None);
        }

        // Crisp berry-cocoa contour so she reads cleanly on both light & dark desktops.
        let outline = Self::solid_paint(self.palette.outline);
        self.pixmap.stroke_path(
            &body_path,
            &outline,
            &self.stroke_outline(self.rig.outline_width),
            body_tx,
            None,
        );
    }

    /// Pure creamy milk-foam tummy patch with a playful 3-puff cloud-feather upper arch — ZERO logos!
    fn draw_milk_belly(&mut self, body_tx: Transform) {
        let bx = 7.0;
        let by = 15.8;

        let mut pb = PathBuilder::new();
        // Cute gentle scalloped 3-puff milk-foam top arch (original signature chest fluff!)
        pb.move_to(bx - 18.5, by - 5.5);
        pb.cubic_to(bx - 17.0, by - 13.5, bx - 9.5, by - 15.8, bx - 5.0, by - 12.2);
        pb.cubic_to(bx - 2.2, by - 17.5, bx + 4.8, by - 17.5, bx + 7.2, by - 12.0);
        pb.cubic_to(bx + 11.8, by - 15.2, bx + 18.5, by - 12.0, bx + 19.6, by - 4.2);
        // Smooth plump lower milk tummy curving neatly inside the red body contour
        pb.cubic_to(bx + 21.0, by + 8.5, bx + 12.2, by + 17.6, bx, by + 17.8);
        pb.cubic_to(bx - 12.8, by + 17.8, bx - 20.5, by + 8.8, bx - 18.5, by - 5.5);
        pb.close();

        if let Some(belly) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(bx + 2.0, by - 5.0),
                Point::from_xy(bx, by + 1.0),
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
                .fill_path(&belly, &paint, FillRule::Winding, body_tx, None);

            // Soft milk-foam shine dot on upper tummy for silky depth.
            if let Some(glow) = ellipse_path(bx + 2.5, by - 2.0, 9.5, 6.2) {
                let p = Self::solid_paint(self.palette.belly_highlight.with_alpha(0.65));
                self.pixmap
                    .fill_path(&glow, &p, FillRule::Winding, body_tx, None);
            }
        }
    }

    fn draw_cheeks(&mut self, body_tx: Transform, pose: &PetPose) {
        let alpha = pose.blush.clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let paint = Self::solid_paint(self.palette.cheek.with_alpha(alpha * 0.68));
        for &(cx, cy, rx, ry) in &[(-10.5, -2.5, 6.2, 3.6), (26.5, -2.5, 5.5, 3.4)] {
            if let Some(cheek) = ellipse_path(cx, cy, rx, ry) {
                self.pixmap
                    .fill_path(&cheek, &paint, FillRule::Winding, body_tx, None);
            }
        }
    }

    /// Original starry espresso-jewel eyes with subtle warm amber lower crescent & twin white sparkles.
    fn draw_eyes(&mut self, body_tx: Transform, pose: &PetPose) {
        let eye_positions = [(-3.0f32, -11.0f32), (20.5f32, -11.0f32)];
        let ink = Self::solid_paint(self.palette.eye_ink);
        let sheen = Self::solid_paint(self.palette.eye_sheen.with_alpha(0.78));
        let sparkle = Self::solid_paint(self.palette.eye_sparkle);
        let rim = Self::solid_paint(self.palette.belly_highlight.with_alpha(0.88));

        let gx = pose.gaze_x.clamp(-1.0, 1.0) * 1.5;
        let gy = pose.gaze_y.clamp(-1.0, 1.0) * 1.2;

        for &(ex, ey) in &eye_positions {
            match pose.eyes {
                EyeExpression::Normal => {
                    // Subtle creamy-white soft rim so dark starry eyes pop radiantly against coral-red!
                    if let Some(halo) = ellipse_path(ex + gx * 0.4, ey + gy * 0.4, 6.2, 6.7) {
                        self.pixmap
                            .fill_path(&halo, &rim, FillRule::Winding, body_tx, None);
                    }
                    // Glossy deep espresso-plum eye
                    if let Some(eye) = ellipse_path(ex + gx * 0.5, ey + gy * 0.5, 5.2, 5.7) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    // Warm jewel-amber lower crescent highlight inside iris
                    if let Some(cres) =
                        ellipse_path(ex + gx * 0.5 + 0.4, ey + gy * 0.5 + 2.6, 2.7, 1.5)
                    {
                        self.pixmap
                            .fill_path(&cres, &sheen, FillRule::Winding, body_tx, None);
                    }
                    // Crisp twin catchlights
                    if let Some(hi1) =
                        ellipse_path(ex + gx * 0.65 - 1.6, ey + gy * 0.65 - 1.9, 1.95, 1.95)
                    {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi2) =
                        ellipse_path(ex + gx * 0.45 + 1.8, ey + gy * 0.45 + 1.5, 0.9, 0.9)
                    {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, body_tx, None);
                    }
                }
                EyeExpression::Blink => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.2, ey + 0.5);
                    pb.quad_to(ex, ey + 3.2, ex + 5.2, ey + 0.3);
                    if let Some(arc) = pb.finish() {
                        self.pixmap
                            .stroke_path(&arc, &rim, &self.stroke_outline(4.2), body_tx, None);
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.4), body_tx, None);
                    }
                }
                EyeExpression::Happy => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.4, ey + 2.0);
                    pb.quad_to(ex, ey - 4.8, ex + 5.4, ey + 2.0);
                    if let Some(arc) = pb.finish() {
                        self.pixmap
                            .stroke_path(&arc, &rim, &self.stroke_outline(4.6), body_tx, None);
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.6), body_tx, None);
                    }
                }
                EyeExpression::Drowsy => {
                    if let Some(halo) = ellipse_path(ex, ey + 1.0, 5.8, 4.2) {
                        self.pixmap
                            .fill_path(&halo, &rim, FillRule::Winding, body_tx, None);
                    }
                    if let Some(eye) = ellipse_path(ex, ey + 1.2, 4.7, 3.3) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    let cover = Self::solid_paint(self.palette.body_base);
                    if let Some(lid_cover) = ellipse_path(ex, ey - 2.0, 6.8, 3.0) {
                        self.pixmap
                            .fill_path(&lid_cover, &cover, FillRule::Winding, body_tx, None);
                    }
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.8, ey - 0.6);
                    pb.line_to(ex + 5.8, ey - 0.2);
                    if let Some(lid) = pb.finish() {
                        self.pixmap
                            .stroke_path(&lid, &ink, &self.stroke_outline(2.2), body_tx, None);
                    }
                }
                EyeExpression::Surprised => {
                    if let Some(halo) = ellipse_path(ex, ey, 6.9, 7.4) {
                        self.pixmap
                            .fill_path(&halo, &rim, FillRule::Winding, body_tx, None);
                    }
                    if let Some(eye) = ellipse_path(ex, ey, 5.5, 6.0) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    if let Some(cres) = ellipse_path(ex + 0.4, ey + 2.8, 2.8, 1.4) {
                        self.pixmap
                            .fill_path(&cres, &sheen, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi1) = ellipse_path(ex - 1.7, ey - 2.1, 2.2, 2.2) {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi2) = ellipse_path(ex + 1.9, ey + 1.8, 0.95, 0.95) {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 3.6, ey - 9.4);
                    brow.quad_to(ex, ey - 11.4, ex + 3.6, ey - 9.4);
                    if let Some(b) = brow.finish() {
                        self.pixmap
                            .stroke_path(&b, &ink, &self.stroke_outline(1.8), body_tx, None);
                    }
                }
                EyeExpression::Focused => {
                    if let Some(halo) = ellipse_path(ex + gx * 0.4, ey + gy * 0.4, 6.0, 6.2) {
                        self.pixmap
                            .fill_path(&halo, &rim, FillRule::Winding, body_tx, None);
                    }
                    if let Some(eye) = ellipse_path(ex + gx * 0.5, ey + gy * 0.5, 5.0, 5.2) {
                        self.pixmap
                            .fill_path(&eye, &ink, FillRule::Winding, body_tx, None);
                    }
                    if let Some(cres) =
                        ellipse_path(ex + gx * 0.5 + 0.3, ey + gy * 0.5 + 2.3, 2.5, 1.3)
                    {
                        self.pixmap
                            .fill_path(&cres, &sheen, FillRule::Winding, body_tx, None);
                    }
                    if let Some(hi1) =
                        ellipse_path(ex + gx * 0.7 - 1.5, ey + gy * 0.7 - 1.7, 1.9, 1.9)
                    {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, body_tx, None);
                    }
                    let slant = if ex < 5.0 { 1.8 } else { -1.8 };
                    let mut brow = PathBuilder::new();
                    brow.move_to(ex - 5.2, ey - 7.8 - slant * 0.5);
                    brow.line_to(ex + 5.2, ey - 7.8 + slant * 0.5);
                    if let Some(b) = brow.finish() {
                        self.pixmap
                            .stroke_path(&b, &ink, &self.stroke_outline(2.1), body_tx, None);
                    }
                }
                EyeExpression::Sleeping => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(ex - 5.2, ey - 0.5);
                    pb.quad_to(ex, ey + 4.0, ex + 5.2, ey - 0.5);
                    if let Some(arc) = pb.finish() {
                        self.pixmap
                            .stroke_path(&arc, &rim, &self.stroke_outline(4.2), body_tx, None);
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.4), body_tx, None);
                    }
                }
            }
        }
    }

    /// Cute rounded honey-apricot little beak.
    fn draw_beak(&mut self, body_tx: Transform, pose: &PetPose) {
        let bx = 9.0;
        let by = -4.2;
        let outline = Self::solid_paint(self.palette.outline);

        match pose.beak {
            BeakPose::Closed => {
                let mut pb = PathBuilder::new();
                pb.move_to(bx - 5.4, by - 0.5);
                pb.quad_to(bx + 0.4, by - 4.6, bx + 6.2, by - 0.4);
                pb.quad_to(bx + 0.4, by + 4.6, bx - 5.4, by - 0.5);
                pb.close();
                if let Some(path) = pb.finish() {
                    let fill = Self::solid_paint(self.palette.beak_base);
                    self.pixmap
                        .fill_path(&path, &fill, FillRule::Winding, body_tx, None);
                    self.pixmap
                        .stroke_path(&path, &outline, &self.stroke_outline(1.85), body_tx, None);
                }
                if let Some(hi) = ellipse_path(bx + 0.1, by - 1.4, 2.4, 1.05) {
                    let p = Self::solid_paint(self.palette.beak_top.with_alpha(0.85));
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
                if let Some(mouth) = ellipse_path(bx + 0.2, by + 1.2, 4.3, open * 0.75) {
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
                let mut top = PathBuilder::new();
                top.move_to(bx - 5.6, by - 0.4);
                top.quad_to(bx + 0.4, by - 5.4, bx + 6.4, by - 0.4);
                top.quad_to(bx + 0.4, by + 1.5, bx - 5.6, by - 0.4);
                top.close();
                if let Some(path) = top.finish() {
                    let fill = Self::solid_paint(self.palette.beak_top);
                    self.pixmap
                        .fill_path(&path, &fill, FillRule::Winding, body_tx, None);
                    self.pixmap
                        .stroke_path(&path, &outline, &self.stroke_outline(1.85), body_tx, None);
                }
            }
        }
    }

    /// Original bouncy twin-plume "flame + milk-foam" crown sprout!
    /// Loosely evokes fiery bird energy with a soft-curved mango-gold plume + creamy side curl.
    fn draw_crown_plume(&mut self, body_tx: Transform, pose: &PetPose) {
        let sway_deg = pose.ahoge_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.35;
        let tx = body_tx.pre_translate(1.5, -33.2).pre_rotate(sway_deg);
        let outline = Self::solid_paint(self.palette.outline);

        // Secondary cheeky creamy-milk side curl (tucked slightly behind/right).
        let mut side = PathBuilder::new();
        side.move_to(1.5, 1.5);
        side.cubic_to(3.8, -6.5, 11.2, -10.2, 13.4, -4.5);
        side.cubic_to(10.0, -4.8, 7.6, -1.8, 6.2, 2.2);
        side.close();
        if let Some(path) = side.finish() {
            let fill = Self::solid_paint(self.palette.crest_cream);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.85),
                tx,
                None,
            );
        }

        // Main sweeping coral-to-mango-gold flame plume!
        let mut main = PathBuilder::new();
        main.move_to(-5.2, 2.2);
        main.cubic_to(-11.2, -6.0, -7.2, -18.5, 3.8, -20.5);
        main.cubic_to(-1.2, -14.2, 2.4, -7.2, 4.6, 1.8);
        main.quad_to(-0.2, 3.8, -5.2, 2.2);
        main.close();
        if let Some(path) = main.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            let tip = if pose.badge_glow > 0.1 {
                self.palette.sparkle
            } else {
                self.palette.crest_tip
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(3.0, -20.0),
                Point::from_xy(-2.0, 3.0),
                vec![
                    GradientStop::new(0.0, tip.to_skia()),
                    GradientStop::new(0.58, self.palette.crest_base.to_skia()),
                    GradientStop::new(1.0, self.palette.body_base.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.crest_base.to_skia());
            }
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.88),
                tx,
                None,
            );
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
                    let hx =
                        anchor_x + facing * (30.0 + i as f32 * 13.0) + (p * PI * 2.0).sin() * 3.0;
                    let hy = body_y - 25.0 - p * 22.0 - i as f32 * 7.0;
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
                    let zy = body_y - 17.0 - p * 26.0 - i as f32 * 6.0;
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
                let bx = anchor_x + facing * 35.0;
                let by = body_y - 31.0 - pulse * 2.5;
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
                let cy = body_y - 45.0;
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
    // Cute gentle twin-feather scalloped tip!
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
    sheet.fill(Color::from_rgba8(229, 231, 235, 255));

    let mut renderer = PetRenderer::new(scale);
    let light_bg = Color::from_rgba8(255, 255, 255, 255);
    let dark_bg = Color::from_rgba8(30, 41, 59, 255);

    for (idx, (_, pose)) in poses.iter().enumerate() {
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
