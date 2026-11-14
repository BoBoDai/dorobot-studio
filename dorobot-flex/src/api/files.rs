//! Backend backed by LeRobot datasets on disk.
//!
//! Local datasets, curation, Hub transfers and managed LeRobot capture.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::data::LeRobotDataset;

use super::*;

fn remove_take_staging(path: &Path) {
    if path.parent() == Some(std::env::temp_dir().as_path())
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("dorobot-capture-"))
    {
        let _ = fs::remove_dir_all(path);
    }
}

/// URDF and asset dir for a dataset's `robot_type`, when one ships in `data/`.
/// Mirrors the shipping player's mapping so both resolve the same robot.
fn urdf_for(robot_type: &str) -> Option<(PathBuf, PathBuf)> {
    let robot = robot_type.to_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("so101", "so100"),
        ("so100", "so100"),
        ("aimee", "so100"),
        ("lekiwi", "lekiwi"),
        ("moss", "moss"),
        ("koch", "koch"),
        ("vx300s", "vx300s"),
    ];
    let dir = KNOWN
        .iter()
        .find(|(pat, _)| robot.contains(pat))
        .map(|(_, d)| (*d).to_string())
        .unwrap_or(robot);
    let assets = PathBuf::from("data").join(&dir);
    let urdf = assets.join(format!("{dir}.urdf"));
    urdf.exists().then_some((urdf, assets))
}

/// Turn a recorded pose into URDF joint values.
///
/// LeRobot does not record units in the dataset, so the convention has to be
/// established against the model. Raw degrees is ruled out: it puts 1183 of
/// 2724 joint samples outside this URDF's declared limits. Two candidates
/// survive, and `DOROBOT_JOINT_MAP` selects between them while the SO-100 case
/// is being confirmed against video:
///
/// - `signflip` — degrees, with shoulder_pan/lift, elbow and wrist_flex
///   inverted. Three of those are forced: the recorded range only fits the
///   joint limit when negated.
/// - `norm` (default) — LeRobot's normalized units, +/-100 across each joint's
///   range and 0..100 for the gripper. The recorded spans match that shape:
///   every joint lands inside +/-102 while the gripper alone stays in 0..29.
///
/// Direction is a separate question, and the data cannot answer it: shoulder
/// pan's limit is symmetric, so a mirrored sweep satisfies every bound the
/// model declares. It took the video to catch that pan runs the other way —
/// the arm swung right as the recording panned left. Pan sits at 0.35 (dead
/// centre) on frame 0, so inverting it moves the rest pose by under a degree
/// and reverses the whole sweep, which is exactly what was observed.
fn to_urdf_pose(robot: &str, state: &[f32], limits: &[(f32, f32)]) -> Vec<f32> {
    if !(robot.contains("so100") || robot.contains("so101") || robot.contains("aimee")) {
        return state.to_vec();
    }
    let signflip = std::env::var("DOROBOT_JOINT_MAP").as_deref() == Ok("signflip");
    const SIGN: [f32; 6] = [-1.0, -1.0, -1.0, -1.0, 1.0, 1.0];
    // Which normalized channels run opposite to the URDF's axis.
    const FLIP: [bool; 6] = [true, false, false, false, false, false];
    state
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if signflip {
                v.to_radians() * SIGN.get(i).copied().unwrap_or(1.0)
            } else {
                let (lo, hi) = limits.get(i).copied().unwrap_or((-3.15, 3.15));
                let v = if FLIP.get(i).copied().unwrap_or(false) {
                    -v
                } else {
                    *v
                };
                // The gripper is the one channel LeRobot normalizes 0..100.
                let t = if i == 5 {
                    v / 100.0
                } else {
                    (v + 100.0) / 200.0
                };
                lo + t * (hi - lo)
            }
        })
        .collect()
}

/// Movable joint limits, in URDF order, read from the model the viewer loads.
fn urdf_limits(path: &Path) -> Vec<(f32, f32)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in text.split("<joint ").skip(1) {
        let end = block.find("</joint>").unwrap_or(block.len());
        let block = &block[..end];
        if block.contains("type=\"fixed\"") {
            continue;
        }
        let grab = |key: &str| -> Option<f32> {
            let i = block.find(key)? + key.len();
            let rest = &block[i..];
            let j = rest.find('"')?;
            rest[..j].parse().ok()
        };
        if let (Some(lo), Some(hi)) = (grab("lower=\""), grab("upper=\"")) {
            out.push((lo, hi));
        }
    }
    out
}

