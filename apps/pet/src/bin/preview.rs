//! Visual preview exporter & performance benchmark CLI for MilkBird Pet core (M1).
//!
//! Exports:
//! - Individual `@2x` Retina PNGs for key poses
//! - `milkbird-showcase.png`: 8 behavioral states on split light/dark backdrops
//! - `milkbird-customization.png`: 4 hairstyles (row 1) + 4 body shapes (row 2)
//!
//! Prints measured single-frame rasterization & hit-test latency benchmarks.

use milkbird_pet::model::{
    BodyShape, EyeExpression, HairStyle, PetAppearance, PetPose, PetRig, PropKind,
};
use milkbird_pet::render::{PetRenderer, render_showcase_sheet, render_variant_sheet};
use milkbird_pet::scheduler::{FrameDecision, FrameScheduler};
use milkbird_pet::state::{PetController, PetEvent, PetState};
use std::env;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use std::time::Instant;

fn showcase_poses() -> Vec<(&'static str, PetPose)> {
    let build = |state: PetState, ticks: usize| {
        let mut ctrl = PetController::new(42);
        ctrl.handle_event(PetEvent::DemoState(state));
        let mut pose = ctrl.compose_pose();
        for _ in 0..ticks {
            let (p, _) = ctrl.tick(1.0 / 60.0);
            pose = p;
        }
        pose
    };

    let mut focused = build(PetState::Idle, 10);
    focused.eyes = EyeExpression::Focused;
    focused.prop = PropKind::FocusRipple { phase: 0.35 };
    focused.badge_glow = 0.25;

    vec![
        ("idle", build(PetState::Idle, 15)),
        ("walk_hop", build(PetState::WalkHop, 12)),
        ("drag_dangle", build(PetState::DragDangle, 18)),
        ("petted", build(PetState::Petted, 10)),
        ("alert_match", build(PetState::AlertMatch, 10)),
        ("sleep", build(PetState::Sleep, 90)),
        ("perch", build(PetState::Perch, 20)),
        ("focused_watch", focused),
    ]
}

fn customization_variants() -> Vec<(&'static str, PetAppearance, PetPose)> {
    let idle = PetPose::default();
    vec![
        // Row 1: Hairstyles 1-4 (自然呆毛 / 元气双马尾 / 霸总大背头 / 憨萌蘑菇头)
        (
            "hair_natural_ahoge",
            PetAppearance::default().with_hair(HairStyle::NaturalAhoge),
            idle,
        ),
        (
            "hair_twin_tails",
            PetAppearance::default().with_hair(HairStyle::TwinTails),
            idle,
        ),
        (
            "hair_slick_back",
            PetAppearance::default().with_hair(HairStyle::SlickBack),
            idle,
        ),
        (
            "hair_mushroom_bob",
            PetAppearance::default().with_hair(HairStyle::MushroomBob),
            idle,
        ),
        // Row 2: Hairstyles 5-8 (奶咖卷刘海 / 俏皮双丸子 / 葬爱爆炸头 / 银粉杀马特)
        (
            "hair_curly_bangs",
            PetAppearance::default().with_hair(HairStyle::CurlyBangs),
            idle,
        ),
        (
            "hair_twin_buns",
            PetAppearance::default().with_hair(HairStyle::TwinBuns),
            idle,
        ),
        (
            "hair_shamate_spike",
            PetAppearance::default().with_hair(HairStyle::ShamateSpike),
            idle,
        ),
        (
            "hair_shamate_flame",
            PetAppearance::default().with_hair(HairStyle::ShamateFlame),
            idle,
        ),
        // Row 3: 4 Body Shapes (圆团子 / 梨形大肚 / 大福麻薯 / 长条软豆)
        (
            "body_round_ball",
            PetAppearance::default().with_body(BodyShape::RoundBall),
            idle,
        ),
        (
            "body_pear_chubby",
            PetAppearance::default().with_body(BodyShape::PearChubby),
            idle,
        ),
        (
            "body_mochi_flat",
            PetAppearance::default().with_body(BodyShape::MochiFlat),
            idle,
        ),
        (
            "body_tall_bean",
            PetAppearance::default().with_body(BodyShape::TallBean),
            idle,
        ),
    ]
}

