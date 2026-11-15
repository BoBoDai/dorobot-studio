//! Native SO-100/101 leader/follower workflow. Register definitions and normalized
//! joint units follow Hugging Face LeRobot v0.4.3 (Apache-2.0).
use crate::{
    base::packet,
    protocol::{now_ns, write_json},
    video::{Camera, Encoder},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
const JOINTS: [&str; 6] = [
    "shoulder_pan",
    "shoulder_lift",
    "elbow_flex",
    "wrist_flex",
    "wrist_roll",
    "gripper",
];
#[derive(Clone, Deserialize, Serialize)]
struct Calibration {
    id: u8,
    drive_mode: u8,
    homing_offset: i32,
    range_min: i32,
    range_max: i32,
}
impl Calibration {
    fn validate(&self, id: u8) -> Result<()> {
        ensure!(
            self.id == id
                && self.drive_mode <= 1
                && self.homing_offset.abs() <= 2047
                && self.range_min >= 0
                && self.range_max <= 4095
                && self.range_max > self.range_min,
            "Invalid motor calibration"
        );
        Ok(())
    }
    fn normalize(&self, raw: i32) -> f64 {
        let p = (raw.clamp(self.range_min, self.range_max) - self.range_min) as f64
            / (self.range_max - self.range_min) as f64;
        let p = if self.drive_mode != 0 { 1.0 - p } else { p };
        if self.id == 6 {
            p * 100.0
        } else {
            p * 200.0 - 100.0
        }
    }
    fn raw(&self, value: f64) -> Result<u16> {
        ensure!(value.is_finite(), "Invalid joint target");
        let p = if self.id == 6 {
            value.clamp(0.0, 100.0) / 100.0
        } else {
            (value.clamp(-100.0, 100.0) + 100.0) / 200.0
        };
        let p = if self.drive_mode != 0 { 1.0 - p } else { p };
        Ok((self.range_min as f64 + p * (self.range_max - self.range_min) as f64) as u16)
    }
}
struct Arm {
    port: Box<dyn serialport::SerialPort>,
    cal: Vec<Calibration>,
}
impl Arm {
    fn open(port: &str) -> Result<Self> {
        let mut arm = Self {
            port: serialport::new(port, 1_000_000)
                .timeout(Duration::from_millis(20))
                .open()?,
            cal: Vec::new(),
        };
        for id in 1..=6 {
            ensure!(arm.read(id, 3, 2)? == 777, "Motor {id} is not an STS3215");
        }
        arm.disable()?;
        Ok(arm)
    }
    fn exchange(
        &mut self,
        id: u8,
        instruction: u8,
        params: &[u8],
        count: usize,
    ) -> Result<Vec<u8>> {
        self.port.clear(serialport::ClearBuffer::Input)?;
        self.port.write_all(&packet(id, instruction, params))?;
        let mut response = vec![0; count + 6];
        self.port.read_exact(&mut response)?;
        ensure!(
            response[..4] == [255, 255, id, (count + 2) as u8]
                && response[2..].iter().fold(0u8, |s, v| s.wrapping_add(*v)) == 255
                && response[4] == 0,
            "Motor {id} invalid acknowledgement"
        );
        Ok(response[5..5 + count].to_vec())
    }
    fn read(&mut self, id: u8, reg: u8, len: u8) -> Result<u16> {
        let bytes = self.exchange(id, 2, &[reg, len], len as usize)?;
        Ok(if len == 2 {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            bytes[0] as u16
        })
    }
    fn write(&mut self, id: u8, reg: u8, value: u16, len: usize) -> Result<()> {
        let mut data = vec![reg];
        data.extend(&value.to_le_bytes()[..len]);
        self.exchange(id, 3, &data, 0)?;
        Ok(())
    }
    fn disable(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for id in 1..=6 {
            for (reg, value) in [(40, 0), (55, 0)] {
                if let Err(e) = self.write(id, reg, value, 1) {
                    errors.push(e.to_string());
                }
            }
        }
        ensure!(
            errors.is_empty(),
            "Torque disable failed: {}",
            errors.join("; ")
        );
        Ok(())
    }
    fn load_calibration(&mut self, path: &Path) -> Result<()> {
        let saved: BTreeMap<String, Calibration> =
            serde_json::from_slice(&fs::read(path).context(
                "Calibration is missing. Calibrate leader and follower in Robot setup first.",
            )?)?;
        let mut cal = Vec::new();
        for (index, name) in JOINTS.iter().enumerate() {
            let id = index as u8 + 1;
            let c = saved.get(*name).context("Incomplete calibration")?.clone();
            c.validate(id)?;
            let raw = self.read(id, 31, 2)?;
            let offset = (raw & 0x7ff) as i32 * if raw & 0x800 != 0 { -1 } else { 1 };
            ensure!(
                offset == c.homing_offset
                    && self.read(id, 9, 2)? == c.range_min as u16
                    && self.read(id, 11, 2)? == c.range_max as u16,
                "Motor calibration does not match this rig. Recalibrate before recording."
            );
            cal.push(c);
        }
        self.cal = cal;
        Ok(())
    }
    fn positions(&mut self) -> Result<Vec<f64>> {
        (1..=6)
            .map(|id| {
                let raw = self.read(id, 56, 2)?;
                let pos = (raw & 0x7fff) as i32 * if raw & 0x8000 != 0 { -1 } else { 1 };
                Ok(self.cal[id as usize - 1].normalize(pos))
            })
            .collect()
    }
    fn configure(&mut self, follower: bool) -> Result<()> {
        self.disable()?;
        for id in 1..=6 {
            self.write(id, 7, 0, 1)?;
            self.write(id, 41, 254, 1)?;
            self.write(id, 33, 0, 1)?;
            for (reg, value) in [(21, 16), (22, 32), (23, 0)] {
                self.write(id, reg, value, 1)?;
            }
        }
        if follower {
            self.write(6, 16, 500, 2)?;
            self.write(6, 28, 250, 2)?;
            self.write(6, 36, 25, 1)?;
            let current = self.positions()?;
            self.targets(&current)?;
            for id in 1..=6 {
                self.write(id, 40, 1, 1)?;
                self.write(id, 55, 1, 1)?;
            }
        }
        Ok(())
    }
    fn targets(&mut self, values: &[f64]) -> Result<()> {
        ensure!(values.len() == 6, "Expected 6 targets");
        for (i, v) in values.iter().enumerate() {
            let raw = self.cal[i].raw(*v)?;
            self.write(i as u8 + 1, 42, raw, 2)?;
        }
        Ok(())
    }
}
impl Drop for Arm {
    fn drop(&mut self) {
        if let Err(e) = self.disable() {
            eprintln!("{e:#}");
        }
    }
}
fn emit(value: Value) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
fn field<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("Rig requires {key}"))
}
fn calibration_path(p: &Value, leader: bool) -> Result<PathBuf> {
    let id = field(p, if leader { "leader_id" } else { "follower_id" })?;
    ensure!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "Use letters, numbers, underscores or hyphens in rig IDs"
    );
    let robot = field(p, "robot_type")?;
    let root = std::env::var_os("HF_LEROBOT_CALIBRATION")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".cache/huggingface/lerobot/calibration")
        });
    Ok(root
        .join(if leader { "teleoperators" } else { "robots" })
        .join(format!(
            "{robot}_{}",
            if leader { "leader" } else { "follower" }
        ))
        .join(format!("{id}.json")))
}
fn wait_enter(rx: &mpsc::Receiver<Option<String>>, stop: &AtomicBool) -> Result<()> {
    loop {
        ensure!(!stop.load(Ordering::Acquire), "Calibration cancelled");
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(s)) if s.trim() != "stop" => return Ok(()),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Calibration cancelled")
            }
            _ => {}
        }
    }
}
fn calibrate(p: &Value, rx: &mpsc::Receiver<Option<String>>, stop: &AtomicBool) -> Result<()> {
    for leader in [true, false] {
        let name = if leader { "leader" } else { "follower" };
        let mut arm = Arm::open(field(
            p,
            if leader {
                "leader_port"
            } else {
                "follower_port"
            },
        )?)?;
        emit(
            json!({"event":"prompt","message":format!("Place the {name} arm in its middle position. Torque is off. Press Continue when ready.")}),
        )?;
        wait_enter(rx, stop)?;
        let mut offsets = Vec::new();
        for id in 1..=6 {
            arm.write(id, 33, 0, 1)?;
            arm.write(id, 31, 0, 2)?;
            arm.write(id, 9, 0, 2)?;
            arm.write(id, 11, 4095, 2)?;
            let pos = arm.read(id, 56, 2)? as i32;
            let offset = pos - 2047;
            ensure!(
                offset.abs() <= 2047,
                "Motor position is outside the calibration range"
            );
            arm.write(
                id,
                31,
                (offset.unsigned_abs() as u16) | if offset < 0 { 0x800 } else { 0 },
                2,
            )?;
            offsets.push(offset);
        }
        emit(
            json!({"event":"prompt","message":format!("Move each {name} joint gently through its full range, including the gripper. Wrist roll uses its full turn. Press Continue to save.")}),
        )?;
        let mut low = [4095; 6];
        let mut high = [0; 6];
        loop {
            ensure!(!stop.load(Ordering::Acquire), "Calibration cancelled");
            match rx.try_recv() {
                Ok(Some(s)) if s.trim() != "stop" => break,
                Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("Calibration cancelled")
                }
                _ => {}
            }
            for id in 1..=6 {
                let p = arm.read(id, 56, 2)? as i32;
                ensure!(
                    (0..=4095).contains(&p),
                    "Calibration crossed encoder wrap; restart from middle position"
                );
                low[id as usize - 1] = low[id as usize - 1].min(p);
                high[id as usize - 1] = high[id as usize - 1].max(p);
            }
            thread::sleep(Duration::from_millis(20));
        }
        low[4] = 0;
        high[4] = 4095;
        let mut saved = BTreeMap::new();
        for (i, joint) in JOINTS.iter().enumerate() {
            let id = i as u8 + 1;
            let c = Calibration {
                id,
                drive_mode: 0,
                homing_offset: offsets[i],
                range_min: low[i],
                range_max: high[i],
            };
            c.validate(id)?;
            ensure!(
                high[i] - low[i] >= 32,
                "Joint {joint} did not move enough; repeat calibration"
            );
            arm.write(id, 9, low[i] as u16, 2)?;
            arm.write(id, 11, high[i] as u16, 2)?;
            saved.insert(joint.to_string(), c);
        }
        let path = calibration_path(p, leader)?;
        fs::create_dir_all(path.parent().unwrap())?;
        write_json(&path, &saved)?;
        arm.load_calibration(&path)?;
    }
    emit(json!({"event":"calibrated"}))
}
#[derive(Default)]
struct Stats {
    min: [f64; 3],
    max: [f64; 3],
    sum: [f64; 3],
    sum_sq: [f64; 3],
    pixels: u64,
}
impl Stats {
    fn add(&mut self, rgb: &[u8]) {
        if self.pixels == 0 {
            self.min = [1.0; 3];
        }
        for y in (0..480).step_by(8) {
            for x in (0..640).step_by(8) {
                for c in 0..3 {
                    let v = rgb[(y * 640 + x) * 3 + c] as f64 / 255.0;
                    self.min[c] = self.min[c].min(v);
                    self.max[c] = self.max[c].max(v);
                    self.sum[c] += v;
                    self.sum_sq[c] += v * v;
                }
                self.pixels += 1;
            }
        }
    }
    fn json(&self, count: u64) -> Value {
        let mean = self.sum.map(|v| v / self.pixels as f64);
        let std = std::array::from_fn::<_, 3, _>(|i| {
            (self.sum_sq[i] / self.pixels as f64 - mean[i] * mean[i])
                .max(0.0)
                .sqrt()
        });
        json!({"min":self.min.map(|v|[[v]]),"max":self.max.map(|v|[[v]]),"mean":mean.map(|v|[[v]]),"std":std.map(|v|[[v]]),"count":[count]})
    }
}
fn record(
    p: &Value,
    staging: &Path,
    rx: &mpsc::Receiver<Option<String>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    fs::create_dir_all(staging)?;
    let fps = p["fps"].as_f64().unwrap_or(30.0);
    let limit = p["max_relative_target"].as_f64().unwrap_or(5.0);
    let seconds = p["max_episode_seconds"].as_f64().unwrap_or(300.0);
    ensure!(
        limit.is_finite()
            && limit > 0.0
            && seconds.is_finite()
            && (0.0..=3600.0).contains(&seconds),
        "Invalid recording limits"
    );
    let mut leader = Arm::open(field(p, "leader_port")?)?;
    leader.load_calibration(&calibration_path(p, true)?)?;
    leader.configure(false)?;
    let mut follower = Arm::open(field(p, "follower_port")?)?;
    follower.load_calibration(&calibration_path(p, false)?)?;
    let mut cameras = Vec::new();
    let mut videos = BTreeMap::new();
    if let Some(specs) = p["cameras"].as_object() {
        for (name, source) in specs {
            ensure!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "Invalid camera name"
            );
            let camera = Camera::open(source, fps)?;
            let path = staging.join(format!("{name}.mp4"));
            let writer = Encoder::new(&path, 640, 480, fps)?;
            videos.insert(format!("observation.images.{name}"), path);
            cameras.push((name.clone(), camera, writer, Stats::default()));
        }
    }
    follower.configure(true)?;
    let follower = Arc::new(Mutex::new(follower));
    let last = Arc::new(AtomicU64::new(now_ns()));
    let fault = Arc::new(AtomicBool::new(false));
    let (arm, clock, shutdown, failed) =
        (follower.clone(), last.clone(), stop.clone(), fault.clone());
    let watchdog = thread::spawn(move || {
        while !shutdown.load(Ordering::Acquire) {
            if now_ns().saturating_sub(clock.load(Ordering::Acquire)) > 250_000_000 {
                failed.store(true, Ordering::Release);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        arm.lock().unwrap().disable()
    });
    let result = (|| -> Result<(u64, BTreeMap<String, Value>)> {
        emit(json!({"event":"ready","joints":JOINTS.map(|n|format!("{n}.pos"))}))?;
        let mut frames = File::create_new(staging.join("frames.jsonl"))?;
        let started = Instant::now();
        let mut count = 0;
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match rx.try_recv() {
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => break,
                Ok(Some(s)) if s.trim() == "stop" => break,
                _ => {}
            }
            ensure!(
                !fault.load(Ordering::Acquire),
                "Capture input timed out; follower torque disabled"
            );
            let requested = leader.positions()?;
            let (state, action) = {
                let mut follower = follower.lock().unwrap();
                ensure!(!fault.load(Ordering::Acquire), "Follower watchdog stopped");
                let state = follower.positions()?;
                let action: Vec<_> = requested
                    .iter()
                    .zip(&state)
                    .map(|(v, p)| v.clamp(p - limit, p + limit))
                    .collect();
                follower.targets(&action)?;
                (state, action)
            };
            last.store(now_ns(), Ordering::Release);
            let timestamp = started.elapsed().as_secs_f64();
            let mut previews = BTreeMap::new();
            for (name, camera, writer, stats) in &mut cameras {
                let rgb = camera.rgb()?;
                writer.append(&rgb)?;
                stats.add(&rgb);
                if count % 6 == 0 {
                    let temp = staging.join(format!("{name}.tmp.png"));
                    image::save_buffer_with_format(
                        &temp,
                        &rgb,
                        640,
                        480,
                        image::ColorType::Rgb8,
                        image::ImageFormat::Png,
                    )?;
                    fs::rename(temp, staging.join(format!("{name}.png")))?;
                }
                previews.insert(name.clone(), staging.join(format!("{name}.png")));
            }
            let frame = json!({"state":state,"action":action,"timestamp":timestamp});
            writeln!(frames, "{frame}")?;
            count += 1;
            if (count - 1) % 6 == 0 {
                frames.flush()?;
                emit(
                    json!({"event":"frame","state":state,"action":action,"timestamp":timestamp,"count":count,"previews":previews}),
                )?;
            }
            if timestamp >= seconds {
                break;
            }
            let deadline = Duration::from_secs_f64(count as f64 / fps);
            if deadline > started.elapsed() {
                thread::sleep(deadline - started.elapsed());
            }
        }
        frames.sync_all()?;
        ensure!(count > 0, "No frames captured");
        let mut stats = BTreeMap::new();
        for (name, _, writer, s) in cameras {
            ensure!(writer.finish()? == count, "Camera frame count mismatch");
            stats.insert(format!("observation.images.{name}"), s.json(count));
        }
        Ok((count, stats))
    })();
    stop.store(true, Ordering::Release);
    let shutdown = watchdog
        .join()
        .map_err(|_| anyhow::anyhow!("Follower watchdog panicked"))?;
    shutdown?;
    let (count, stats) = result?;
    emit(
        json!({"event":"take","frames":staging.join("frames.jsonl"),"videos":videos,"count":count,"camera_stats":stats}),
    )
}
pub fn run(mode: &str, profile: &Path, staging: &Path) -> Result<()> {
    let result = run_inner(mode, profile, staging);
    if let Err(e) = &result {
        emit(json!({"event":"error","message":format!("{e:#}")}))?;
    }
    result
}
fn run_inner(mode: &str, profile: &Path, staging: &Path) -> Result<()> {
    let p: Value = serde_json::from_slice(&fs::read(profile)?)?;
    ensure!(
        ["so100", "so101"].contains(&field(&p, "robot_type")?),
        "Choose SO-100 or SO-101"
    );
    for key in [
        "task",
        "dataset",
        "follower_port",
        "leader_port",
        "follower_id",
        "leader_id",
    ] {
        field(&p, key)?;
    }
    ensure!(
        p["leader_port"] != p["follower_port"],
        "Leader and follower need different ports"
    );
    let fps = p["fps"].as_f64().unwrap_or(30.0);
    ensure!(
        fps.is_finite() && (1.0..=60.0).contains(&fps),
        "FPS must be 1–60"
    );
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::Release))?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(s) => {
                    if tx.send(Some(s)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    let result = match mode {
        "calibrate" => calibrate(&p, &rx, &stop),
        "record" => record(&p, staging, &rx, stop),
        _ => Err(anyhow::anyhow!("Unknown capture mode")),
    };
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_units_match_lerobot() {
        for id in [1, 6] {
            for drive_mode in [0, 1] {
                let c = Calibration {
                    id,
                    drive_mode,
                    homing_offset: 0,
                    range_min: 500,
                    range_max: 3500,
                };
                for raw in [500, 2000, 3500] {
                    assert!((c.raw(c.normalize(raw)).unwrap() as i32 - raw).abs() <= 1);
                }
                assert!(c.raw(f64::NAN).is_err());
            }
        }
    }
}
