use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use dorobot_nano_runtime::protocol::*;
use serde_json::json;
use std::{
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Parser)]
#[command(version, about = "Native Rust Nano / Dora runtime")]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}
#[derive(Subcommand)]
enum Mode {
    #[cfg(feature = "physics")]
    AisleNode,
    Sim(Session),
    Base(Session),
    Node {
        role: String,
    },
    Capture {
        mode: String,
        profile: PathBuf,
        staging: PathBuf,
    },
    Hub {
        mode: String,
        repo: String,
        path: PathBuf,
    },
    #[cfg(feature = "physics")]
    Probe {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 5)]
        frames: u64,
    },
}
#[derive(clap::Args)]
struct Session {
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long)]
    owner_pid: Option<u32>,
    #[arg(long)]
    mock: bool,
    #[arg(long)]
    port: Option<String>,
    #[arg(long)]
    self_test: bool,
}
fn dora() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("DOROBOT_DORA") {
        return Ok(p.into());
    }
    let beside = std::env::current_exe()?
        .parent()
        .context("Runtime directory")?
        .join("dora");
    Ok(if beside.is_file() {
        beside
    } else {
        PathBuf::from("dora")
    })
}
fn supervisor(args: Session, base: bool) -> Result<()> {
    ensure!(
        !base || args.mock || args.port.as_ref().is_some_and(|p| !p.trim().is_empty()),
        "Choose a serial port or --mock"
    );
    ensure!(
        !args.self_test || !base || args.mock,
        "Self-test is restricted to simulation or mock hardware"
    );
    ensure!(
        base || !args.output.exists(),
        "Use a new simulation output folder"
    );
    fs::create_dir_all(&args.output)?;
    let output = args.output.canonicalize()?;
    let socket = args.socket.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("dorobot-nano-{}.sock", std::process::id()))
    });
    ensure!(!socket.exists(), "Preview socket already exists");
    let executable = std::env::current_exe()?;
    let default_model = executable
        .parent()
        .unwrap()
        .join("../../studio/data/adora_nano/adora_nano.urdf");
    let model = args.model.unwrap_or(default_model);
    ensure!(base || model.is_file(), "Nano URDF is missing");
    let config = Config {
        model,
        output: output.clone(),
        socket,
        run_id: output.file_name().unwrap().to_string_lossy().into(),
        owner_pid: args.owner_pid,
        supervisor_pid: std::process::id(),
        base,
        mock: args.mock,
        port: args.port,
        self_test: args.self_test,
    };
    let cli = dora()?;
    let version = Command::new(&cli)
        .arg("--version")
        .output()
        .context("Native Dora CLI is missing")?;
    ensure!(
        version.status.success()
            && String::from_utf8_lossy(&version.stdout).trim() == "dora-cli 1.0.1",
        "Native Dora CLI 1.0.1 is required"
    );
    let config_path = output.join("config.json");
    write_json(&config_path, &config)?;
    let timer =
        |millis: u32| json!({"source":format!("dora/timer/millis/{millis}"),"queue_size":1});
    let source = |s: &str, size: u32| json!({"source":s,"queue_size":size});
    let mut nodes = vec![
        json!({"id":"gamepad","inputs":{"tick":timer(20)},"outputs":["input"]}),
        json!({"id":"control","inputs":{"input":source("gamepad/input",1)},"outputs":["command"]}),
        json!({"id":"guard","inputs":{"command":source("control/command",1)},"outputs":["safe_command"]}),
    ];
    if base {
        nodes.push(json!({"id":"driver","inputs":{"command":source("guard/safe_command",1),"tick":timer(100)},"outputs":["status"]}));
    } else {
        nodes.extend([
            json!({"id":"sim","inputs":{"command":source("guard/safe_command",1),"tick":timer(20),"viewer_ready":"preview/viewer_ready"},"outputs":["scene_info","preview_frame","recording"]}),
            json!({"id":"recorder","inputs":{"recording":source("sim/recording",64),"tick":timer(200)}}),
            json!({"id":"preview","inputs":{"scene_info":"sim/scene_info","frame":source("sim/preview_frame",1)},"outputs":["viewer_ready"]}),
        ]);
    }
    for node in &mut nodes {
        node["path"] = json!(executable);
        node["args"] = json!(format!("node {}", node["id"].as_str().unwrap()));
        node["env"] = json!({"DOROBOT_NATIVE_CONFIG":config_path});
    }
    let graph = output.join("dataflow.yaml");
    fs::write(&graph, serde_yaml::to_string(&json!({"nodes":nodes}))?)?;
    let log = fs::File::create(output.join("dora.log"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::Release))?;
    let mut child = Command::new(cli)
        .args(["run"])
        .arg(&graph)
        .args(["--log-format", "json"])
        .current_dir(&output)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let mut failure = None;
    let mut requested = false;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                failure = Some(anyhow::anyhow!(
                    "Dora stopped: {status}; see {}",
                    output.join("dora.log").display()
                ));
            }
            break;
        }
        if stop.load(Ordering::Acquire)
            || output.join("stop").exists()
            || config
                .owner_pid
                .is_some_and(|pid| unsafe { libc::kill(pid as i32, 0) != 0 })
        {
            requested = true;
            break;
        }
        if let Some(file) = fs::read_dir(&output)?
            .filter_map(Result::ok)
            .find(|p| p.file_name().to_string_lossy().starts_with("failure-"))
        {
            failure = Some(anyhow::anyhow!("{}", fs::read_to_string(file.path())?));
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if child.try_wait()?.is_none() {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGTERM);
        }
        let started = Instant::now();
        while child.try_wait()?.is_none() && started.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(50));
        }
        if child.try_wait()?.is_none() {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            child.wait()?;
        }
    }
    let _ = fs::remove_file(&config.socket);
    if !requested {
        if let Some(error) = failure {
            return Err(error);
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    match Args::parse().mode {
        #[cfg(feature = "physics")]
        Mode::AisleNode => dorobot_nano_runtime::aisle::run(),
        Mode::Sim(a) => supervisor(a, false),
        Mode::Base(a) => supervisor(a, true),
        Mode::Capture {
            mode,
            profile,
            staging,
        } => dorobot_nano_runtime::capture::run(&mode, &profile, &staging),
        Mode::Hub { mode, repo, path } => dorobot_nano_runtime::hub::transfer(&mode, &repo, &path),
        Mode::Node { role } => {
            let config: Config = serde_json::from_slice(&fs::read(
                std::env::var_os("DOROBOT_NATIVE_CONFIG").context("Dora node config missing")?,
            )?)?;
            write_json(
                &config.output.join(format!("node-{role}.json")),
                &json!({"pid":std::process::id(),"runtime":"rust"}),
            )?;
            let result = dorobot_nano_runtime::nodes::run(&role, &config);
            if let Err(error) = &result {
                let _ = write_json(
                    &config.output.join(format!("failure-{role}.json")),
                    &json!({"node":role,"error":format!("{error:#}")}),
                );
            }
            result
        }
        #[cfg(feature = "physics")]
        Mode::Probe {
            model,
            output,
            frames,
        } => {
            fs::create_dir_all(&output)?;
            let mut sim = dorobot_nano_runtime::physics::Simulation::new(&model)?;
            for seq in 0..frames {
                for _ in 0..20 {
                    sim.step(true)?;
                }
                let frame =
                    sim.frame(seq, true, Vec::new(), vec![0.0; 17], "Native probe".into())?;
                write_json(
                    &output.join(format!("frame-{seq}.json")),
                    &json!({"sequence":seq,"observation":frame.observation,"cameras":frame.cameras.iter().map(|c|json!({"name":c.name,"rgb_bytes":c.rgb.len(),"depth_pixels":c.depth.len(),"seg_pixels":c.seg.len(),"world_from_camera_gl":c.world_from_camera_gl})).collect::<Vec<_>>()}),
                )?;
            }
            Ok(())
        }
    }
}
