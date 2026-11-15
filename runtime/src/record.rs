use crate::{protocol::*, video::Encoder};
use anyhow::{Context, Result, ensure};
use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, Float64Array, Int64Array, RecordBatch,
};
use arrow_schema::{DataType, Field};
use ndarray::{Array1, Array2, Array3, arr0};
use ndarray_npy::NpzWriter;
use parquet::{arrow::ArrowWriter, basic::Compression, file::properties::WriterProperties};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

fn parquet(path: &Path, columns: Vec<(&str, ArrayRef)>) -> Result<()> {
    fs::create_dir_all(path.parent().context("Parquet directory")?)?;
    let batch = RecordBatch::try_from_iter(columns)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(path)?, batch.schema(), Some(props))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}
fn floats(values: Vec<f64>, size: i32) -> Result<ArrayRef> {
    Ok(Arc::new(FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float64, true)),
        size,
        Arc::new(Float64Array::from(values)),
        None,
    )?))
}
fn states(values: Vec<f32>) -> Result<ArrayRef> {
    Ok(Arc::new(FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        17,
        Arc::new(Float32Array::from(values)),
        None,
    )?))
}
fn jsonl(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, format!("{value}\n"))?;
    Ok(())
}
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        ensure!(!ty.is_symlink(), "Dataset symlinks are not allowed");
        let dest = target.join(entry.file_name());
        if ty.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}
