#[cfg(feature = "physics")]
use crate::control;
use crate::{
    control::{Guard, Mapper, Model},
    protocol::*,
};
use anyhow::{Result, ensure};
use dora_node_api::{DoraArray, DoraNode, Event, EventStream};
use gilrs::{Axis, Button, Gilrs};
#[cfg(feature = "physics")]
use serde_json::{Value, json};
#[cfg(feature = "physics")]
use std::{fs, time::Instant};

pub fn send(node: &mut DoraNode, id: &str, message: &Message) -> Result<()> {
    let bytes = bincode::serialize(message)?;
    node.send_output_bytes(
        id.to_owned().into(),
        Default::default(),
        bytes.len(),
        &bytes,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
pub fn decode(data: &DoraArray) -> Result<Message> {
    let bytes: &[u8] = data.try_into().map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(bincode::deserialize(bytes)?)
}
pub fn run(role: &str, config: &Config) -> Result<()> {
    let (mut node, mut events) = DoraNode::init_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    match role {
        "gamepad" => gamepad(config, &mut node, &mut events),
        "control" | "guard" => control_node(role, config, &mut node, &mut events),
        "preview" => crate::preview::run(config, &mut node, &mut events),
        "recorder" => crate::record::run(config, &mut events),
        "driver" => crate::base::run(config, &mut node, &mut events),
        #[cfg(feature = "physics")]
        "sim" => sim(config, &mut node, &mut events),
        _ => anyhow::bail!("Unknown or unavailable native node: {role}"),
    }
}
fn gamepad(config: &Config, node: &mut DoraNode, events: &mut EventStream) -> Result<()> {
    let mut gilrs = Gilrs::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut seq = 0;
    while let Some(event) = events.recv() {
        match event {
            Event::Input { .. } => {
                while gilrs.next_event().is_some() {}
                let mut input = Input {
                    sequence: seq,
                    input_ns: now_ns(),
                    name: "No controller".into(),
                    ..Default::default()
                };
                if config.self_test {
                    ensure!(
                        config.mock || !config.base,
                        "Self-test cannot drive real hardware"
                    );
                    input.connected = true;
                    input.name = "Native test input".into();
                    if config.base && (40..200).contains(&seq) {
                        input.buttons.insert("enable".into(), true);
                        let (name, value) = match seq / 40 {
                            1 => ("left_y", -0.6),
                            2 => ("left_x", 0.6),
                            _ => ("right_x", -0.6),
                        };
                        input.axes.insert(name.into(), value);
                    } else if !config.base {
                        // Advance against rendered simulation frames, not startup
                        // wall time: shader warmup must not consume the test drive.
                        let stage = std::fs::read(config.output.join("progress.json"))
                            .ok()
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|v| v["frame"].as_u64())
                            .map(|frame| frame / 5)
                            .unwrap_or(0);
                        if matches!(stage, 5 | 9 | 13) {
                            input.buttons.insert("mode".into(), true);
                        } else if matches!(stage, 2..=4 | 7..=8 | 11..=12 | 15..=16) {
                            input.buttons.insert("enable".into(), true);
                            if stage <= 4 {
                                input.axes.insert(
                                    if stage == 4 { "right_x" } else { "left_y" }.into(),
                                    -0.6,
                                );
                            } else {
                                for name in ["left_x", "left_y", "right_x", "right_y"] {
                                    input.axes.insert(name.into(), 0.6);
                                }
                                input.buttons.insert("right".into(), true);
                                input.buttons.insert("close".into(), true);
                            }
                        }
                    }
                } else if let Some((_, pad)) = gilrs.gamepads().find(|(_, p)| p.is_connected()) {
                    input.connected = true;
                    input.name = pad.name().into();
                    for (name, axis) in [
                        ("left_x", Axis::LeftStickX),
                        ("left_y", Axis::LeftStickY),
                        ("right_x", Axis::RightStickX),
                        ("right_y", Axis::RightStickY),
                    ] {
                        // gilrs is +Y up; the existing Nano mapping is +Y down.
                        input.axes.insert(
                            name.into(),
                            pad.value(axis) as f64 * if name.ends_with('y') { -1.0 } else { 1.0 },
                        );
                    }
                    for (name, button) in [
                        ("enable", Button::LeftTrigger),
                        ("mode", Button::RightTrigger),
                        ("pause", Button::Start),
                        ("open", Button::South),
                        ("close", Button::East),
                        ("up", Button::DPadUp),
                        ("down", Button::DPadDown),
                        ("left", Button::DPadLeft),
                        ("right", Button::DPadRight),
                    ] {
                        input.buttons.insert(name.into(), pad.is_pressed(button));
                    }
                }
                send(node, "input", &Message::Input(input.clone()))?;
                if seq % 10 == 0 {
                    write_json(&config.output.join("controller.json"), &input)?;
                }
                seq += 1;
            }
            Event::Stop(_) => break,
            Event::Error(e) => anyhow::bail!(e),
            _ => {}
        }
    }
    Ok(())
}
fn control_node(
    role: &str,
    config: &Config,
    node: &mut DoraNode,
    events: &mut EventStream,
) -> Result<()> {
    let names = if config.base {
        vec![
            "wheel_2_joint".into(),
            "wheel_1_joint".into(),
            "wheel_3_joint".into(),
        ]
    } else {
        Model::read_nano(&config.model)?.names
    };
    let mut mapper = Mapper::new(names.clone());
    let mut guard = Guard::new(names);
    let mut base_mapper = crate::base::BaseMapper::default();
    let mut base_guard = crate::base::BaseGuard::default();
    while let Some(event) = events.recv() {
        match event {
            Event::Input { data, .. } => match decode(&data)? {
                Message::Input(i) if role == "control" => send(
                    node,
                    "command",
                    &Message::Command(if config.base {
                        base_mapper.command(&i)?
                    } else {
                        mapper.command(&i)?
                    }),
                )?,
                Message::Command(c) if role == "guard" => {
                    let c = if config.base {
                        base_guard.accept(c, now_ns())?
                    } else {
                        guard.accept(c, now_ns())?
                    };
                    if c.sequence % 10 == 0 {
                        write_json(&config.output.join("command.json"), &c)?;
                    }
                    send(node, "safe_command", &Message::Command(c))?;
                }
                _ => anyhow::bail!("Unexpected control message"),
            },
            Event::Stop(_) => break,
            Event::Error(e) => anyhow::bail!(e),
            _ => {}
        }
    }
    Ok(())
}
#[cfg(feature = "physics")]
fn sim(config: &Config, node: &mut DoraNode, events: &mut EventStream) -> Result<()> {
    write_json(
        &config.output.join("startup.json"),
        &json!({"message":"Preparing native physics and 3 cameras…"}),
    )?;
    let mut sim = crate::physics::Simulation::new(&config.model)?;
    let info = json!({"version":1,"session":format!("Nano gamepad · {}",config.run_id),"fps":FPS,"joint_names":sim.model.names,"objects":sim.objects,"cameras":sim.camera_specs,"recording":false,"physics_dt":DT,"requested_control_hz":50,"mode":"simulation","controller":"PS4 / PS5 via gilrs","studio_session_version":1,"contact_audit_hz":100,"initial_objects":sim.initial_objects,"model":config.model.file_name(),"runtime":"rust","engine":"Nexus ca2cc91"});
    write_json(&config.output.join("scene-info.json"), &info)?;
    send(node, "scene_info", &Message::Info(info.to_string()))?;
    let mut command: Option<Command> = None;
    let mut seq = 0;
    let started = Instant::now();
    let mut next_preview = 20;
    let mut armed_before = false;
    let mut rearm = false;
    let mut request_id = Value::Null;
    let mut recording = false;
    let mut record_seq = 0;
    let mut contacts = Vec::new();
    while let Some(event) = events.recv() {
        match event {
            Event::Input { id, data, .. } => {
                if id.as_str() == "command" {
                    let Message::Command(c) = decode(&data)? else {
                        anyhow::bail!("Expected guarded command")
                    };
                    ensure!(c.guarded, "Unguarded command");
                    if command.as_ref().is_none_or(|p| c.sequence > p.sequence) {
                        command = Some(c);
                    }
                    continue;
                }
                if id.as_str() != "tick" {
                    continue;
                }
                if let Ok(request) = fs::read(config.output.join("session-command.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .ok_or(())
                {
                    if !request["id"].is_null() && request["id"] != request_id {
                        request_id = request["id"].clone();
                        match request["action"].as_str() {
                            Some("record") if !recording => {
                                send(
                                    node,
                                    "recording",
                                    &Message::RecordStart {
                                        info: info.to_string(),
                                        request: request.to_string(),
                                    },
                                )?;
                                recording = true;
                                record_seq = 0;
                                contacts.clear();
                            }
                            Some("stop") if recording => {
                                recording = false;
                                send(
                                    node,
                                    "recording",
                                    &Message::RecordStop {
                                        request: request.to_string(),
                                        frame_count: record_seq,
                                    },
                                )?;
                            }
                            Some("save") if !recording => {
                                send(node, "recording", &Message::RecordSave(request.to_string()))?
                            }
                            Some("discard") if !recording => send(
                                node,
                                "recording",
                                &Message::RecordDiscard(request.to_string()),
                            )?,
                            _ => {}
                        }
                    }
                }
                let now = now_ns();
                let fresh = command.as_ref().is_some_and(|c| {
                    now.checked_sub(c.input_ns)
                        .is_some_and(|v| v <= control::TIMEOUT_NS)
                });
                if armed_before && !fresh {
                    rearm = true;
                }
                if fresh && command.as_ref().is_some_and(|c| !c.enable_held) {
                    rearm = false;
                }
                let armed = control::active(command.as_ref(), now) && !rearm;
                let velocity = if armed {
                    command.as_ref().unwrap().velocities.clone()
                } else {
                    vec![0.0; 17]
                };
                if armed_before && !armed {
                    sim.targets = sim.measured()?;
                }
                armed_before = armed;
                for _ in 0..2 {
                    control::integrate(&mut sim.targets, &velocity, &sim.model.limits, DT as f64);
                    contacts.extend(sim.step(recording)?);
                }
                if sim.tick < next_preview {
                    continue;
                }
                next_preview += 20;
                if fs::read(config.output.join("recording-status.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .is_some_and(|v| v["phase"] == "error")
                {
                    recording = false;
                }
                let mode = command.as_ref().map_or("Drive", |c| c.mode.as_str());
                let status = if !fresh || rearm {
                    "Input timed out · release L1"
                } else {
                    command.as_ref().unwrap().status.as_str()
                };
                let phase = format!(
                    "{} · Gamepad · {mode} · {status}",
                    if recording {
                        "Recording"
                    } else {
                        "Preview only"
                    }
                );
                let mut frame = sim.frame(
                    if recording { record_seq } else { seq },
                    recording,
                    std::mem::take(&mut contacts),
                    velocity,
                    phase.clone(),
                )?;
                if recording {
                    send(node, "recording", &Message::Frame(frame.clone()))?;
                    record_seq += 1;
                }
                frame.sequence = seq;
                frame.sim_time_ns = frame.source_sim_time_ns;
                for c in &mut frame.cameras {
                    c.depth.clear();
                    c.seg.clear();
                }
                let elapsed = started.elapsed().as_secs_f64();
                write_json(
                    &config.output.join("progress.json"),
                    &json!({"frame":seq,"sim_time":sim.tick as f64*0.01,"wall_seconds":elapsed,"physics_hz":sim.tick as f64/elapsed,"preview_hz":(seq+1) as f64/elapsed,"phase":phase,"mode":mode,"enabled":armed,"controller_fresh":fresh,"input_sequence":command.as_ref().map(|c|c.sequence),"base_position":&frame.observation[..3],"base_quat_wxyz":&frame.observation[3..7],"joints":&frame.observation[7..24],"targets":&frame.observation[24..41],"velocity":frame.command_velocity,"recording":recording,"root_pose_writes":0}),
                )?;
                send(node, "preview_frame", &Message::Frame(frame))?;
                seq += 1;
            }
            Event::Stop(_) => break,
            Event::Error(e) => anyhow::bail!(e),
            _ => {}
        }
    }
    Ok(())
}