/// Serial ports that could be a robot arm.
///
/// Bluetooth and debug consoles are always present on macOS and are never an
/// arm, so they are listed separately rather than offered as candidates: the
/// SO-100 shows up through a USB serial bridge (`usbserial`/`usbmodem` here,
/// `ttyUSB`/`ttyACM` on Linux).
fn serial_ports() -> (Vec<String>, usize) {
    let Ok(entries) = fs::read_dir("/dev") else {
        return (Vec::new(), 0);
    };
    let mut candidates = Vec::new();
    let mut others = 0usize;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let is_serial =
            name.starts_with("cu.") || name.starts_with("ttyUSB") || name.starts_with("ttyACM");
        if !is_serial {
            continue;
        }
        if name.contains("usbserial")
            || name.contains("usbmodem")
            || name.starts_with("ttyUSB")
            || name.starts_with("ttyACM")
        {
            candidates.push(format!("/dev/{name}"));
        } else {
            others += 1;
        }
    }
    candidates.sort();
    (candidates, others)
}

/// Scan `root` one level deep for anything that looks like a LeRobot dataset.
fn discover(root: &Path) -> Vec<PathBuf> {
    if root.join("meta/info.json").is_file() {
        return vec![root.to_path_buf()];
    }
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("meta").join("info.json").exists() {
            found.push(p);
        }
    }
    found.sort();
    found
}

/// Total bytes under a directory. Metadata-only walk, so it stays fast even
/// for datasets with hundreds of megabytes of video.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

pub struct FileBackend {
    root: PathBuf,
    status: String,
    hub_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    rig: Option<(PathBuf, super::capture::Rig)>,
    worker: Option<super::capture::Worker>,
    take: Option<serde_json::Value>,
    save_rx: Option<std::sync::mpsc::Receiver<Result<u64, String>>>,
    excluded: BTreeSet<u64>,
    undo_exclude: Option<u64>,
    screen: Screen,
    library: LibraryState,
    playback: PlaybackState,
    hardware: HardwareState,
    record: RecordState,
    eval: EvalState,
    /// Currently opened dataset, kept so episode selection avoids a reparse.
    open: Option<(String, LeRobotDataset)>,
}

