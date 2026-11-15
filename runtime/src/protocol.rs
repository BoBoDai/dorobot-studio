use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const CAMERAS: [&str; 3] = ["head", "left_wrist", "right_wrist"];
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const FPS: u64 = 5;
pub const FRAME_NS: u64 = 1_000_000_000 / FPS;
pub const DT: f32 = 0.01;

/// CLOCK_MONOTONIC is shared by all Dora processes; Instant epochs are not.
pub fn now_ns() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t);
    }
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub model: PathBuf,
    pub output: PathBuf,
    pub socket: PathBuf,
    pub run_id: String,
    pub owner_pid: Option<u32>,
    pub supervisor_pid: u32,
    #[serde(default)]
    pub base: bool,
    #[serde(default)]
    pub mock: bool,
    pub port: Option<String>,
    #[serde(default)]
    pub self_test: bool,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Input {
    pub sequence: u64,
    pub input_ns: u64,
    pub connected: bool,
    pub name: String,
    pub axes: BTreeMap<String, f64>,
    pub buttons: BTreeMap<String, bool>,
}
impl Input {
    pub fn button(&self, key: &str) -> bool {
        self.buttons.get(key).copied().unwrap_or(false)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub sequence: u64,
    pub input_ns: u64,
    pub connected: bool,
    pub enabled: bool,
    pub enable_held: bool,
    pub mode: String,
    pub status: String,
    pub velocities: Vec<f64>,
    pub guarded: bool,
    pub neutral: bool,
    pub controller: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    pub size: [f32; 3],
    pub color: [f32; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sensor {
    pub name: String,
    pub rgb: Vec<u8>,
    pub depth: Vec<f32>,
    pub seg: Vec<i64>,
    pub world_from_camera_gl: [[f64; 4]; 4],
    pub world_from_mount_link: [[f64; 4]; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub sequence: u64,
    pub sim_time_ns: u64,
    pub source_sim_time_ns: u64,
    pub observation: Vec<f64>,
    pub transforms: Vec<[f32; 16]>,
    pub cameras: Vec<Sensor>,
    pub contacts: Vec<[i64; 2]>,
    pub physics_contacts: Vec<[i64; 3]>,
    pub cube_contact: bool,
    pub command_velocity: Vec<f64>,
    pub phase: String,
}
impl Frame {
    pub fn validate(&self, recorded: bool) -> Result<()> {
        ensure!(
            self.observation.len() == 41 && self.observation.iter().all(|v| v.is_finite()),
            "Invalid robot observation"
        );
        ensure!(self.command_velocity.len() == 17, "Invalid command shape");
        ensure!(
            self.cameras.len() == 3,
            "Three synchronized cameras required"
        );
        for (sensor, expected) in self.cameras.iter().zip(CAMERAS) {
            ensure!(
                sensor.name == expected && sensor.rgb.len() == WIDTH * HEIGHT * 3,
                "Invalid camera identity/RGB"
            );
            if recorded {
                ensure!(
                    sensor.depth.len() == WIDTH * HEIGHT && sensor.seg.len() == WIDTH * HEIGHT,
                    "Missing depth/segmentation"
                );
                ensure!(
                    sensor.depth.iter().all(|v| v.is_finite()),
                    "Nonfinite depth"
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    Input(Input),
    Command(Command),
    // JSON stored as UTF-8: serde_json::Value needs deserialize_any, which bincode lacks.
    Info(String),
    Frame(Frame),
    RecordStart { info: String, request: String },
    RecordStop { request: String, frame_count: u64 },
    RecordSave(String),
    RecordDiscard(String),
    Ready,
}
