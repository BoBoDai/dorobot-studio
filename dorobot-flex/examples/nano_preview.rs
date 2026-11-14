//! Generate explicitly synthetic Nano playback clips using Studio's dataset writer.
use dorobot_flex::api::{files::FileBackend, Backend};
use dorobot_flex::data::lerobot_writer::{DatasetSpec, DatasetWriter, FrameRecord};
use std::{f32::consts::PI, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args().nth(1).expect("output dataset"));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("data/adora_nano/adora_nano.urdf");
    let robot = urdf_rs::read_file(&source)?;
    let joints: Vec<_> = robot
        .joints
        .iter()
        .filter(|j| j.joint_type != urdf_rs::JointType::Fixed)
        .collect();
    let names: Vec<_> = joints.iter().map(|j| j.name.clone()).collect();
    let mut writer = DatasetWriter::create(
        &root,
        DatasetSpec {
            robot_type: "adora_nano".into(),
            fps: 30.0,
            camera_keys: vec![],
            joint_names: names.clone(),
        },
    )?;
    for (label, frames, kind) in [
        (
            "SYNTHETIC PREVIEW - arm joints and grippers; radians; no hardware",
            720,
            0,
        ),
        (
            "SYNTHETIC PREVIEW - head and wheel rotation; fixed base; no physics",
            360,
            1,
        ),
        ("SYNTHETIC PREVIEW - CAD zero pose; no hardware", 90, 2),
    ] {
        let mut episode = writer.begin_episode(label)?;
        for frame in 0..frames {
            let mut state = vec![0.0_f32; names.len()];
            let t = frame as f32 / 30.0;
            for (i, joint) in joints.iter().enumerate() {
                let name = &joint.name;
                if kind == 0 {
                    for k in 1..=6 {
                        if name.contains(&format!("_j{k}_")) {
                            let phase = (t - (k - 1) as f32 * 4.0) / 4.0;
                            if phase > 0.0 && phase < 1.0 {
                                state[i] =
                                    (PI * phase).sin().powi(2) * if k == 6 { 0.65 } else { 0.3 };
                            }
                        }
                    }
                } else if kind == 1 {
                    let pulse = (PI * t / 12.0).sin().powi(2);
                    if name.starts_with("head_") {
                        state[i] = 0.3 * pulse;
                    }
                    if name.starts_with("wheel_") {
                        state[i] = 2.0 * PI * pulse;
                    }
                }
                if joint.joint_type != urdf_rs::JointType::Continuous {
                    assert!(
                        (state[i] as f64) >= joint.limit.lower
                            && (state[i] as f64) <= joint.limit.upper
                    );
                }
            }
            episode.push(FrameRecord {
                action: state.clone(),
                state,
            })?;
        }
        println!("episode {}: {frames} frames", writer.commit(episode)?);
    }
    drop(writer);
    std::fs::write(root.join("PREVIEW.txt"), "Synthetic kinematic preview of Adora Nano CAD revision.\nNot captured sensor data, a physics simulation, or a hardware command sequence.\nAll joint values are radians in movable-joint URDF order. Base pose stays fixed.\nThree clips: arm/gripper sweep, head/wheel sweep, zero pose. No camera video.\n")?;
    std::fs::write(
        root.join("meta/preview-provenance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind":"synthetic_kinematic_preview","source_urdf":source,"units":"radians",
            "joint_order":names,"hardware_connected":false,"physics_simulated":false
        }))?,
    )?;
    let backend = FileBackend::with_open(&root, None);
    let playback = backend.playback();
    assert_eq!(playback.joint_frames.len(), 720);
    assert_eq!(playback.joint_frames[0].len(), 17);
    assert!(playback.robot_urdf.is_some());
    println!(
        "Verified 3 clips, 17 channels; model {:?}",
        playback.robot_urdf
    );
    Ok(())
}
