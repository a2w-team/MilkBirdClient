//! Visual preview exporter & performance benchmark CLI for MilkBird Pet core (M1).
//!
//! Exports `@2x` Retina PNGs for key poses plus a combined 4x2 split light/dark showcase sheet,
//! and prints measured single-frame rasterization & hit-test latency benchmarks.

use milkbird_pet::model::{EyeExpression, PetPose, PetRig, PropKind};
use milkbird_pet::render::{PetRenderer, render_showcase_sheet};
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

    // Adaptive scheduler deduplication check over static suspended/settled poses.
    let mut sched = FrameScheduler::new();
    let static_pose = poses[0].1;
    for _ in 0..100 {
        let _ = sched.plan_frame(milkbird_pet::scheduler::FpsTier::IdleBreeze, &static_pose);
    }
    let stats = sched.stats();
    let suspend_decision = sched.plan_frame(milkbird_pet::scheduler::FpsTier::Suspended, &static_pose);

    println!("=== MilkBird Pet Core M1 Preview & Benchmark ===");
    println!("Output dir        : {}", out_dir.display());
    println!("Showcase sheet    : {}", sheet_path.display());
    println!(
        "Model footprints  : PetRig={} B, PetPose={} B, Pixmap={}x{} ({} KB)",
        size_of::<PetRig>(),
        size_of::<PetPose>(),
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
        matches!(suspend_decision, FrameDecision::Skip { next_interval: None, .. })
    );
}