struct Row {
    observation: Vec<f64>,
    command: Vec<f64>,
    stamp: u64,
}
struct Take {
    path: PathBuf,
    info: Value,
    task: String,
    writers: Vec<Encoder>,
    audit: File,
    rows: Vec<Row>,
    scene: Vec<Value>,
    bad: Vec<Value>,
    cube_frames: u64,
    started: Instant,
}
impl Take {
    fn new(root: &Path, info: Value, request: &Value) -> Result<Self> {
        let task = request["task"].as_str().unwrap_or("").trim().to_owned();
        ensure!(
            !task.is_empty(),
            "Enter a task description before recording."
        );
        let id = request["id"].as_u64().context("Invalid take ID")?;
        let path = root.join("takes").join(id.to_string());
        fs::create_dir_all(path.parent().unwrap())?;
        fs::create_dir(&path)?;
        fs::create_dir(path.join("raw"))?;
        write_json(&path.join("scene-info.json"), &info)?;
        let mut writers = Vec::new();
        for name in CAMERAS {
            let dir = path.join(format!(
                "lerobot/videos/chunk-000/observation.images.{name}"
            ));
            fs::create_dir_all(&dir)?;
            writers.push(Encoder::new(
                &dir.join("episode_000000.mp4"),
                WIDTH,
                HEIGHT,
                FPS as f64,
            )?);
        }
        let audit = File::create(path.join("raw/frames.jsonl"))?;
        Ok(Self {
            path,
            info,
            task,
            writers,
            audit,
            rows: Vec::new(),
            scene: Vec::new(),
            bad: Vec::new(),
            cube_frames: 0,
            started: Instant::now(),
        })
    }
    fn append(&mut self, frame: Frame) -> Result<()> {
        frame.validate(true)?;
        ensure!(
            frame.sequence == self.rows.len() as u64,
            "Recording frame order mismatch"
        );
        ensure!(
            frame.sim_time_ns == (frame.sequence + 1) * FRAME_NS,
            "Camera clock mismatch"
        );
        let path = self
            .path
            .join(format!("raw/frame_{:06}.npz", frame.sequence));
        let mut npz = NpzWriter::new_compressed(File::create(&path)?);
        npz.add_array("observation", &Array1::from_vec(frame.observation.clone()))?;
        npz.add_array(
            "transforms",
            &Array2::from_shape_vec(
                (frame.transforms.len(), 16),
                frame.transforms.iter().flatten().copied().collect(),
            )?,
        )?;
        npz.add_array(
            "contacts",
            &Array2::from_shape_vec(
                (frame.contacts.len(), 2),
                frame.contacts.iter().flatten().copied().collect(),
            )?,
        )?;
        npz.add_array(
            "physics_contacts",
            &Array2::from_shape_vec(
                (frame.physics_contacts.len(), 3),
                frame.physics_contacts.iter().flatten().copied().collect(),
            )?,
        )?;
        npz.add_array(
            "cube_contact",
            &Array1::from_vec(vec![i64::from(frame.cube_contact)]),
        )?;
        npz.add_array(
            "command_velocity",
            &Array1::from_vec(frame.command_velocity.clone()),
        )?;
        for name in ["command", "proposed"] {
            npz.add_array(
                name,
                &Array1::from_vec(frame.command_velocity[..3].to_vec()),
            )?;
        }
        npz.add_array("source_sim_time_ns", &arr0(frame.source_sim_time_ns as i64))?;
        npz.add_array("sim_time_ns", &arr0(frame.sim_time_ns as i64))?;
        for c in &frame.cameras {
            npz.add_array(
                format!("{}__rgb", c.name),
                &Array3::from_shape_vec((HEIGHT, WIDTH, 3), c.rgb.clone())?,
            )?;
            npz.add_array(
                format!("{}__depth", c.name),
                &Array2::from_shape_vec((HEIGHT, WIDTH), c.depth.clone())?,
            )?;
            npz.add_array(
                format!("{}__seg", c.name),
                &Array2::from_shape_vec((HEIGHT, WIDTH), c.seg.clone())?,
            )?;
            for (name, m) in [
                ("world_from_camera_gl", c.world_from_camera_gl),
                ("world_from_mount_link", c.world_from_mount_link),
            ] {
                npz.add_array(
                    format!("{}__{name}", c.name),
                    &Array2::from_shape_vec((4, 4), m.into_iter().flatten().collect())?,
                )?;
            }
            npz.add_array(
                format!("{}__sim_time_ns", c.name),
                &arr0(frame.sim_time_ns as i64),
            )?;
        }
        npz.finish()?.sync_all()?;
        let mut source = File::open(&path)?;
        let mut hash = Sha256::new();
        let mut buf = [0; 65536];
        loop {
            let n = source.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        writeln!(
            self.audit,
            "{}",
            json!({"sequence":frame.sequence,"sim_time_ns":frame.sim_time_ns,"sha256":format!("{:x}",hash.finalize()),"pids":{},"phase":frame.phase,"goal_index":0})
        )?;
        self.audit.flush()?;
        for (writer, c) in self.writers.iter_mut().zip(&frame.cameras) {
            writer.append(&c.rgb)?;
        }
        self.cube_frames += u64::from(frame.cube_contact);
        if !frame.contacts.is_empty() {
            self.bad
                .push(json!({"frame":frame.sequence,"pairs":frame.contacts}));
        }
        let obs = &frame.observation;
        self.scene.push(json!({"timestamp":frame.sequence as f64/FPS as f64,"sim_time_ns":frame.sim_time_ns,"transforms":frame.transforms,"base_pose":{"position":&obs[..3],"quat_xyzw":[obs[4],obs[5],obs[6],obs[3]]}}));
        self.rows.push(Row {
            observation: frame.observation,
            command: frame.command_velocity,
            stamp: frame.sim_time_ns,
        });
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        let n = self.rows.len();
        ensure!(
            n > 0,
            "No frames captured. Discard this empty take and try again."
        );
        for writer in std::mem::take(&mut self.writers) {
            ensure!(writer.finish()? == n as u64, "Video frame count mismatch");
        }
        self.audit.sync_all()?;
        let indexes = || (0..n as i64).collect::<Vec<_>>();
        let zeros = || vec![0_i64; n];
        parquet(
            &self
                .path
                .join("lerobot/data/chunk-000/episode_000000.parquet"),
            vec![
                (
                    "observation.state",
                    states(
                        self.rows
                            .iter()
                            .flat_map(|r| r.observation[7..24].iter().map(|v| *v as f32))
                            .collect(),
                    )?,
                ),
                (
                    "action",
                    states(
                        self.rows
                            .iter()
                            .flat_map(|r| r.observation[24..41].iter().map(|v| *v as f32))
                            .collect(),
                    )?,
                ),
                (
                    "timestamp",
                    Arc::new(Float64Array::from(
                        (0..n).map(|i| i as f64 / FPS as f64).collect::<Vec<_>>(),
                    )),
                ),
                ("frame_index", Arc::new(Int64Array::from(indexes()))),
                ("episode_index", Arc::new(Int64Array::from(zeros()))),
                ("index", Arc::new(Int64Array::from(indexes()))),
                ("task_index", Arc::new(Int64Array::from(zeros()))),
            ],
        )?;
        parquet(
            &self.path.join("raw/control.parquet"),
            vec![
                (
                    "sim_time_ns",
                    Arc::new(Int64Array::from(
                        self.rows.iter().map(|r| r.stamp as i64).collect::<Vec<_>>(),
                    )),
                ),
                (
                    "base_pose",
                    floats(
                        self.rows
                            .iter()
                            .flat_map(|r| r.observation[..7].iter().copied())
                            .collect(),
                        7,
                    )?,
                ),
                (
                    "proposed_wheel_rates",
                    floats(
                        self.rows
                            .iter()
                            .flat_map(|r| r.command[..3].iter().copied())
                            .collect(),
                        3,
                    )?,
                ),
                (
                    "applied_wheel_rates",
                    floats(
                        self.rows
                            .iter()
                            .flat_map(|r| r.command[..3].iter().copied())
                            .collect(),
                        3,
                    )?,
                ),
            ],
        )?;
        let meta = self.path.join("lerobot/meta");
        fs::create_dir_all(meta.join("scene"))?;
        let mut features = serde_json::Map::new();
        for key in ["observation.state", "action"] {
            features.insert(
                key.into(),
                json!({"dtype":"float32","shape":[17],"names":self.info["joint_names"]}),
            );
        }
        for key in [
            "timestamp",
            "frame_index",
            "episode_index",
            "index",
            "task_index",
        ] {
            features.insert(key.into(),json!({"dtype":if key=="timestamp"{"float32"}else{"int64"},"shape":[1],"names":null}));
        }
        for name in CAMERAS {
            features.insert(format!("observation.images.{name}"),json!({"dtype":"video","shape":[HEIGHT,WIDTH,3],"names":["height","width","channels"],"info":{"video.fps":FPS,"video.codec":"mpeg4","video.pix_fmt":"yuv420p","video.is_depth_map":false,"has_audio":false}}));
        }
        write_json(
            &meta.join("scene/episode_000000.json"),
            &json!({"version":2,"objects":self.info["objects"],"frames":self.scene}),
        )?;
        jsonl(
            &meta.join("episodes.jsonl"),
            &json!({"episode_index":0,"tasks":[self.task],"length":n}),
        )?;
        jsonl(
            &meta.join("tasks.jsonl"),
            &json!({"task_index":0,"task":self.task}),
        )?;
        let distance: f64 = self
            .rows
            .windows(2)
            .map(|r| {
                (r[0].observation[0] - r[1].observation[0])
                    .hypot(r[0].observation[1] - r[1].observation[1])
            })
            .sum();
        let cube = self.info["objects"]
            .as_array()
            .context("Scene objects")?
            .iter()
            .position(|v| v["name"] == "cube_6")
            .context("Cube object")?;
        let last = &self.scene[n - 1]["transforms"][cube];
        let start = &self.info["initial_objects"][cube];
        let displacement = (last[12].as_f64().unwrap_or(0.0) - start[0].as_f64().unwrap_or(0.0))
            .hypot(last[13].as_f64().unwrap_or(0.0) - start[1].as_f64().unwrap_or(0.0));
        let result = json!({"success":true,"task_success":null,"navigation_success":null,"recording_kind":"manual teleoperation; task success not evaluated","frames":n,"camera_frames":CAMERAS.into_iter().map(|s|(s,n)).collect::<BTreeMap<_,_>>(),"fps":FPS,"sim_seconds":n as f64/FPS as f64,"wall_seconds":self.started.elapsed().as_secs_f64(),"base_distance_m":distance,"cube_push_m":displacement,"cube_contact_frames":self.cube_frames,"furniture_contact_frames":self.bad.len(),"furniture_contacts":self.bad,"contact_audit_hz":100,"root_pose_writes_during_rollout":0,"runtime":"rust","limitations":["Rigid furniture, no sofa cushion deformation","Solid-wheel collision meshes; simulation calibration, not hardware omni-wheel validation"]});
        write_json(&self.path.join("result.json"), &result)?;
        write_json(&meta.join("capture-result.json"), &result)?;
        let mut capture = self.info.clone();
        capture["clock"] = json!(
            "Episode stamps start at one capture interval; source_sim_time_ns retains the scene clock in each raw frame."
        );
        capture["raw_layout"] =
            json!("raw/frame_XXXXXX.npz; camera arrays use camera-name__field keys");
        write_json(&meta.join("capture.json"), &capture)?;
        write_json(
            &meta.join("info.json"),
            &json!({"codebase_version":"v2.1","robot_type":"adora_nano","fps":FPS,"total_episodes":1,"total_frames":n,"total_tasks":1,"total_videos":3,"total_chunks":1,"chunks_size":1000,"splits":{"train":"0:1"},"data_path":"data/chunk-{episode_chunk:03d}/episode_{episode_index:06d}.parquet","video_path":"videos/chunk-{episode_chunk:03d}/{video_key}/episode_{episode_index:06d}.mp4","features":features}),
        )?;
        Ok(())
    }
}
pub struct Recorder {
    root: PathBuf,
    take: Option<Take>,
    status: Value,
    saved: u64,
    pending: BTreeMap<u64, Frame>,
    stop: Option<(Value, u64, Instant)>,
}
impl Recorder {
    pub fn new(root: &Path) -> Result<Self> {
        let s = Self {
            root: root.into(),
            take: None,
            status: json!({"phase":"idle","frames":0,"saved":0,"message":"Ready to record."}),
            saved: 0,
            pending: BTreeMap::new(),
            stop: None,
        };
        s.publish()?;
        Ok(s)
    }
    fn publish(&self) -> Result<()> {
        write_json(&self.root.join("recording-status.json"), &self.status)
    }
    fn phase(&mut self, phase: &str, message: &str) {
        self.status["phase"] = json!(phase);
        self.status["message"] = json!(message);
    }
    pub fn accept(&mut self, message: Message) -> Result<()> {
        match message {
            Message::RecordStart { info, request } => {
                let request: Value = serde_json::from_str(&request)?;
                self.status["request_id"] = request["id"].clone();
                ensure!(
                    self.take.is_none(),
                    "Save or discard the pending episode first."
                );
                self.take = Some(Take::new(
                    &self.root,
                    serde_json::from_str(&info)?,
                    &request,
                )?);
                self.pending.clear();
                self.stop = None;
                self.status["frames"] = json!(0);
                self.phase(
                    "recording",
                    "Recording 3 cameras, 17 joints and scene motion.",
                );
            }
            Message::Frame(f) => {
                ensure!(
                    self.status["phase"] == "recording",
                    "Frame without a started episode"
                );
                let next = self.take.as_ref().context("No active take")?.rows.len() as u64;
                ensure!(
                    f.sequence >= next && !self.pending.contains_key(&f.sequence),
                    "Duplicate recording frame"
                );
                ensure!(
                    self.pending.len() < 64,
                    "Recording cannot keep up; raw data retained."
                );
                self.pending.insert(f.sequence, f);
            }
            Message::RecordStop {
                request,
                frame_count,
            } => {
                let request: Value = serde_json::from_str(&request)?;
                ensure!(self.status["phase"] == "recording", "No recording to stop");
                ensure!(
                    self.take.as_ref().unwrap().rows.len() as u64 <= frame_count,
                    "Stop count is behind captured frames"
                );
                self.stop = Some((request, frame_count, Instant::now()));
            }
            Message::RecordSave(request) => {
                let r: Value = serde_json::from_str(&request)?;
                self.status["request_id"] = r["id"].clone();
                ensure!(
                    self.status["phase"] == "review",
                    "Stop recording before saving the episode."
                );
                let dest = PathBuf::from(
                    r["destination"]
                        .as_str()
                        .context("Recording folder required")?,
                );
                ensure!(dest.is_absolute(), "Choose an absolute recording folder.");
                fs::create_dir_all(&dest)?;
                let take = self.take.as_ref().context("No pending take")?;
                let name = take
                    .path
                    .file_name()
                    .context("Take name")?
                    .to_string_lossy();
                let target = dest.join(format!("nano-{name}"));
                let staging = dest.join(format!(".nano-{name}.saving"));
                ensure!(
                    !target.exists() && !staging.exists(),
                    "Destination or unfinished save already exists."
                );
                if take.path.join("raw").exists() {
                    fs::rename(take.path.join("raw"), take.path.join("lerobot/raw"))?;
                }
                let source = take.path.join("lerobot");
                self.phase("saving", "Saving episode and raw sensor data…");
                self.publish()?;
                if let Err(e) = copy_tree(&source, &staging).and_then(|_| {
                    fs::rename(&staging, &target)?;
                    Ok(())
                }) {
                    let _ = fs::remove_dir_all(&staging);
                    self.phase("review", "Save failed; captured data retained.");
                    return Err(e);
                }
                self.saved += 1;
                self.status["saved"] = json!(self.saved);
                self.status["dataset"] = json!(target);
                self.status["staged"] = Value::Null;
                self.phase(
                    "saved",
                    "Episode saved. Open Replay or record the next episode.",
                );
                self.publish()?;
                let take = self.take.take().unwrap();
                fs::remove_dir_all(take.path)?;
            }
            Message::RecordDiscard(request) => {
                let r: Value = serde_json::from_str(&request)?;
                self.status["request_id"] = r["id"].clone();
                ensure!(
                    self.status["phase"] != "recording",
                    "Stop recording before discarding the episode."
                );
                if let Some(take) = self.take.take() {
                    fs::remove_dir_all(take.path)?;
                }
                self.pending.clear();
                self.stop = None;
                self.status["frames"] = json!(0);
                self.status["staged"] = Value::Null;
                self.phase("idle", "Take discarded. Ready to record.");
            }
            _ => {}
        }
        if self.status["phase"] == "recording" {
            let take = self.take.as_mut().unwrap();
            while let Some(f) = self.pending.remove(&(take.rows.len() as u64)) {
                take.append(f)?;
            }
            self.status["frames"] = json!(take.rows.len());
            if self
                .stop
                .as_ref()
                .is_some_and(|(_, count, _)| *count == take.rows.len() as u64)
            {
                let (request, _, _) = self.stop.take().unwrap();
                self.status["request_id"] = request["id"].clone();
                self.phase("saving", "Finishing videos and checking captured frames…");
                self.publish()?;
                let take = self.take.as_mut().unwrap();
                take.finish()?;
                self.status["staged"] = json!(take.path);
                self.phase(
                    "review",
                    "Episode ready. Save it to keep it, or discard it.",
                );
            }
        }
        self.publish()
    }
    fn tick(&mut self) -> Result<()> {
        if self
            .stop
            .as_ref()
            .is_some_and(|(_, _, time)| time.elapsed().as_secs() > 15)
        {
            if let Some((r, _, _)) = self.stop.take() {
                self.status["request_id"] = r["id"].clone();
            }
            anyhow::bail!("Capture did not receive every frame. Raw data retained.");
        }
        Ok(())
    }
    fn fail(&mut self, error: anyhow::Error) -> Result<()> {
        if let Some(t) = &mut self.take {
            t.writers.clear();
            self.status["staged"] = json!(t.path);
        }
        let complete = self
            .take
            .as_ref()
            .is_some_and(|t| t.path.join("lerobot/meta/info.json").is_file());
        self.phase(
            if complete { "review" } else { "error" },
            &format!("{error:#} Captured data retained; retry Save or discard the take."),
        );
        self.pending.clear();
        self.stop = None;
        self.publish()
    }
}
pub fn run(config: &Config, events: &mut dora_node_api::EventStream) -> Result<()> {
    use dora_node_api::Event;
    let mut recorder = Recorder::new(&config.output)?;
    while let Some(event) = events.recv() {
        let result = match event {
            Event::Input { id, data, .. } => {
                if id.as_str() == "tick" {
                    recorder.tick()
                } else {
                    crate::nodes::decode(&data).and_then(|m| recorder.accept(m))
                }
            }
            Event::Stop(_) => break,
            Event::Error(e) => Err(anyhow::anyhow!(e)),
            _ => Ok(()),
        };
        if let Err(e) = result {
            recorder.fail(e)?;
        }
    }
    if recorder.status["phase"] == "recording" {
        recorder.fail(anyhow::anyhow!("Simulation ended during capture."))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(seq: u64) -> Frame {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut observation = vec![0.0; 41];
        observation[3] = 1.0;
        observation[7] = seq as f64 * 0.1;
        observation[24] = seq as f64 * 0.2;
        Frame {
            sequence: seq,
            sim_time_ns: (seq + 1) * FRAME_NS,
            source_sim_time_ns: 900_000_000 + (seq + 1) * FRAME_NS,
            observation,
            transforms: vec![[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]],
            cameras: CAMERAS
                .iter()
                .enumerate()
                .map(|(i, name)| Sensor {
                    name: name.to_string(),
                    rgb: vec![40 + i as u8 * 50 + seq as u8 * 15; WIDTH * HEIGHT * 3],
                    depth: vec![1.5; WIDTH * HEIGHT],
                    seg: vec![1; WIDTH * HEIGHT],
                    world_from_camera_gl: identity,
                    world_from_mount_link: identity,
                })
                .collect(),
            contacts: Vec::new(),
            physics_contacts: vec![[1, 0, 1]],
            cube_contact: false,
            command_velocity: vec![0.0; 17],
            phase: "Recorder acceptance fixture".into(),
        }
    }
    #[test]
    fn reordered_stop_waits_for_all_three_cameras_and_saves_replay_dataset() {
        let root = std::env::temp_dir().join(format!(
            "dorobot-native-recorder-{}-{}",
            std::process::id(),
            now_ns()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut r = Recorder::new(&root).unwrap();
        let info = json!({"session":"Native recorder acceptance","joint_names":(0..17).map(|i|format!("joint_{i}")).collect::<Vec<_>>(),"objects":[{"name":"cube_6","size":[0.12,0.12,0.12],"color":[1.0,1.0,1.0,1.0]}],"initial_objects":[[0.0,0.0,0.0]]});
        r.accept(Message::RecordStart {
            info: info.to_string(),
            request: json!({"id":1,"task":"Test full native capture"}).to_string(),
        })
        .unwrap();
        r.accept(Message::Frame(frame(1))).unwrap();
        r.accept(Message::RecordStop {
            request: json!({"id":2}).to_string(),
            frame_count: 2,
        })
        .unwrap();
        assert_eq!(r.status["phase"], "recording");
        r.accept(Message::Frame(frame(0))).unwrap();
        assert_eq!(r.status["phase"], "review");
        assert_eq!(r.status["frames"], 2);
        let destination = root.join("saved");
        r.accept(Message::RecordSave(
            json!({"id":3,"destination":destination}).to_string(),
        ))
        .unwrap();
        assert_eq!(r.status["phase"], "saved");
        let dataset = destination.join("nano-1");
        let info: Value =
            serde_json::from_slice(&fs::read(dataset.join("meta/info.json")).unwrap()).unwrap();
        assert_eq!(info["total_frames"], 2);
        assert_eq!(info["total_videos"], 3);
        for name in CAMERAS {
            let source = json!(dataset.join(format!(
                "videos/chunk-000/observation.images.{name}/episode_000000.mp4"
            )));
            let mut camera = crate::video::Camera::open(&source, 5.0).unwrap();
            let a = camera.rgb().unwrap();
            let b = camera.rgb().unwrap();
            assert_ne!(a, b);
            assert!(camera.rgb().is_err());
        }
        for seq in 0..2 {
            let mut npz = ndarray_npy::NpzReader::new(
                File::open(dataset.join(format!("raw/frame_{seq:06}.npz"))).unwrap(),
            )
            .unwrap();
            let observation: Array1<f64> = npz.by_name("observation").unwrap();
            assert_eq!(observation[7], seq as f64 * 0.1);
            for name in CAMERAS {
                let depth: Array2<f32> = npz.by_name(&format!("{name}__depth")).unwrap();
                assert_eq!(depth.shape(), &[HEIGHT, WIDTH]);
            }
        }
        assert!(!root.join("takes/1").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn nonfinite_sensor_payload_is_rejected() {
        let mut f = frame(0);
        f.cameras[1].depth[0] = f32::NAN;
        assert!(f.validate(true).is_err());
    }
}