fn main() {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("milkbird-pet-preview"));
    fs::create_dir_all(&out_dir).expect("create preview output directory");

    let poses = showcase_poses();
    let mut renderer = PetRenderer::new(2.0);

    for (name, pose) in &poses {
        let pixmap = renderer.render(pose);
        let path = out_dir.join(format!("milkbird-{name}.png"));
        pixmap.save_png(&path).expect("save pose png");
    }

    let sheet = render_showcase_sheet(&poses, 2.0);
    let sheet_path = out_dir.join("milkbird-showcase.png");
    sheet.save_png(&sheet_path).expect("save showcase sheet");

    let variants = customization_variants();
    let custom_sheet = render_variant_sheet(&variants, 2.0);
    let custom_path = out_dir.join("milkbird-customization.png");
    custom_sheet
        .save_png(&custom_path)
        .expect("save customization sheet");

    // Performance benchmark: 500 Retina @2x (320x320) frames across animated states.
    let mut ctrl = PetController::new(7);
    ctrl.handle_event(PetEvent::DemoState(PetState::AlertMatch));
    let mut times_us = Vec::with_capacity(500);
    for _ in 0..500 {
        let (pose, _) = ctrl.tick(1.0 / 60.0);
        let start = Instant::now();
        let _ = renderer.render(&pose);
        times_us.push(start.elapsed().as_micros() as u64);
    }
    times_us.sort_unstable();
    let avg_us = times_us.iter().sum::<u64>() as f64 / times_us.len() as f64;
    let p95_us = times_us[(times_us.len() * 95) / 100];
    let p99_us = times_us[(times_us.len() * 99) / 100];

    // Hit-test latency benchmark: 50,000 O(1) alpha checks.
    let hit_start = Instant::now();
    let mut hits = 0u64;
    for i in 0..50_000u32 {
        let x = (i % 160) as f32 + 0.5;
        let y = ((i / 160) % 160) as f32 + 0.5;
        if renderer.hit_test(x, y) {
            hits += 1;
        }
    }
    let hit_ns = hit_start.elapsed().as_nanos() as f64 / 50_000.0;

    // Adaptive scheduler deduplication check.
    let mut sched = FrameScheduler::new();
    let static_pose = poses[0].1;
    for _ in 0..100 {
        let _ = sched.plan_frame(milkbird_pet::scheduler::FpsTier::IdleBreeze, &static_pose);
    }
    let stats = sched.stats();
    let suspend_decision =
        sched.plan_frame(milkbird_pet::scheduler::FpsTier::Suspended, &static_pose);

    println!("=== MilkBird Pet Core M1 Preview & Benchmark ===");
    println!("Output dir        : {}", out_dir.display());
    println!("Showcase sheet    : {}", sheet_path.display());
    println!("Custom sheet      : {}", custom_path.display());
    println!(
        "Model footprints  : PetRig={} B, PetPose={} B, PetAppearance={} B, Pixmap={}x{} ({} KB)",
        size_of::<PetRig>(),
        size_of::<PetPose>(),
        size_of::<PetAppearance>(),
        renderer.pixel_size().0,
        renderer.pixel_size().1,
        renderer.pixels().len() / 1024
    );
    println!(
        "Render @2x 320px  : avg={:.2} us ({:.3} ms), p95={} us, p99={} us",
        avg_us,
        avg_us / 1000.0,
        p95_us,
        p99_us
    );
    println!("Hit-test latency  : avg={hit_ns:.1} ns ({hits}/50000 hits)");
    println!(
        "Dirty skip (idle) : rendered={}, skipped={} ({:.1}% saved)",
        stats.rendered_frames,
        stats.skipped_frames,
        stats.skip_ratio() * 100.0
    );
    println!(
        "Suspended mode    : {:?}",
        matches!(
            suspend_decision,
            FrameDecision::Skip {
                next_interval: None,
                ..
            }
        )
    );
}
