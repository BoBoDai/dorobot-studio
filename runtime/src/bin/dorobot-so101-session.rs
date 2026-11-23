use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use dorobot_nano_runtime::{
    physics::Simulation,
    protocol::{Frame, now_ns},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, BufWriter, Write},
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const STEPS_PER_SAMPLE: u64 = 5;
const SAMPLE_PERIOD: Duration = Duration::from_millis(10 * STEPS_PER_SAMPLE);
const INPUT_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Parser)]
#[command(version, about = "Native Nexus SO-101 tabletop session")]
struct Args {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Subcommand)]
enum Mode {
    Capture {
        model: PathBuf,
        scene: PathBuf,
        session: PathBuf,
    },
    Rollout {
        model: PathBuf,
        scene: PathBuf,
        initial_json: PathBuf,
        steps: u64,
    },
}

#[derive(Deserialize)]
struct Action {
    seq: u64,
    target_rad: [f64; 6],
}

struct LeaderInput {
    socket: UnixDatagram,
    path: PathBuf,
    received: Option<([f64; 6], Instant)>,
}

impl LeaderInput {
    fn bind(path: PathBuf) -> Result<Self> {
        let socket = UnixDatagram::bind(&path)
            .with_context(|| format!("Open leader socket {}", path.display()))?;
        let input = Self {
            socket,
            path,
            received: None,
        };
        input.socket.set_nonblocking(true)?;
        Ok(input)
    }

    fn latest(&mut self) -> Result<Option<[f64; 6]>> {
        let mut buffer = [0; 256];
        for _ in 0..64 {
            match self.socket.recv(&mut buffer) {
                Ok(size) => {
                    if let Ok(positions) = serde_json::from_slice::<[f64; 6]>(&buffer[..size])
                        && positions.iter().all(|q| q.is_finite())
                    {
                        self.received = Some((positions, Instant::now()));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("Read leader socket"),
            }
        }
        Ok(self
            .received
            .filter(|(_, stamp)| stamp.elapsed() < INPUT_TIMEOUT)
            .map(|(positions, _)| positions))
    }
}

impl Drop for LeaderInput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn take_marker(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Consume marker {}", path.display())),
    }
}

fn tabletop_objects(simulation: &Simulation) -> Result<(usize, usize)> {
    let index = |name| {
        simulation
            .objects
            .iter()
            .position(|object| object.name == name)
            .with_context(|| format!("Missing tabletop object {name}"))
    };
    Ok((index("cube_6")?, index("plate")?))
}

fn sample(simulation: &mut Simulation, sequence: u64, phase: &str) -> Result<Frame> {
    simulation.frame(sequence, false, Vec::new(), vec![0.0; 6], phase.into())
}

fn write_packet(output: &mut impl Write, header: &Value, frame: &Frame) -> Result<()> {
    serde_json::to_writer(&mut *output, header)?;
    output.write_all(b"\n")?;
    output.write_all(&frame.cameras[0].rgb)?;
    output.flush()?;
    Ok(())
}

fn capture(model: &Path, scene: &Path, session: &Path) -> Result<()> {
    ensure!(
        session.is_dir(),
        "Missing session directory {}",
        session.display()
    );
    let mut simulation = Simulation::with_so101_scene(model, scene)?;
    let (cube, plate) = tabletop_objects(&simulation)?;
    let mut leader = LeaderInput::bind(session.join("leader.sock"))?;
    let stop = session.join("stop");
    let reset = session.join("reset-request");
    let pause = session.join("pause");
    let step = session.join("step-request");
    let mut output = BufWriter::new(io::stdout().lock());
    let mut deadline = Instant::now();
    let mut sequence = 0;
    let mut reset_id = 0_u64;

    while !stop.try_exists()? {
        if take_marker(&reset)? {
            simulation.reset_tabletop_cube()?;
            reset_id += 1;
        }
        let paused = pause.try_exists()?;
        let single_step = take_marker(&step)?;
        let angles = leader.latest()?;
        if (!paused || single_step)
            && let Some(angles) = angles
        {
            for (index, ((target, value), [lo, hi])) in simulation
                .targets
                .iter_mut()
                .zip(angles)
                .zip(&simulation.model.limits)
                .enumerate()
            {
                *target = if index == 5 {
                    lo + (hi - lo) * (value / 100.0).clamp(0.0, 1.0)
                } else {
                    value.to_radians().clamp(*lo, *hi)
                };
            }
        }
        if paused && single_step {
            simulation.step(false)?;
        }

        let frame = sample(&mut simulation, sequence, "SO-101 sim")?;
        write_packet(
            &mut output,
            &json!({
                "seq": sequence,
                "reset_id": reset_id,
                "paused": paused,
                "sim_tick": simulation.tick,
                "cube_xyz": &frame.transforms[cube][12..15],
                "plate_xyz": &frame.transforms[plate][12..15],
                "capture_monotonic_ns": now_ns(),
                "leader_deg_percent": angles,
                "state_rad": &frame.observation[7..13],
                "target_rad": simulation.targets,
                "gripper_limits_rad": simulation.model.limits[5],
                "rgb_bytes": frame.cameras[0].rgb.len(),
            }),
            &frame,
        )?;
        if !paused {
            for _ in 0..STEPS_PER_SAMPLE {
                simulation.step(false)?;
            }
        }
        sequence += 1;
        deadline = (deadline + SAMPLE_PERIOD).max(Instant::now());
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
    }
    Ok(())
}

fn rollout(model: &Path, scene: &Path, initial_json: &Path, steps: u64) -> Result<()> {
    let positions =
        serde_json::from_slice::<[f64; 6]>(&fs::read(initial_json)?).with_context(|| {
            format!(
                "Expected six radian positions in {}",
                initial_json.display()
            )
        })?;
    let mut simulation = Simulation::with_so101_scene(model, scene)?;
    simulation.set_so101_joint_positions(positions)?;
    let (cube, plate) = tabletop_objects(&simulation)?;
    let mut input = io::stdin().lock().lines();
    let mut output = BufWriter::new(io::stdout().lock());

    for sequence in 0..=steps {
        let frame = sample(&mut simulation, sequence, "ACT evaluation")?;
        write_packet(
            &mut output,
            &json!({
                "seq": sequence,
                "sim_tick": simulation.tick,
                "state_rad": &frame.observation[7..13],
                "cube_xyz": &frame.transforms[cube][12..15],
                "plate_xyz": &frame.transforms[plate][12..15],
                "joint_limits_rad": simulation.model.limits,
                "rgb_bytes": frame.cameras[0].rgb.len(),
            }),
            &frame,
        )?;
        if sequence == steps {
            break;
        }
        let action: Action =
            serde_json::from_str(&input.next().context("Missing rollout action")??)
                .context("Invalid rollout action")?;
        ensure!(
            action.seq == sequence,
            "Action sequence {} does not match observation {sequence}",
            action.seq
        );
        ensure!(
            action.target_rad.iter().all(|q| q.is_finite()),
            "Invalid action"
        );
        for ((target, value), [lo, hi]) in simulation
            .targets
            .iter_mut()
            .zip(action.target_rad)
            .zip(&simulation.model.limits)
        {
            *target = value.clamp(*lo, *hi);
        }
        for _ in 0..STEPS_PER_SAMPLE {
            simulation.step(false)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().command {
        Mode::Capture {
            model,
            scene,
            session,
        } => capture(&model, &scene, &session),
        Mode::Rollout {
            model,
            scene,
            initial_json,
            steps,
        } => rollout(&model, &scene, &initial_json, steps),
    }
}