impl FileBackend {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self::with_open(root, None)
    }

    /// `prefer` names the dataset to open on start; falls back to the first
    /// one found.
    pub fn with_open(root: impl AsRef<Path>, prefer: Option<&str>) -> Self {
        let mut b = Self {
            root: root.as_ref().to_path_buf(),
            status: "Open a dataset folder to review, or load a rig profile to record.".into(),
            hub_rx: None,
            rig: None,
            worker: None,
            take: None,
            save_rx: None,
            excluded: BTreeSet::new(),
            undo_exclude: None,
            screen: Screen::Library,
            library: LibraryState::default(),
            playback: PlaybackState::default(),
            hardware: HardwareState::default(),
            record: RecordState::default(),
            eval: EvalState::default(),
            open: None,
        };
        b.rescan();
        // Open something immediately so Play is never blank.
        let pick = prefer
            .and_then(|p| b.library.datasets.iter().find(|d| d.id.contains(p)))
            .or_else(|| b.library.datasets.first())
            .map(|d| d.id.clone());
        if let Some(id) = pick {
            b.open_dataset(&id);
        }
        b
    }

    pub fn rescan(&mut self) {
        let mut datasets = Vec::new();
        for path in discover(&self.root) {
            let ds = match LeRobotDataset::open(&path) {
                Ok(ds) => ds,
                Err(e) => {
                    self.status = format!("{}: {e}", path.display());
                    continue;
                }
            };
            let tags = Self::load_tags(&path.join("meta/dorobot_tags.json"));
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "dataset".into());
            datasets.push(DatasetSummary {
                id: name.clone(),
                name,
                episodes: ds.info.total_episodes as u32,
                fps: ds.info.fps,
                robot: if ds.info.robot_type.is_empty() {
                    "unknown".into()
                } else {
                    ds.info.robot_type.clone()
                },
                size_gb: dir_size(&path) as f64 / 1e9,
                // Curation tallies come from the persisted sidecar.
                good: tags.iter().filter(|(_, t)| *t == Tag::Good).count() as u32,
                bad: tags.iter().filter(|(_, t)| *t == Tag::Bad).count() as u32,
                sync: SyncState::LocalOnly,
                thumbnail: None,
                path,
            });
        }
        self.library.datasets = datasets;
        self.library.devices.clear();
        self.library.sessions.clear();
    }

    fn open_dataset(&mut self, id: &str) {
        let Some(summary) = self.library.datasets.iter().find(|d| d.id == id).cloned() else {
            return;
        };
        let ds = match LeRobotDataset::open(&summary.path) {
            Ok(ds) => ds,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        self.excluded = fs::read(summary.path.join("meta/dorobot_excluded.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let fps = ds.info.fps.max(1e-6);
        let episodes: Vec<EpisodeEntry> = ds
            .episodes
            .iter()
            .filter(|e| !self.excluded.contains(&e.episode_index))
            .map(|e| EpisodeEntry {
                index: e.episode_index,
                task_group: e
                    .tasks
                    .first()
                    .and_then(|t| ds.get_task(*t))
                    .unwrap_or("untitled")
                    .to_string(),
                duration_s: e.length as f64 / fps,
                tag: None,
            })
            .collect();

        // Channel names come from `features`, so plots can label series
        // instead of falling back to state[0..n].
        let names_of = |key: &str| -> Vec<String> {
            ds.info
                .features
                .get(key)
                .and_then(|f| f.names.clone())
                .unwrap_or_default()
        };

        self.playback = PlaybackState {
            dataset_name: summary.name.clone(),
            selected: episodes.first().map(|e| e.index),
            episodes,
            stats: EpisodeStats::default(),
            drift_series: Vec::new(),
            state_names: names_of("observation.state"),
            action_names: names_of("action"),
            state_series: Vec::new(),
            action_series: Vec::new(),
            current_time: 0.0,
            is_playing: false,
            speed: 1.0,
            video_paths: BTreeMap::new(),
            video_frame_offset: 0,
            video_frame_offsets: BTreeMap::new(),
            robot_urdf: urdf_for(&ds.info.robot_type),
            joint_frames: Vec::new(),
        };
        // Restore any curation recorded for this dataset before.
        let saved = Self::load_tags(&summary.path.join("meta").join("dorobot_tags.json"));
        for (idx, tag) in saved {
            if let Some(e) = self.playback.episodes.iter_mut().find(|e| e.index == idx) {
                e.tag = Some(tag);
            }
        }
        self.open = Some((id.to_string(), ds));
        if let Some(idx) = self.playback.selected {
            self.select_episode(idx);
        }
    }

    fn select_episode(&mut self, index: u64) {
        let Some((_, ds)) = &self.open else { return };
        let data = match ds.load_episode(index) {
            Ok(data) => data,
            Err(e) => {
                self.status = format!("Episode {index}: {e}");
                self.playback.is_playing = false;
                return;
            }
        };
        let fps = ds.info.fps.max(1e-6);

        // Real drift: how far each frame's recorded timestamp is from where a
        // constant frame rate would put it, in frames.
        let drift: Vec<f64> = data
            .frames
            .iter()
            .map(|f| (f.timestamp - f.frame_index as f64 / fps) * fps)
            .collect();
        let mean_drift = if drift.is_empty() {
            0.0
        } else {
            drift.iter().sum::<f64>() / drift.len() as f64
        };

        let task = ds
            .episodes
            .iter()
            .find(|e| e.episode_index == index)
            .and_then(|e| e.tasks.first().copied())
            .and_then(|t| ds.get_task(t))
            .unwrap_or("")
            .to_string();

        // Transpose the frames into channel-major series for the plot. Capped
        // because a legible plot is the point: six traces already crowd the
        // pane, and SO-100 has exactly six joints.
        let series = |pick: fn(&crate::data::lerobot_dataset::EpisodeFrame) -> &Vec<f32>,
                      names: &[String]| {
            let width = data
                .frames
                .first()
                .map(|f| pick(f).len())
                .unwrap_or(0)
                .min(6);
            (0..width)
                .map(|c| PlotChannel {
                    name: names.get(c).cloned().unwrap_or_else(|| format!("ch {c}")),
                    points: data
                        .frames
                        .iter()
                        .filter_map(|f| pick(f).get(c).map(|v| (f.timestamp, *v as f64)))
                        .collect(),
                })
                .collect::<Vec<_>>()
        };
        self.playback.state_series = series(|f| &f.state, &self.playback.state_names);
        self.playback.action_series = series(|f| &f.action, &self.playback.action_names);

        self.playback.video_paths = data
            .video_paths
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.playback.video_frame_offset = data.video_frame_offset;
        self.playback.video_frame_offsets = data.video_frame_offsets.into_iter().collect();
        let robot = ds.info.robot_type.to_lowercase();
        let limits = self
            .playback
            .robot_urdf
            .as_ref()
            .map(|(u, _)| urdf_limits(u))
            .unwrap_or_default();
        self.playback.joint_frames = data
            .frames
            .iter()
            .map(|f| to_urdf_pose(&robot, &f.state, &limits))
            .collect();
        // A new episode starts at its head, stopped.
        self.playback.current_time = 0.0;
        self.playback.is_playing = false;

        self.playback.selected = Some(index);
        self.playback.stats = EpisodeStats {
            frames: data.frames.len() as u64,
            duration_s: data.frames.len() as f64 / fps,
            fps,
            drift_frames: mean_drift,
            task,
            state_channels: data.frames.first().map(|f| f.state.len()).unwrap_or(0),
            action_channels: data.frames.first().map(|f| f.action.len()).unwrap_or(0),
        };
        self.playback.drift_series = drift;
        self.status = format!(
            "{} · episode {index} · {} frames",
            self.playback.dataset_name, self.playback.stats.frames
        );
    }

    /// Rebuild the setup wizard from what is actually attached.
    ///
    /// Only the first step can be answered without the arm: finding the port is
    /// a filesystem question, while talking to the motors and sweeping the
    /// joints are not. The later steps stay pending and say so rather than
    /// showing a wizard that looks ready to run.
    pub fn rescan_hardware(&mut self) {
        let (ports, others) = serial_ports();
        let found = !ports.is_empty();
        let step = |title: &str, state: StepState| WizardStep {
            title: title.into(),
            state,
        };
        self.hardware.steps = vec![
            step(
                "Find port",
                if found {
                    StepState::Done
                } else {
                    StepState::Active
                },
            ),
            step("Motors", StepState::Pending),
            step("Calibrate", StepState::Pending),
            step("Cameras", StepState::Pending),
            step("Save profile", StepState::Pending),
        ];
        self.hardware.robot_label = if found {
            "unidentified arm".into()
        } else {
            String::new()
        };
        // The mirror shows the model for whatever robot the open dataset names,
        // falling back to the SO-100 so the stage is not empty before one is
        // chosen.
        let robot = self
            .open
            .as_ref()
            .map(|(_, ds)| ds.info.robot_type.to_lowercase())
            .unwrap_or_else(|| "so100".into());
        self.hardware.robot_urdf = urdf_for(&robot).or_else(|| urdf_for("so100"));
        self.hardware.instruction = if found {
            format!(
                "Found {}: {}",
                if ports.len() == 1 { "a port" } else { "ports" },
                ports.join(", ")
            )
        } else if others > 0 {
            format!("No arm found. {others} serial ports are present but all are Bluetooth or debug consoles — connect the arm over USB and rescan.")
        } else {
            "No serial ports at all. Connect the arm over USB and rescan.".into()
        };
        // Joints stay empty until a motor answers; a calibration list here
        // would be an invention.
        self.hardware.joints.clear();
        self.hardware.live_angles.clear();
        self.hardware.active_joint = None;
        self.hardware.can_advance = self.rig.is_some();
        if let Some((_, rig)) = &self.rig {
            self.hardware.robot_label = rig.robot_type.clone();
            self.hardware.instruction.push_str("\nRig loaded. Continue starts LeRobot calibration; follow the prompts and press Continue when asked.");
        }
    }

    /// Where curation tags live for a dataset. A sidecar, not a rewrite of the
    /// LeRobot metadata: tagging must never risk the recording itself.
    fn tag_file(&self) -> Option<PathBuf> {
        let id = self.open.as_ref().map(|(id, _)| id.clone())?;
        let ds = self.library.datasets.iter().find(|d| d.id == id)?;
        Some(ds.path.join("meta").join("dorobot_tags.json"))
    }

    /// Persist tags. Curation that does not survive a restart is not curation.
    fn save_tags(&mut self) -> Result<(), String> {
        let path = self.tag_file().ok_or("No dataset selected")?;
        // Keep tags belonging to excluded episodes, which are absent from the list.
        let mut tags: BTreeMap<u64, Tag> = Self::load_tags(&path).into_iter().collect();
        for e in &self.playback.episodes {
            tags.remove(&e.index);
            if let Some(t) = e.tag {
                tags.insert(e.index, t);
            }
        }
        let mut out = String::from("{\n");
        let mut first = true;
        for (index, t) in tags {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(&format!("  \"{}\": \"{}\"", index, t.label()));
        }
        out.push_str("\n}\n");
        let temp = path.with_extension("json.tmp");
        if let Err(e) = fs::write(&temp, out).and_then(|_| fs::rename(temp, &path)) {
            return Err(format!("Could not save tags: {e}"));
        }
        // Library tallies are derived, so they follow the same edit.
        let (good, bad) = self
            .playback
            .episodes
            .iter()
            .fold((0, 0), |(g, b), e| match e.tag {
                Some(Tag::Good) => (g + 1, b),
                Some(Tag::Bad) => (g, b + 1),
                None => (g, b),
            });
        let id = self.open.as_ref().map(|(id, _)| id.clone());
        if let Some(d) = self
            .library
            .datasets
            .iter_mut()
            .find(|d| Some(&d.id) == id.as_ref())
        {
            d.good = good;
            d.bad = bad;
        }
        Ok(())
    }

    /// Read the sidecar written by [`Self::save_tags`].
    fn load_tags(path: &Path) -> Vec<(u64, Tag)> {
        let Ok(bytes) = fs::read(path) else {
            return Vec::new();
        };
        let Ok(map) = serde_json::from_slice::<BTreeMap<u64, String>>(&bytes) else {
            return Vec::new();
        };
        map.into_iter()
            .filter_map(|(i, tag)| match tag.as_str() {
                "good" => Some((i, Tag::Good)),
                "bad" => Some((i, Tag::Bad)),
                _ => None,
            })
            .collect()
    }

    fn configure(&mut self, path: PathBuf) {
        if self.worker.is_some() || self.save_rx.is_some() || self.take.is_some() {
            self.status =
                "Stop and save or discard the current take before changing profiles.".into();
            return;
        }
        match super::capture::Rig::load(&path) {
            Ok(rig) => {
                self.record = RecordState {
                    robot_urdf: urdf_for(&rig.robot_type),
                    profile_label: rig.robot_type.clone(),
                    task: rig.task.clone(),
                    episode_target: rig.episode_target,
                    episode_index: 1,
                    cameras: rig.cameras.keys().cloned().collect(),
                    ..Default::default()
                };
                self.hardware.robot_label = rig.robot_type.clone();
                self.hardware.can_advance = true;
                self.hardware.instruction = "Rig loaded. Continue opens the LeRobot calibration console. Follow its instructions and press Continue when prompted.".into();
                let pending = fs::read(path.with_extension("pending-take.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
                self.rig = Some((path, rig));
                self.status =
                    "Rig loaded. Calibrate in Hardware, then start a recording session.".into();
                if let Some(take) =
                    pending.filter(|t| t["frames"].as_str().is_some_and(|p| Path::new(p).is_file()))
                {
                    self.record.last_take = Some(TakeReview {
                        thumbnails: Vec::new(),
                        verdict: TakeVerdict::ReadyToSave,
                        warnings: Vec::new(),
                    });
                    self.take = Some(take);
                    self.status =
                        "Recovered an unsaved take. Save or discard it before recording.".into();
                }
            }
            Err(e) => self.status = e,
        }
    }
    fn start_worker(&mut self, calibration: bool) {
        if self.worker.is_some() || self.save_rx.is_some() || (!calibration && self.take.is_some())
        {
            self.status = "Save or discard the pending take before recording again.".into();
            return;
        }
        let Some((path, _)) = &self.rig else {
            self.status = "Load a rig profile using New Recording Session.".into();
            return;
        };
        match super::capture::Worker::start(path, calibration) {
            Ok(w) => {
                self.worker = Some(w);
                self.record.busy = true;
                self.status = if calibration {
                    "Calibration starting…"
                } else {
                    "Connecting to the rig…"
                }
                .into();
            }
            Err(e) => self.status = e,
        }
    }
    fn exclude(&mut self, index: u64, undo: bool) {
        let Some((id, ds)) = &self.open else { return };
        let id = id.clone();
        let mut excluded = self.excluded.clone();
        if undo {
            excluded.remove(&index);
        } else {
            excluded.insert(index);
        }
        let path = ds.root_path.join("meta/dorobot_excluded.json");
        let temp = path.with_extension("json.tmp");
        let result = fs::write(&temp, serde_json::to_vec(&excluded).unwrap())
            .and_then(|_| fs::rename(temp, path));
        match result {
            Ok(()) => {
                self.undo_exclude = if undo { None } else { Some(index) };
                self.open_dataset(&id);
                self.status = if undo { "Episode restored." } else { "Episode excluded from review; original data preserved. Press Ctrl/Cmd+Z to undo." }.into();
            }
            Err(e) => self.status = format!("Could not save exclusion: {e}"),
        }
    }

    /// Distinct task descriptions, used by the Play tree's group headers.
    pub fn task_groups(&self) -> Vec<String> {
        self.playback
            .episodes
            .iter()
            .map(|e| e.task_group.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

impl Backend for FileBackend {
    fn dataset_id(&self) -> Option<&str> {
        self.open.as_ref().map(|(id, _)| id.as_str())
    }
    fn status(&self) -> Option<&str> {
        Some(&self.status)
    }
    fn screen(&self) -> Screen {
        self.screen
    }
    fn library(&self) -> &LibraryState {
        &self.library
    }
    fn hardware(&self) -> &HardwareState {
        &self.hardware
    }
    fn record(&self) -> &RecordState {
        &self.record
    }
    fn playback(&self) -> &PlaybackState {
        &self.playback
    }
    fn eval(&self) -> &EvalState {
        &self.eval
    }

    fn dispatch(&mut self, intent: Intent) {
        match intent {
            Intent::ConfigureRig(path) => self.configure(path),
            Intent::HubTransfer { repo, destination } => {
                if self.hub_rx.is_some() || self.record.recording || self.save_rx.is_some() { self.status = "Finish the active operation before transferring a dataset.".into(); return; }
                let upload = destination.is_none();
                let path = destination.or_else(|| self.open.as_ref().map(|(_,ds)| ds.root_path.clone()));
                let Some(path) = path else { self.status = "Open a dataset first.".into(); return; };
                let (tx, rx) = std::sync::mpsc::channel(); self.hub_rx = Some(rx);
                self.status = format!("{} {repo}…", if upload { "Uploading" } else { "Downloading" });
                self.library.busy = Some(self.status.clone());
                std::thread::spawn(move || {
                    let python = std::env::var_os("DOROBOT_PYTHON").unwrap_or_else(|| "python3".into());
                    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/capture/hub.py");
                    let result = std::process::Command::new(python).arg(script).arg(if upload { "push" } else { "pull" }).arg(repo).arg(&path).output()
                        .map_err(|e| e.to_string()).and_then(|o| if o.status.success() { Ok(path) } else { Err(String::from_utf8_lossy(&o.stderr).chars().rev().take(1500).collect::<String>().chars().rev().collect()) });
                    let _ = tx.send(result);
                });
            },
            Intent::OpenFolder(path) => { self.root = path; self.rescan(); if let Some(id) = self.library.datasets.first().map(|d| d.id.clone()) { self.open_dataset(&id); self.screen = Screen::Play; } },
            Intent::NewRecordingSession => { self.screen = Screen::Record; },
            Intent::WizardAdvance => {
                if let Some(w) = &mut self.worker { if w.calibration { if let Err(e) = w.send("") { self.status = e; } } }
                else { self.start_worker(true); }
            }
            Intent::RecordStart => self.start_worker(false),
            Intent::RecordStop => { if let Some(w) = &mut self.worker { if let Err(e) = w.send("stop") { self.status = e; } self.status = "Stopping capture and closing cameras…".into(); self.record.busy = true; } },
            Intent::SaveEpisode => {
                if self.hub_rx.is_none() && self.save_rx.is_none() && self.worker.is_none() {
                    if let (Some((_, rig)), Some(take)) = (&self.rig, &self.take) {
                        let (rig, take) = (rig.clone(), take.clone());
                        let (tx, rx) = std::sync::mpsc::channel(); self.save_rx = Some(rx); self.record.busy = true;
                        std::thread::spawn(move || { let _ = tx.send(super::capture::save_take(&rig, &take)); });
                        self.status = "Saving episode…".into();
                    }
                }
            }
            Intent::DiscardLast | Intent::ReRecord => {
                if self.worker.is_none() && self.save_rx.is_none() {
                    if let Some(take) = self.take.take() { if let Some(path) = take["frames"].as_str().and_then(|p| Path::new(p).parent()) { remove_take_staging(path); } self.record.discarded += 1; }
                    if let Some((profile,_)) = &self.rig { let _ = fs::remove_file(profile.with_extension("pending-take.json")); }
                    self.record.last_take = None; self.status = "Take discarded. Ready to record.".into();
                    if matches!(intent, Intent::ReRecord) { self.start_worker(false); }
                }
            }
            Intent::SetSoundCues(on) => self.record.sound_cues = on,
            Intent::DeleteEpisode(index) => self.exclude(index, false),
            Intent::UndoExclude => { if let Some(i) = self.undo_exclude { self.exclude(i, true); } },
            Intent::Navigate(s) => {
                if s == Screen::Hardware {
                    self.rescan_hardware();
                }
                self.screen = s;
            }
            Intent::RescanDatasets => {
                self.rescan();
                self.rescan_hardware();
            }
            Intent::OpenDataset(id) => {
                self.open_dataset(&id);
                self.screen = Screen::Play;
            }
            Intent::SelectEpisode(i) => self.select_episode(i),
            // Re-enumerate ports: the only wizard step that can be answered
            // without the arm attached.
            Intent::WizardRestartStep => { if self.worker.as_ref().is_some_and(|w| !w.calibration) { self.status = "Stop and review the recording before restarting hardware setup.".into(); return; } self.worker = None; self.record.busy = false; self.rescan_hardware(); self.hardware.can_advance = self.rig.is_some(); },
            Intent::TogglePlay => {
                // Restart from the head when play is pressed at the end,
                // rather than sitting there doing nothing.
                if !self.playback.is_playing
                    && self.playback.current_time >= self.playback.stats.duration_s - 1e-6
                {
                    self.playback.current_time = 0.0;
                }
                self.playback.is_playing = !self.playback.is_playing && self.playback.stats.frames > 0;
            }
            Intent::Seek(t) => {
                if t.is_finite() { self.playback.current_time = t.clamp(0.0, self.playback.stats.duration_s); }
            }
            Intent::StepFrames(n) => {
                let fps = self.playback.stats.fps.max(1e-6);
                let t = self.playback.current_time + n as f64 / fps;
                if t.is_finite() { self.playback.current_time = t.clamp(0.0, self.playback.stats.duration_s); }
                self.playback.is_playing = false;
            }
            Intent::SetSpeed(s) => { if s.is_finite() { self.playback.speed = s.clamp(0.05, 8.0); } },
            Intent::TagEpisode { episode, tag } => {
                let previous = self.playback.episodes.iter().find(|e| e.index == episode).and_then(|e| e.tag);
                if let Some(e) = self.playback.episodes.iter_mut().find(|e| e.index == episode) {
                    // Clicking the tag an episode already carries clears it,
                    // so the same button both sets and undoes.
                    e.tag = if e.tag == tag { None } else { tag };
                }
                if let Err(e) = self.save_tags() {
                    if let Some(row) = self.playback.episodes.iter_mut().find(|e| e.index == episode) { row.tag = previous; }
                    self.status = e;
                }
            }
            // Transfers require a repository and destination in the native form.
            Intent::PullFromHub | Intent::PushToHub => self.status = "Use hf download/upload for Hub transfers; then Open folder or Rescan. No transfer has been started.".into(),
            Intent::StopRollout => { self.eval.driving = false; self.status = "No policy runner is connected.".into(); },
        }
    }

    fn poll(&mut self) {
        if let Some(result) = self.hub_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.hub_rx = None;
            self.library.busy = None;
            match result {
                Ok(path) => {
                    self.root = path;
                    self.rescan();
                    self.status = "Hub transfer completed.".into();
                }
                Err(e) => self.status = format!("Hub transfer failed: {e}"),
            }
        }
        let events: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.events.try_iter().collect())
            .unwrap_or_default();
        for v in events {
            match v["event"].as_str().unwrap_or("") {
                "ready" => {
                    self.record.recording = true;
                    self.record.busy = false;
                    self.record.elapsed_s = 0.0;
                    self.record.joints.clear();
                    self.status = "Recording from the rig.".into();
                }
                "frame" => {
                    self.record.elapsed_s = v["timestamp"].as_f64().unwrap_or(0.0);
                    if let Some(values) = v["state"].as_array() {
                        for (i, value) in values.iter().enumerate() {
                            let value = value.as_f64().unwrap_or(0.0) as f32;
                            if self.record.joints.len() <= i {
                                self.record.joints.push(LiveJoint {
                                    name: format!("J{}", i + 1),
                                    value,
                                    min: value,
                                    max: value,
                                    history: Vec::new(),
                                });
                            }
                            let j = &mut self.record.joints[i];
                            j.value = value;
                            j.min = j.min.min(value);
                            j.max = j.max.max(value);
                            j.history.push(value);
                            if j.history.len() > 120 {
                                j.history.remove(0);
                            }
                        }
                    }
                    if let (Some((_, rig)), Some((urdf, _))) = (&self.rig, &self.record.robot_urdf)
                    {
                        self.record.pose = to_urdf_pose(
                            &rig.robot_type,
                            &self
                                .record
                                .joints
                                .iter()
                                .map(|j| j.value)
                                .collect::<Vec<_>>(),
                            &urdf_limits(urdf),
                        );
                    }
                    self.record.previews =
                        serde_json::from_value(v["previews"].clone()).unwrap_or_default();
                }
                "take" => {
                    self.record.recording = false;
                    self.record.busy = false;
                    self.record.last_take = Some(TakeReview {
                        thumbnails: self.record.previews.values().cloned().collect(),
                        verdict: TakeVerdict::ReadyToSave,
                        warnings: Vec::new(),
                    });
                    self.status =
                        "Take ready. Save it or discard it before recording again.".into();
                    if let Some((profile, _)) = &self.rig {
                        let temp = profile.with_extension("pending-take.tmp");
                        let result =
                            serde_json::to_vec(&v)
                                .map_err(|e| e.to_string())
                                .and_then(|bytes| {
                                    fs::write(&temp, bytes)
                                        .and_then(|_| {
                                            fs::rename(
                                                temp,
                                                profile.with_extension("pending-take.json"),
                                            )
                                        })
                                        .map_err(|e| e.to_string())
                                });
                        if let Err(e) = result {
                            self.status =
                                format!("Take ready, but restart recovery could not be saved: {e}");
                        }
                    }
                    self.take = Some(v);
                }
                "prompt" => {
                    self.hardware.instruction = v["message"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .rev()
                        .take(1200)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    self.hardware.can_advance = true;
                }
                "calibrated" => {
                    self.hardware.instruction =
                        "Both arms calibrated. Open Record to capture.".into();
                    self.status = self.hardware.instruction.clone();
                    self.record.busy = false;
                }
                "worker_exit" => {
                    self.worker = None;
                    if self.record.recording {
                        self.record.recording = false;
                        self.status = "Capture worker exited; no take was saved.".into();
                    }
                    self.record.busy = false;
                }
                "error" => {
                    self.status = v["message"].as_str().unwrap_or("Capture failed").into();
                    self.hardware.instruction = self.status.clone();
                    self.record.recording = false;
                    self.record.busy = false;
                }
                _ => {}
            }
        }
        let result = self.save_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.save_rx = None;
            self.record.busy = false;
            match result {
                Ok(index) => {
                    if let Some((profile, _)) = &self.rig {
                        let _ = fs::remove_file(profile.with_extension("pending-take.json"));
                    }
                    self.record.saved += 1;
                    self.record.episode_index = index as u32 + 2;
                    self.record.last_take = None;
                    if let Some(take) = self.take.take() {
                        if let Some(dir) =
                            take["frames"].as_str().and_then(|p| Path::new(p).parent())
                        {
                            remove_take_staging(dir);
                        }
                    }
                    if let Some((_, rig)) = &self.rig {
                        self.root = rig.dataset.clone();
                    }
                    self.rescan();
                    self.status = format!("Episode {index} saved. Open it in Library to review.");
                }
                Err(e) => self.status = format!("Save failed; take retained: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_root_yields_an_empty_library() {
        let b = FileBackend::new("does/not/exist");
        assert!(b.library().datasets.is_empty());
        assert!(b.playback().episodes.is_empty());
    }
    #[test]
    fn dataset_root_tags_exclusion_undo_and_restart() {
        use crate::data::lerobot_writer::{DatasetSpec, DatasetWriter, FrameRecord};
        let root = std::env::temp_dir().join(format!("dorobot-curation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let spec = DatasetSpec {
            robot_type: "so100".into(),
            fps: 30.0,
            camera_keys: vec![],
            joint_names: vec!["j1".into()],
        };
        let mut writer = DatasetWriter::create(&root, spec).unwrap();
        for _ in 0..2 {
            let mut ep = writer.begin_episode("pick").unwrap();
            ep.push(FrameRecord {
                state: vec![0.0],
                action: vec![1.0],
            })
            .unwrap();
            writer.commit(ep).unwrap();
        }
        drop(writer);
        let mut b = FileBackend::new(&root);
        assert_eq!(b.library.datasets.len(), 1);
        assert_eq!(b.playback.stats.frames, 1);
        b.dispatch(Intent::TagEpisode {
            episode: 0,
            tag: Some(Tag::Good),
        });
        b.dispatch(Intent::DeleteEpisode(0));
        assert_eq!(b.playback.episodes.len(), 1);
        b.dispatch(Intent::TagEpisode {
            episode: 1,
            tag: Some(Tag::Bad),
        });
        b.dispatch(Intent::UndoExclude);
        assert_eq!(b.playback.episodes.len(), 2);
        let mut b = FileBackend::new(&root);
        assert_eq!(b.playback.episodes[0].tag, Some(Tag::Good));
        assert_eq!(b.playback.episodes[1].tag, Some(Tag::Bad));
        // A UI relaunch without the original --dataset argument reopens the
        // actual folder, even when it lives outside the default library.
        b.dispatch(Intent::SelectEpisode(1));
        b.dispatch(Intent::Navigate(Screen::Play));
        let saved = crate::preferences::Session::snapshot(&b, true);
        let mut reopened = FileBackend::new(root.join("missing-library"));
        saved.restore(&mut reopened, true);
        assert_eq!(reopened.playback.selected, Some(1));
        assert_eq!(reopened.playback.episodes[1].tag, Some(Tag::Bad));
        assert_eq!(reopened.screen, Screen::Play);
        assert!(!reopened.playback.is_playing);
        // An explicit empty/different dataset takes precedence over recovery.
        let mut explicit = FileBackend::new(root.join("different-library"));
        saved.restore(&mut explicit, false);
        assert!(explicit.playback.episodes.is_empty());
        // A failed sidecar publication must not show an unsaved tag.
        fs::create_dir(root.join("meta/dorobot_tags.json.tmp")).unwrap();
        b.dispatch(Intent::TagEpisode {
            episode: 0,
            tag: Some(Tag::Bad),
        });
        assert_eq!(b.playback.episodes[0].tag, Some(Tag::Good));
        assert!(b.status.contains("Could not save tags"));
        fs::remove_dir_all(root).unwrap();
    }
}
