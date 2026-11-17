//! Typed Arrow bridge for the experimental AISLE Nano profile.
//! Oracle object state goes only to the independent judge, never the policy.
use crate::{physics::Simulation, protocol::*, record::EpisodeCapture};
use anyhow::{Context, Result, ensure};
use dora_node_api::{DoraNode, Event, MetadataParameters, Parameter};
use serde_json::json;
use std::{fs, path::PathBuf, time::Duration};

fn env_path(key: &str) -> Result<PathBuf> {
    Ok(std::env::var_os(key)
        .with_context(|| format!("Missing {key}"))?
        .into())
}
fn params(seq: u64, stamp: u64, attempt: u64) -> MetadataParameters {
    [
        (
            "seq".into(),
            Parameter::Integer((seq * 64 + attempt) as i64),
        ),
        ("observation_id".into(), Parameter::Integer(seq as i64)),
        ("sim_time_ns".into(), Parameter::Integer(stamp as i64)),
        ("env_id".into(), Parameter::Integer(0)),
        ("h".into(), Parameter::Integer(HEIGHT as i64)),
        ("w".into(), Parameter::Integer(WIDTH as i64)),
        ("enc".into(), Parameter::String("rgb8".into())),
    ]
    .into_iter()
    .collect()
}
fn publish(node: &mut DoraNode, frame: &Frame, truth: &[f64], attempt: u64) -> Result<()> {
    let meta = params(frame.sequence, frame.source_sim_time_ns, attempt);
    node.send_output("truth".into(), meta.clone(), truth.to_vec())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // AISLE uses XYZW; the existing Studio dataset uses WXYZ.
    let mut state = frame.observation[..3].to_vec();
    state.extend_from_slice(&frame.observation[4..7]);
    state.push(frame.observation[3]);
    state.extend_from_slice(&frame.observation[7..24]);
    node.send_output(
        "state".into(),
        meta.clone(),
        state.iter().map(|x| *x as f32).collect::<Vec<_>>(),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    for c in &frame.cameras {
        node.send_output(
            format!("rgb_{}", c.name).into(),
            meta.clone(),
            c.rgb.clone(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let dropped = attempt == 0
            && c.name == "head"
            && std::env::var("AISLE_NANO_DROP_DEPTH_ONCE")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                == Some(frame.sequence);
        if !dropped {
            node.send_output(
                format!("depth_{}", c.name).into(),
                meta.clone(),
                c.depth.clone(),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        node.send_output(
            format!("pose_{}", c.name).into(),
            meta.clone(),
            c.world_from_camera_gl
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}
fn valid_command(q: &[f32], seq: i64, expected: u64) -> Result<()> {
    ensure!(
        seq >= 0 && seq as u64 == expected,
        "Unexpected command sequence"
    );
    ensure!(
        q.len() == 17 && q.iter().all(|x| x.is_finite()),
        "Invalid command"
    );
    ensure!(
        q.iter()
            .enumerate()
            .all(|(i, v)| v.abs() <= if i < 3 { 5.001 } else { 0.501 }),
        "Command exceeds guard envelope"
    );
    ensure!(
        !q[5..].iter().any(|v| v.abs() > 1e-5) || q[..3].iter().all(|v| v.abs() < 1e-5),
        "Base/arm interlock"
    );
    Ok(())
}
fn valid_receipt(values: &[u64], expected: u64) -> Result<()> {
    ensure!(values == [expected], "Invalid observation receipt");
    Ok(())
}
fn reset_wheel_targets(targets: &mut [f64], measured: &[f64]) {
    // A stopped/slipping wheel must not retain a backlog of position commands.
    targets[..3].copy_from_slice(&measured[..3]);
}
pub fn run() -> Result<()> {
    let output = env_path("AISLE_NANO_OUTPUT")?;
    let scene = env_path("AISLE_NANO_SCENE")?;
    let model = env_path("AISLE_NANO_MODEL")?;
    let side = std::env::var("AISLE_NANO_ARM").unwrap_or_else(|_| "right".into());
    ensure!(matches!(side.as_str(), "left" | "right"), "Invalid arm");
    let (mut node, mut events) = DoraNode::init_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut sim = Simulation::with_scene(&model, Some(&scene))?;
    let mut info = json!({"version":1,"session":"AISLE Nano Rust indoor","fps":FPS,"joint_names":sim.model.names,"objects":sim.objects,"cameras":sim.camera_specs,"recording":true,"recording_kind":"AISLE autonomous simulation; see aisle-result.json for independent task evaluation","physics_dt":DT,"mode":"simulation","studio_session_version":1,"initial_objects":sim.initial_objects,"runtime":"rust","controller":"AISLE RGB/depth + URDF IK","contact_audit_hz":100});
    info["simulation_settings"] = json!({"physics_hz":1.0/DT,"capture_hz":FPS,"actuators":sim.actuator_settings(),"arm_target_lead_rad":0.10,"wheel_target_reset":"measured angle each control observation","self_collision":false,"contact_tolerance_m":0.0015,"object_attachments":false,"root_pose_writes_during_rollout":0});
    write_json(&output.join("scene-info.json"), &info)?;
    let mut capture = EpisodeCapture::new(
        &output,
        info.clone(),
        "AISLE complex indoor navigation and RGB cube pickup",
    )?;
    let preview = crate::preview::Preview::new(&output.join("preview.sock"))?;
    let cube = sim
        .objects
        .iter()
        .position(|o| o.name == "cube_6")
        .context("Target cube")?;
    let mut seq = 0;
    let mut velocity = vec![0.; 17];
    let mut contacts = Vec::new();
    let mut retries = 0u64;
    let mut terminal = false;
    loop {
        let mut frame = sim.frame(
            seq,
            true,
            contacts,
            velocity.clone(),
            "AISLE autonomous control".into(),
        )?;
        frame.source_sim_time_ns = sim.tick * 10_000_000;
        let stamp = frame.source_sim_time_ns;
        let both = sim.gripper_contact(&side, &frame.physics_contacts);
        let t = frame.transforms[cube];
        let truth = vec![
            frame.observation[0],
            frame.observation[1],
            t[12] as f64,
            t[13] as f64,
            t[14] as f64,
            if both { 1. } else { 0. },
            if frame.contacts.is_empty() { 0. } else { 1. },
            if frame.cube_contact { 1. } else { 0. },
        ];
        publish(&mut node, &frame, &truth, 0)?;
        if seq % 10 == 0 {
            for c in &frame.cameras {
                image::save_buffer(
                    output.join(format!("latest-{}.png", c.name)),
                    &c.rgb,
                    WIDTH as u32,
                    HEIGHT as u32,
                    image::ColorType::Rgb8,
                )?;
            }
        }
        preview.publish(&info, &frame)?;
        write_json(
            &output.join("progress.json"),
            &json!({"frame":seq,"sim_time_ns":stamp,"base":&frame.observation[..7],"joints":&frame.observation[7..24],"cube_truth":&truth_placeholder(t),"both_fingers":both}),
        )?;
        capture.append(frame.clone())?;
        let mut stop = false;
        let mut commanded = false;
        let mut judged = false;
        let mut attempt = 0;
        loop {
            match events.recv_timeout(Duration::from_secs(1)) {
                Some(Event::Input { id, data, metadata }) => {
                    let actual = match metadata.parameters.get("observation_id") {
                        Some(Parameter::Integer(n)) => *n,
                        _ => -1,
                    };
                    if actual >= 0 && (actual as u64) < seq {
                        continue;
                    }
                    ensure!(
                        actual >= 0 && actual as u64 == seq,
                        "Unexpected observation acknowledgement"
                    );
                    ensure!(
                        metadata.parameters.get("sim_time_ns")
                            == Some(&Parameter::Integer(stamp as i64)),
                        "Stale command stamp"
                    );
                    ensure!(
                        metadata.parameters.get("env_id") == Some(&Parameter::Integer(0)),
                        "Invalid environment receipt"
                    );
                    match id.as_str() {
                        "command" => {
                            let values: &[f32] =
                                (&data).try_into().map_err(|e| anyhow::anyhow!("{e}"))?;
                            valid_command(values, actual, seq)?;
                            let received: Vec<f64> = values.iter().map(|x| *x as f64).collect();
                            ensure!(
                                !commanded || velocity == received,
                                "Changed duplicate command"
                            );
                            velocity = received;
                            commanded = true;
                        }
                        "judge_ack" | "done" => {
                            let values: &[u64] =
                                (&data).try_into().map_err(|e| anyhow::anyhow!("{e}"))?;
                            valid_receipt(values, seq)?;
                            if id.as_str() == "judge_ack" {
                                judged = true;
                            } else {
                                stop = true;
                            }
                        }
                        _ => anyhow::bail!("Unknown input {id}"),
                    }
                    if judged && (commanded || stop) {
                        terminal = stop;
                        break;
                    }
                }
                Some(Event::Error(error)) if error.starts_with("Timeout event stream error") => {
                    attempt += 1;
                    retries += 1;
                    if attempt >= 30 {
                        write_json(
                            &output.join("transport-failure.json"),
                            &json!({"reason":"no_sim_clock","observation_id":seq,"commanded":commanded,"judged":judged,"wall_timeout_s":30}),
                        )?;
                        stop = true;
                        break;
                    }
                    // Retransmit the immutable observation. Neither physics nor
                    // recording advances until guard and judge acknowledge it.
                    publish(&mut node, &frame, &truth, attempt)?;
                }
                Some(Event::Error(error)) => anyhow::bail!("Dora input error: {error}"),
                Some(Event::Stop { .. }) | None => {
                    stop = true;
                    break;
                }
                _ => {}
            }
        }
        if stop {
            break;
        }
        contacts = Vec::new();
        let measured = sim.measured()?;
        reset_wheel_targets(&mut sim.targets, &measured);
        for _ in 0..20 {
            for (i, v) in velocity.iter().enumerate() {
                let target = sim.targets[i] + v * DT as f64;
                // Stop position-target windup against a contacted object.
                sim.targets[i] = if i >= 3 {
                    target.clamp(measured[i] - 0.10, measured[i] + 0.10)
                } else {
                    target
                };
                sim.targets[i] =
                    sim.targets[i].clamp(sim.model.limits[i][0], sim.model.limits[i][1]);
            }
            contacts.extend(sim.step(true)?);
        }
        seq += 1;
    }
    let dataset = capture.finish(&output)?;
    write_json(
        &output.join("capture-complete.json"),
        &json!({"ok":true,"frames":seq+1,"dataset":dataset,"cameras":CAMERAS,"delivery_retries":retries,"terminal_acknowledged":terminal}),
    )?;
    if terminal {
        node.send_output(
            "finished".into(),
            params(seq, sim.tick * 10_000_000, 63),
            vec![seq],
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    fs::remove_file(output.join("preview.sock")).ok();
    Ok(())
}
fn truth_placeholder(t: [f32; 16]) -> [f32; 3] {
    [t[12], t[13], t[14]]
}

#[cfg(test)]
mod tests {
    use super::*;
    /// CON-4, BG-1/2, MOB-3: bridge rejects malformed, replayed or bypassed guard data.
    #[test]
    fn rejects_unsafe_arrow_command() {
        let mut q = vec![0.; 17];
        assert!(valid_command(&q, 0, 0).is_ok());
        assert!(valid_command(&q, 0, 1).is_err());
        q[5] = 0.3;
        q[0] = 1.;
        assert!(valid_command(&q, 0, 0).is_err());
        q[0] = 0.;
        q[5] = f32::NAN;
        assert!(valid_command(&q, 0, 0).is_err());
        assert!(valid_command(&[0.; 16], 0, 0).is_err());
    }
    /// CON-4: a receipt must name exactly the in-flight observation.
    #[test]
    fn rejects_malformed_receipts() {
        assert!(valid_receipt(&[4], 4).is_ok());
        assert!(valid_receipt(&[], 4).is_err());
        assert!(valid_receipt(&[3], 4).is_err());
        assert!(valid_receipt(&[4, 4], 4).is_err());
    }
    /// MOB-3: zero velocity after a stalled wheel removes stored position drive.
    #[test]
    fn stopped_wheels_do_not_replay_a_stalled_target() {
        let mut targets = vec![10.; 17];
        let measured = vec![0.25; 17];
        reset_wheel_targets(&mut targets, &measured);
        assert_eq!(&targets[..3], &[0.25; 3]);
        assert_eq!(targets[5], 10.);
    }
}
