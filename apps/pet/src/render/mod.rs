//! Zero-jitter pre-allocated `tiny-skia` vector renderer and O(1) alpha HitMask
//! for the HKUST-inspired "小红鸟 / 奶鸟 (MilkBird)" plush character.

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
        Self::with_config(PetRig::default(), Palette::milkbird_red(), scale)
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

    /// Render the complete 11-layer Red Bird ("小红鸟") character into the pre-allocated `Pixmap`.
    pub fn render(&mut self, pose: &PetPose) -> &Pixmap {
        self.pixmap.data_mut().fill(0);

        let scale = self.scale;
        let anchor_x = self.rig.anchor_x + pose.offset_x;
        let ground_y = self.rig.ground_y;
        let facing = if pose.facing >= 0.0 { 1.0 } else { -1.0 };

        // 1. Ground shadow (stays on ground, shrinks softly as bird hops/dangles).
        self.draw_ground_shadow(anchor_x, ground_y, pose, scale);

        let breath_scale_x = 1.0 + 0.018 * pose.breath;
        let breath_scale_y = 1.0 + 0.022 * pose.breath;
        let sx = pose.scale_x * breath_scale_x * facing;
        let sy = pose.scale_y * breath_scale_y;

        // Plump chubby plush center sits slightly lower when curled up sleeping.
        let rest_height = 41.0 - 8.0 * pose.sleep_curl;
        let body_y = ground_y - pose.altitude - rest_height * pose.scale_y;

        let body_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(sx, sy);

        let body_rigid_tx = Transform::from_scale(scale, scale)
            .pre_translate(anchor_x, body_y)
            .pre_rotate(pose.tilt_rad.to_degrees())
            .pre_scale(facing, 1.0);

        // 2. Small flame-tipped tail feathers peeking behind the back contour.
        self.draw_tail_feathers(body_tx, pose);

        // 3. Back plush 3-lobed wing (right/front side in 3/4 view).
        self.draw_right_wing(body_tx, pose);

        // 4. Plump caramel-tan plush feet.
        self.draw_feet(body_rigid_tx, anchor_x, ground_y, facing, pose, scale);

        // 5. Vivid crimson chubby plush body + fluffy head sheen + warm burgundy contour.
        self.draw_body_blob(body_tx);

        // 6. Iconic snowy-white round tummy patch + minimalist gold/ink club sundial emblem.
        self.draw_white_belly_and_emblem(body_tx, pose);

        // 7. Front plush 3-lobed wing (left side, hugging/waving playfully).
        self.draw_left_wing(body_tx, pose);

        // 8. Soft rosy cheek glow under the big eyes.
        self.draw_cheeks(body_tx, pose);

        // 9. Iconic big plush eyes (white outer ring + sky-blue iris + glossy black pupil + catchlight).
        self.draw_eyes(body_tx, pose);

        // 10. Plump rounded berry-red plush beak right above the white tummy arch.
        self.draw_beak(body_tx, pose);

        // 11. Iconic HKUST Red Bird sundial-inspired flame-orange multi-spike crest!
        self.draw_flame_crest(body_tx, pose);

        // 12. Floating micro-prop overlay (hearts, sleep Zzz, unread pill, sparkles, focus ripple).
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
        let rx = (34.0 + 4.0 * pose.sleep_curl) * pose.scale_x * alt_factor;
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
        let pivot_x = -34.0;
        let pivot_y = 14.0;
        let wag_deg = pose.tail_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.45;

        // Lower crimson tail feather.
        let lower_tx = body_tx
            .pre_translate(pivot_x, pivot_y + 2.0)
            .pre_rotate(16.0 + wag_deg * 0.85);
        if let Some(path) = feather_path(16.0, 5.8) {
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

        // Upper flame-tipped feather.
        let upper_tx = body_tx
            .pre_translate(pivot_x + 1.0, pivot_y - 3.0)
            .pre_rotate(-14.0 + wag_deg);
        if let Some(path) = feather_path(18.5, 6.2) {
            let fill = Self::solid_paint(self.palette.tail_accent);
            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .fill_path(&path, &fill, FillRule::Winding, upper_tx, None);
            self.pixmap.stroke_path(
                &path,
                &outline,
                &self.stroke_outline(self.rig.outline_width * 0.88),
                upper_tx,
                None,
            );
        }
    }

    /// Right-side 3-lobed plush wing stretching out joyfully like the photo plushie.
    fn draw_right_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        let flap_deg = -6.0 - pose.wing_flap.to_degrees() * 1.1 - pose.wing_spread * 24.0;
        let tx = body_tx
            .pre_translate(31.5 + pose.wing_spread * 2.5, 4.0 - pose.wing_spread * 2.5)
            .pre_rotate(flap_deg);
        self.draw_plush_three_lobed_wing(tx, 1.0, false);
    }

    /// Left-side 3-lobed plush wing with scalloped berry-red inner pads.
    fn draw_left_wing(&mut self, body_tx: Transform, pose: &PetPose) {
        let flap_deg = 8.0 + pose.wing_flap.to_degrees() * 1.15 + pose.wing_spread * 26.0;
        let tx = body_tx
            .pre_translate(-28.5 - pose.wing_spread * 2.5, 5.5 - pose.wing_spread * 2.5)
            .pre_rotate(flap_deg);
        self.draw_plush_three_lobed_wing(tx, -1.0, true);
    }

    fn draw_plush_three_lobed_wing(&mut self, tx: Transform, dir: f32, front: bool) {
        // Fluffy outer crimson wing silhouette with 3 cute rounded scalloped feather lobes!
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, -9.0);
        // Top lobe
        pb.cubic_to(dir * 8.5, -13.0, dir * 18.0, -10.5, dir * 18.5, -4.5);
        pb.quad_to(dir * 16.5, -1.5, dir * 13.5, -1.5);
        // Middle lobe (longest)
        pb.cubic_to(dir * 20.5, -1.0, dir * 21.8, 5.8, dir * 15.8, 7.8);
        pb.quad_to(dir * 13.0, 8.2, dir * 11.5, 7.5);
        // Bottom lobe
        pb.cubic_to(dir * 16.5, 11.0, dir * 12.2, 16.8, dir * 5.2, 14.5);
        pb.cubic_to(dir * 0.5, 13.0, -dir * 3.5, 8.0, -dir * 3.0, 1.0);
        pb.cubic_to(-dir * 3.0, -4.5, -dir * 1.5, -7.8, 0.0, -9.0);
        pb.close();

        if let Some(wing) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            let base = if front {
                self.palette.wing_base
            } else {
                self.palette.wing_lobe
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, -11.0),
                Point::from_xy(dir * 16.0, 15.0),
                vec![
                    GradientStop::new(0.0, self.palette.body_highlight.to_skia()),
                    GradientStop::new(0.55, base.to_skia()),
                    GradientStop::new(1.0, self.palette.wing_shade.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(base.to_skia());
            }
            self.pixmap
                .fill_path(&wing, &paint, FillRule::Winding, tx, None);

            // Inner velvety berry-crimson 3-lobe plush paw detail (just like the reference plushie!).
            let pad_paint = Self::solid_paint(self.palette.wing_lobe.with_alpha(0.85));
            for &(px, py, rx, ry) in &[
                (dir * 11.5, -4.8, 4.4, 2.9),
                (dir * 13.2, 2.8, 4.8, 3.1),
                (dir * 8.2, 9.8, 4.2, 2.7),
            ] {
                if let Some(pad) = ellipse_path(px, py, rx, ry) {
                    self.pixmap
                        .fill_path(&pad, &pad_paint, FillRule::Winding, tx, None);
                }
            }

            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap.stroke_path(
                &wing,
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
            self.draw_plush_foot(tx, -12.0, 0.0, 0.0);
            self.draw_plush_foot(tx, 13.0, 0.0, 0.0);
            return;
        }

        match pose.foot_pose {
            FootPose::Planted | FootPose::Tucked => {
                let tx = Transform::from_scale(scale, scale)
                    .pre_translate(anchor_x, ground_y - 4.5 - pose.altitude)
                    .pre_scale(facing, 1.0);
                self.draw_plush_foot(tx, -12.5, 0.0, 0.0);
                self.draw_plush_foot(tx, 13.5, 0.0, 0.0);
            }
            FootPose::Walking => {
                let step = pose.foot_phase.sin();
                let lift_l = step.max(0.0) * 6.5;
                let lift_r = (-step).max(0.0) * 6.5;
                let stride_l = step * 6.5;
                let stride_r = -step * 6.5;
                let tx = Transform::from_scale(scale, scale)
                    .pre_translate(anchor_x, ground_y - 4.5 - pose.altitude)
                    .pre_scale(facing, 1.0);
                self.draw_plush_foot(tx, -12.5 + stride_l, -lift_l, step * 18.0);
                self.draw_plush_foot(tx, 13.5 + stride_r, -lift_r, -step * 18.0);
            }
            FootPose::Dangling => {
                let kick = (pose.foot_phase * 1.4).sin();
                self.draw_plush_foot(body_rigid_tx, -12.5, 36.5, -20.0 + kick * 26.0);
                self.draw_plush_foot(body_rigid_tx, 13.5, 36.5, 20.0 - kick * 26.0);
            }
        }
    }

    /// Chubby caramel-tan 3-toe plushie foot (like the reference toy!).
    fn draw_plush_foot(&mut self, base_tx: Transform, x: f32, y: f32, rot_deg: f32) {
        let tx = base_tx.pre_translate(x, y).pre_rotate(rot_deg);
        let mut pb = PathBuilder::new();
        pb.move_to(-8.5, -3.5);
        pb.line_to(8.5, -3.5);
        pb.cubic_to(11.0, -1.0, 10.5, 5.0, 6.2, 5.8);
        pb.quad_to(3.8, 6.4, 2.8, 4.4);
        pb.quad_to(0.0, 6.8, -2.8, 4.4);
        pb.quad_to(-3.8, 6.4, -6.2, 5.8);
        pb.cubic_to(-10.5, 5.0, -11.0, -1.0, -8.5, -3.5);
        pb.close();

        if let Some(path) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, -4.0),
                Point::from_xy(0.0, 6.5),
                vec![
                    GradientStop::new(0.0, self.palette.foot_base.to_skia()),
                    GradientStop::new(1.0, self.palette.foot_shade.to_skia()),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                paint.set_color(self.palette.foot_base.to_skia());
            }
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, tx, None);

            // Subtle toe crease lines for plush stuffed look.
            let crease = Self::solid_paint(self.palette.outline.with_alpha(0.65));
            let mut cr = PathBuilder::new();
            cr.move_to(-2.8, 1.0);
            cr.line_to(-2.8, 4.5);
            cr.move_to(2.8, 1.0);
            cr.line_to(2.8, 4.5);
            if let Some(cp) = cr.finish() {
                self.pixmap
                    .stroke_path(&cp, &crease, &self.stroke_outline(1.3), tx, None);
            }

            let outline = Self::solid_paint(self.palette.outline);
            self.pixmap
                .stroke_path(&path, &outline, &self.stroke_outline(1.9), tx, None);
        }
    }

    fn draw_body_blob(&mut self, body_tx: Transform) {
        // Super round, fluffy chubby red-bird ball silhouette!
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, -35.5);
        pb.cubic_to(25.0, -35.5, 40.5, -18.0, 39.8, 3.5);
        pb.cubic_to(39.2, 23.0, 24.5, 35.5, 0.0, 35.5);
        pb.cubic_to(-24.5, 35.5, -39.8, 23.0, -40.2, 3.5);
        pb.cubic_to(-40.5, -18.0, -25.0, -35.5, 0.0, -35.5);
        pb.close();

        let Some(body_path) = pb.finish() else {
            return;
        };

        // Vivid plush radial crimson gradient.
        let mut grad_paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(6.0, -14.0),
            Point::from_xy(2.0, -4.0),
            54.0,
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

        // Soft plush velvet highlight on upper forehead.
        if let Some(crown) = ellipse_path(4.0, -24.5, 16.0, 6.2) {
            let sheen = Self::solid_paint(Rgba::new(255, 128, 138, 66));
            self.pixmap
                .fill_path(&crown, &sheen, FillRule::Winding, body_tx, None);
        }

        // Crisp warm deep-burgundy plush outline (guarantees legibility on light AND dark walls).
        let outline = Self::solid_paint(self.palette.outline);
        self.pixmap.stroke_path(
            &body_path,
            &outline,
            &self.stroke_outline(self.rig.outline_width),
            body_tx,
            None,
        );
    }

    /// Signature big snowy-white plush belly patch + minimalist HKUST-inspired club sundial emblem.
    fn draw_white_belly_and_emblem(&mut self, body_tx: Transform, pose: &PetPose) {
        let bx = 3.5;
        let by = 14.8;

        // Plump soft snowy-white tummy oval nestled cleanly in the lower red chest.
        let mut pb = PathBuilder::new();
        pb.move_to(bx, by - 16.8);
        pb.cubic_to(bx + 15.5, by - 16.8, bx + 22.8, by - 6.2, bx + 21.8, by + 4.8);
        pb.cubic_to(bx + 20.6, by + 15.2, bx + 11.8, by + 18.8, bx, by + 18.8);
        pb.cubic_to(bx - 11.8, by + 18.8, bx - 20.6, by + 15.2, bx - 21.8, by + 4.8);
        pb.cubic_to(bx - 22.8, by - 6.2, bx - 15.5, by - 16.8, bx, by - 16.8);
        pb.close();

        if let Some(belly) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(bx + 2.0, by - 6.0),
                Point::from_xy(bx, by),
                26.0,
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

            // Delicate soft border around the white plush patch.
            let seam = Self::solid_paint(self.palette.body_shade.with_alpha(0.32));
            self.pixmap
                .stroke_path(&belly, &seam, &self.stroke_outline(1.2), body_tx, None);
        }

        // Minimalist golden-sun & dark-ink geometric emblem right in the center of the white tummy!
        let ex = bx;
        let ey = by + 1.0;
        let sun_color = if pose.badge_glow > 0.4 {
            self.palette.badge_alert
        } else if pose.badge_glow > 0.1 {
            self.palette.badge_ok
        } else {
            self.palette.emblem_gold
        };

        // Top golden sun dot (pulses cobalt/emerald when alert/watching demands!).
        let sun_r = 2.6 + 0.55 * pose.badge_glow;
        if let Some(sun) = ellipse_path(ex, ey - 7.5, sun_r, sun_r) {
            let fill = Self::solid_paint(sun_color);
            self.pixmap
                .fill_path(&sun, &fill, FillRule::Winding, body_tx, None);
        }

        // Sleek twin curved emblem arcs + top crossbar (echoes the classic geometric crest cleanly).
        let mut ink = PathBuilder::new();
        // Top winged bar
        ink.move_to(ex - 6.0, ey - 3.6);
        ink.quad_to(ex, ey - 5.2, ex + 6.0, ey - 3.6);
        // Left & right stylized U-pillars
        ink.move_to(ex - 5.2, ey - 2.6);
        ink.line_to(ex - 5.2, ey + 2.5);
        ink.quad_to(ex - 5.0, ey + 6.2, ex - 2.0, ey + 6.8);
        ink.move_to(ex + 5.2, ey - 2.6);
        ink.line_to(ex + 5.2, ey + 2.5);
        ink.quad_to(ex + 5.0, ey + 6.2, ex + 2.0, ey + 6.8);
        // Inner vertical stems
        ink.move_to(ex - 2.2, ey - 2.4);
        ink.line_to(ex - 2.2, ey + 3.2);
        ink.move_to(ex + 2.2, ey - 2.4);
        ink.line_to(ex + 2.2, ey + 3.2);
        if let Some(path) = ink.finish() {
            let p = Self::solid_paint(self.palette.emblem_ink);
            self.pixmap
                .stroke_path(&path, &p, &self.stroke_outline(1.45), body_tx, None);
        }

        // Golden sundial triangle base at the bottom of the emblem.
        let mut tri = PathBuilder::new();
        tri.move_to(ex, ey + 2.4);
        tri.line_to(ex + 3.6, ey + 8.6);
        tri.line_to(ex - 3.6, ey + 8.6);
        tri.close();
        if let Some(path) = tri.finish() {
            let p = Self::solid_paint(self.palette.emblem_gold);
            self.pixmap
                .stroke_path(&path, &p, &self.stroke_outline(1.5), body_tx, None);
        }
    }

    fn draw_cheeks(&mut self, body_tx: Transform, pose: &PetPose) {
        let alpha = pose.blush.clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let paint = Self::solid_paint(self.palette.cheek.with_alpha(alpha * 0.55));
        for &(cx, cy, rx, ry) in &[(-15.5, -1.8, 5.8, 3.3), (23.5, -1.8, 5.4, 3.1)] {
            if let Some(cheek) = ellipse_path(cx, cy, rx, ry) {
                self.pixmap
                    .fill_path(&cheek, &paint, FillRule::Winding, body_tx, None);
            }
        }
    }

    /// Iconic big glossy plushie eyes: crisp white outer ring + electric sky-blue iris + big black pupil + catchlight!
    fn draw_eyes(&mut self, body_tx: Transform, pose: &PetPose) {
        // Slightly tilted symmetrical 3/4 face framing the plush beak.
        let eyes = [(-9.5f32, -10.5f32, -10.0f32), (17.5f32, -10.5f32, 10.0f32)];
        let ink = Self::solid_paint(self.palette.eye_ink);
        let sclera = Self::solid_paint(self.palette.eye_sclera);
        let iris = Self::solid_paint(self.palette.eye_iris);
        let iris_rim = Self::solid_paint(self.palette.eye_iris_deep);
        let sparkle = Self::solid_paint(self.palette.eye_sparkle);
        let brow = Self::solid_paint(self.palette.brow_accent);

        let gx = pose.gaze_x.clamp(-1.0, 1.0) * 1.6;
        let gy = pose.gaze_y.clamp(-1.0, 1.0) * 1.3;

        for &(ex, ey, tilt_deg) in &eyes {
            let etx = body_tx.pre_translate(ex, ey).pre_rotate(tilt_deg);

            match pose.eyes {
                EyeExpression::Normal => {
                    // Subtle embroidered red brow accent above eye socket (from plushie!).
                    let mut bpb = PathBuilder::new();
                    bpb.move_to(-6.2, -10.8);
                    bpb.quad_to(0.0, -13.2, 6.2, -10.6);
                    if let Some(bp) = bpb.finish() {
                        self.pixmap
                            .stroke_path(&bp, &brow, &self.stroke_outline(1.7), etx, None);
                    }
                    // 1. Crisp white outer ring
                    if let Some(outer) = ellipse_path(0.0, 0.0, 7.6, 8.8) {
                        self.pixmap
                            .fill_path(&outer, &sclera, FillRule::Winding, etx, None);
                        let rim = Self::solid_paint(self.palette.outline.with_alpha(0.75));
                        self.pixmap
                            .stroke_path(&outer, &rim, &self.stroke_outline(1.1), etx, None);
                    }
                    // 2. Electric sky-blue iris ring
                    if let Some(ir) = ellipse_path(gx * 0.3, gy * 0.3, 6.4, 7.5) {
                        self.pixmap
                            .fill_path(&ir, &iris, FillRule::Winding, etx, None);
                        self.pixmap
                            .stroke_path(&ir, &iris_rim, &self.stroke_outline(0.9), etx, None);
                    }
                    // 3. Large glossy jet-black pupil
                    if let Some(pup) = ellipse_path(gx * 0.55, gy * 0.55, 4.9, 5.9) {
                        self.pixmap
                            .fill_path(&pup, &ink, FillRule::Winding, etx, None);
                    }
                    // 4. Crisp twin catchlights
                    if let Some(hi1) =
                        ellipse_path(gx * 0.65 - 1.9, gy * 0.65 - 2.3, 2.0, 2.0)
                    {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, etx, None);
                    }
                    if let Some(hi2) =
                        ellipse_path(gx * 0.45 + 1.8, gy * 0.45 + 2.2, 0.95, 0.95)
                    {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, etx, None);
                    }
                }
                EyeExpression::Blink => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(-6.2, 0.8);
                    pb.quad_to(0.0, 3.8, 6.2, 0.5);
                    if let Some(arc) = pb.finish() {
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.6), etx, None);
                    }
                }
                EyeExpression::Happy => {
                    // Joyful curved upward smiling eyes with tiny white glow rim for crisp pop on crimson!
                    let mut pb = PathBuilder::new();
                    pb.move_to(-6.4, 2.4);
                    pb.quad_to(0.0, -5.2, 6.4, 2.4);
                    if let Some(arc) = pb.finish() {
                        let glow = Self::solid_paint(self.palette.eye_sclera.with_alpha(0.85));
                        self.pixmap
                            .stroke_path(&arc, &glow, &self.stroke_outline(4.8), etx, None);
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.6), etx, None);
                    }
                }
                EyeExpression::Drowsy => {
                    if let Some(outer) = ellipse_path(0.0, 0.6, 7.2, 6.2) {
                        self.pixmap
                            .fill_path(&outer, &sclera, FillRule::Winding, etx, None);
                    }
                    if let Some(ir) = ellipse_path(0.0, 1.0, 5.8, 4.9) {
                        self.pixmap
                            .fill_path(&ir, &iris, FillRule::Winding, etx, None);
                    }
                    if let Some(pup) = ellipse_path(0.0, 1.2, 4.3, 3.6) {
                        self.pixmap
                            .fill_path(&pup, &ink, FillRule::Winding, etx, None);
                    }
                    // Heavy plush red upper eyelid.
                    let lid_fill = Self::solid_paint(self.palette.body_base);
                    if let Some(cover) = ellipse_path(0.0, -3.5, 8.6, 4.2) {
                        self.pixmap
                            .fill_path(&cover, &lid_fill, FillRule::Winding, etx, None);
                    }
                    let mut pb = PathBuilder::new();
                    pb.move_to(-7.2, -0.8);
                    pb.line_to(7.2, -0.4);
                    if let Some(line) = pb.finish() {
                        self.pixmap
                            .stroke_path(&line, &ink, &self.stroke_outline(2.2), etx, None);
                    }
                }
                EyeExpression::Surprised => {
                    // Wide bright blue-ringed startled eyes!
                    if let Some(outer) = ellipse_path(0.0, 0.0, 8.4, 9.6) {
                        self.pixmap
                            .fill_path(&outer, &sclera, FillRule::Winding, etx, None);
                    }
                    if let Some(ir) = ellipse_path(0.0, 0.0, 7.1, 8.2) {
                        self.pixmap
                            .fill_path(&ir, &iris, FillRule::Winding, etx, None);
                    }
                    if let Some(pup) = ellipse_path(0.0, 0.0, 4.7, 5.6) {
                        self.pixmap
                            .fill_path(&pup, &ink, FillRule::Winding, etx, None);
                    }
                    if let Some(hi1) = ellipse_path(-2.0, -2.5, 2.3, 2.3) {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, etx, None);
                    }
                    if let Some(hi2) = ellipse_path(2.0, 2.3, 1.0, 1.0) {
                        self.pixmap
                            .fill_path(&hi2, &sparkle, FillRule::Winding, etx, None);
                    }
                    let mut bpb = PathBuilder::new();
                    bpb.move_to(-4.2, -12.0);
                    bpb.quad_to(0.0, -14.2, 4.2, -12.0);
                    if let Some(bp) = bpb.finish() {
                        self.pixmap
                            .stroke_path(&bp, &ink, &self.stroke_outline(1.8), etx, None);
                    }
                }
                EyeExpression::Focused => {
                    if let Some(outer) = ellipse_path(0.0, 0.0, 7.4, 8.0) {
                        self.pixmap
                            .fill_path(&outer, &sclera, FillRule::Winding, etx, None);
                    }
                    if let Some(ir) = ellipse_path(gx * 0.4, gy * 0.4, 6.2, 6.8) {
                        self.pixmap
                            .fill_path(&ir, &iris, FillRule::Winding, etx, None);
                    }
                    if let Some(pup) = ellipse_path(gx * 0.6, gy * 0.6, 4.6, 5.2) {
                        self.pixmap
                            .fill_path(&pup, &ink, FillRule::Winding, etx, None);
                    }
                    if let Some(hi1) = ellipse_path(gx * 0.7 - 1.6, gy * 0.7 - 1.8, 1.9, 1.9) {
                        self.pixmap
                            .fill_path(&hi1, &sparkle, FillRule::Winding, etx, None);
                    }
                    // Determined slanted brows on red fur with subtle white underline for crisp read!
                    let slant = if ex < 0.0 { 2.4 } else { -2.4 };
                    let mut bpb = PathBuilder::new();
                    bpb.move_to(-6.5, -9.2 - slant * 0.5);
                    bpb.line_to(6.5, -9.2 + slant * 0.5);
                    if let Some(bp) = bpb.finish() {
                        self.pixmap
                            .stroke_path(&bp, &ink, &self.stroke_outline(2.3), etx, None);
                    }
                }
                EyeExpression::Sleeping => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(-6.2, -0.4);
                    pb.quad_to(0.0, 4.6, 6.2, -0.4);
                    if let Some(arc) = pb.finish() {
                        let glow = Self::solid_paint(self.palette.eye_sclera.with_alpha(0.7));
                        self.pixmap
                            .stroke_path(&arc, &glow, &self.stroke_outline(4.2), etx, None);
                        self.pixmap
                            .stroke_path(&arc, &ink, &self.stroke_outline(2.4), etx, None);
                    }
                }
            }
        }
    }

    /// Plump berry-red plush beak nestled right between the eyes & top of the white belly!
    fn draw_beak(&mut self, body_tx: Transform, pose: &PetPose) {
        let bx = 4.0;
        let by = -2.2;
        let outline = Self::solid_paint(self.palette.outline);

        match pose.beak {
            BeakPose::Closed => {
                let mut pb = PathBuilder::new();
                pb.move_to(bx - 5.6, by - 0.8);
                pb.cubic_to(bx - 3.0, by - 5.5, bx + 3.0, by - 5.5, bx + 5.6, by - 0.8);
                pb.cubic_to(bx + 4.8, by + 3.6, bx + 1.5, by + 5.4, bx, by + 5.4);
                pb.cubic_to(bx - 1.5, by + 5.4, bx - 4.8, by + 3.6, bx - 5.6, by - 0.8);
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
                if let Some(hi) = ellipse_path(bx - 0.5, by - 1.8, 2.6, 1.3) {
                    let p = Self::solid_paint(self.palette.beak_top.with_alpha(0.85));
                    self.pixmap
                        .fill_path(&hi, &p, FillRule::Winding, body_tx, None);
                }
            }
            BeakPose::Chirp | BeakPose::HappyOpen => {
                let open = if pose.beak == BeakPose::HappyOpen {
                    4.8
                } else {
                    3.0
                };
                if let Some(mouth) = ellipse_path(bx, by + 1.8, 4.4, open * 0.78) {
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
                top.move_to(bx - 5.8, by - 0.4);
                top.cubic_to(bx - 2.8, by - 5.8, bx + 2.8, by - 5.8, bx + 5.8, by - 0.4);
                top.quad_to(bx, by + 1.8, bx - 5.8, by - 0.4);
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

    /// Signature HKUST Red Bird sundial-inspired flame-orange crown crest!
    /// Features one tall sweeping left flame blade + 3 stepped flame spikes driven by spring physics.
    fn draw_flame_crest(&mut self, body_tx: Transform, pose: &PetPose) {
        let sway_deg = pose.ahoge_angle.to_degrees() + pose.scarf_sway.to_degrees() * 0.35;
        let tx = body_tx.pre_translate(1.0, -33.2).pre_rotate(sway_deg);

        let mut pb = PathBuilder::new();
        // Start at left root of tall sweeping flame blade
        pb.move_to(-7.5, 2.2);
        // Tall sweeping outer flame curving gracefully up-and-left (echoes the HKUST Red Bird sundial!)
        pb.cubic_to(-12.0, -6.5, -9.0, -16.5, -14.8, -24.5);
        pb.cubic_to(-7.5, -20.0, -3.2, -12.5, -3.0, -3.8);
        // First mini stepped flame spike
        pb.quad_to(-0.8, -10.8, 3.2, -12.2);
        pb.quad_to(2.0, -6.8, 3.0, -3.4);
        // Second mini stepped flame spike
        pb.quad_to(6.2, -9.8, 10.2, -10.5);
        pb.quad_to(8.2, -5.8, 8.8, -2.5);
        // Third outer flame flick
        pb.quad_to(12.2, -7.2, 15.6, -7.0);
        pb.cubic_to(14.0, -1.8, 11.2, 1.5, 8.5, 2.6);
        // Smooth curved base nesting snugly into the red forehead
        pb.quad_to(0.5, 4.4, -7.5, 2.2);
        pb.close();

        if let Some(path) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(-12.0, -24.0),
                Point::from_xy(6.0, 4.0),
                vec![
                    GradientStop::new(0.0, self.palette.crest_tip.to_skia()),
                    GradientStop::new(0.65, self.palette.crest_base.to_skia()),
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

            let outline = Self::solid_paint(self.palette.outline);
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
                    let hy = body_y - 26.0 - p * 22.0 - i as f32 * 7.0;
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
                    let zx = anchor_x + facing * (28.0 + p * 14.0 + i as f32 * 6.0);
                    let zy = body_y - 18.0 - p * 26.0 - i as f32 * 6.0;
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
                // High-contrast club cobalt pill with white ring & mini envelope pops vibrantly beside red bird!
                let bx = anchor_x + facing * 36.0;
                let by = body_y - 32.0 - pulse * 2.5;
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
                let coords = [(-37.0, -30.0, 7.2), (40.0, -34.0, 8.4), (44.0, -8.0, 5.6)];
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
                let cy = body_y - 46.0;
                for i in 0..2 {
                    let p = (phase + i as f32 * 0.5).fract();
                    let r = 6.0 + p * 12.0;
                    let fade = (1.0 - p) * 0.65;
                    let mut pb = PathBuilder::new();
                    pb.move_to(cx - r * 0.6, cy - r * 0.45);
                    pb.quad_to(cx, cy - r * 0.95, cx + r * 0.6, cy - r * 0.45);
                    if let Some(arc) = pb.finish() {
                        let paint = Self::solid_paint(self.palette.eye_iris.with_alpha(fade));
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
