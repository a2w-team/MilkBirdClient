//! MilkBird Desktop Pet (`milkbird-pet`) — performance-first native desktop companion.

use milkbird_pet::platform::LaunchOptions;
use milkbird_pet::state::{BehaviorMode, PetState};
use std::path::PathBuf;
use std::process::ExitCode;

fn print_help() {
    println!(
        "\
milkbird-pet {ver} — 奶鸟俱乐部原生轻量桌宠（零 WebView）

用法:
  milkbird-pet [选项]

选项:
  --mode <free|stay|edge>          启动行为模式（自由散步 / 原地乖坐 / 贴边挂机）
  --state <idle|walk|petted|alert|sleep|perch|suspended>
                                   以指定状态启动（便于演示或性能实测）
  --demo [状态|tour]               演示模式（可选 `tour` 自动轮播全部 8 种状态与表情）
  --bubble <文本>                  启动时展示自定义轻量中文气泡
  --socket <路径>                  指定 `tray.sock` Unix Socket 路径（默认 ~/.milkbird/tray.sock）
  --offline                        纯离线伴侣模式（关闭 `tray.sock` 轮询）
  --duration <秒>                  运行指定秒数后自动退出（用于自动化压测/基准测试）
  --no-fullscreen-check            关闭全屏应用自动挂起检测
  --config <路径>                  指定配置文件路径（默认 ~/.milkbird/pet-config.json）
  -h, --help                       显示帮助信息
  -V, --version                    显示版本信息",
        ver = env!("CARGO_PKG_VERSION")
    );
}

fn parse_mode(raw: &str) -> Result<BehaviorMode, String> {
    match raw.to_ascii_lowercase().as_str() {
        "free" | "roam" | "freeroam" => Ok(BehaviorMode::FreeRoam),
        "stay" | "stayput" | "sit" => Ok(BehaviorMode::StayPut),
        "edge" | "perch" | "edgeperch" => Ok(BehaviorMode::EdgePerch),
        other => Err(format!("未知行为模式 `{other}`，可选: free | stay | edge")),
    }
}

fn parse_state(raw: &str) -> Result<PetState, String> {
    match raw.to_ascii_lowercase().as_str() {
        "idle" => Ok(PetState::Idle),
        "walk" | "hop" | "walkhop" => Ok(PetState::WalkHop),
        "drag" | "dragdangle" => Ok(PetState::DragDangle),
        "petted" | "pat" => Ok(PetState::Petted),
        "alert" | "alertmatch" => Ok(PetState::AlertMatch),
        "sleep" => Ok(PetState::Sleep),
        "perch" => Ok(PetState::Perch),
        "suspended" | "suspend" => Ok(PetState::Suspended),
        other => Err(format!(
            "未知状态 `{other}`，可选: idle | walk | petted | alert | sleep | perch | suspended"
        )),
    }
}

fn parse_args() -> Result<Option<LaunchOptions>, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut opts = LaunchOptions::default();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("milkbird-pet {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--mode" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--mode 需要参数（free | stay | edge）".to_string())?;
                opts.initial_mode = Some(parse_mode(&val)?);
            }
            "--state" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--state 需要状态名".to_string())?;
                opts.initial_state = Some(parse_state(&val)?);
            }
            "--demo" => {
                opts.offline = true;
                let next_is_arg = args.peek().is_some_and(|s| !s.starts_with('-'));
                if next_is_arg {
                    let val = args.next().unwrap();
                    if val.eq_ignore_ascii_case("tour") {
                        opts.demo_tour = true;
                    } else {
                        opts.initial_state = Some(parse_state(&val)?);
                    }
                } else {
                    opts.initial_state = Some(PetState::AlertMatch);
                }
                opts.demo_bubble
                    .get_or_insert_with(|| "叮咚！演示模式：点击我摸头，右键切换行为或轮播 8 种状态～".to_string());
            }
            "--bubble" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--bubble 需要气泡文本".to_string())?;
                opts.demo_bubble = Some(val);
            }
            "--socket" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--socket 需要 Unix Socket 路径".to_string())?;
                opts.socket_path = Some(PathBuf::from(val));
            }
            "--offline" => {
                opts.offline = true;
            }
            "--duration" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--duration 需要秒数".to_string())?;
                let secs: f64 = val
                    .parse()
                    .map_err(|_| format!("无效 --duration 秒数 `{val}`"))?;
                if secs <= 0.0 || !secs.is_finite() {
                    return Err("--duration 必须大于 0".to_string());
                }
                opts.duration_secs = Some(secs);
            }
            "--no-fullscreen-check" => {
                opts.auto_suspend_fullscreen = Some(false);
            }
            "--config" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--config 需要配置文件路径".to_string())?;
                opts.config_path = Some(PathBuf::from(val));
            }
            unknown => {
                return Err(format!("未知参数 `{unknown}`，使用 --help 查看用法"));
            }
        }
    }

    Ok(Some(opts))
}

fn main() -> ExitCode {
    let options = match parse_args() {
        Ok(Some(opts)) => opts,
        Ok(None) => return ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("参数错误: {msg}");
            return ExitCode::FAILURE;
        }
    };

    #[cfg(target_os = "macos")]
    {
        if let Err(err) = milkbird_pet::platform::macos::run(options) {
            eprintln!("milkbird-pet 启动失败: {err}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = options;
        eprintln!("milkbird-pet 当前桌面窗口后端仅支持 macOS（核心库支持跨平台测试）");
        ExitCode::FAILURE
    }
}
