//! Opt-in acceptance against the real GPU engine and separate Dora processes.
use dorobot_nano_runtime::{protocol::*, video::Camera};
use ndarray::{Array1, Array2, Array3};
use ndarray_npy::NpzReader;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

struct Session {
    child: Child,
    root: PathBuf,
}
impl Session {
    fn read(&self, name: &str) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.root.join(name)).ok()?).ok()
    }
    fn wait(&mut self, name: &str, predicate: impl Fn(&Value) -> bool, timeout: u64) -> Value {
        let start = Instant::now();
        loop {
            if let Some(v) = self.read(name) {
                assert_ne!(v["phase"], "error", "{v}");
                if predicate(&v) {
                    return v;
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Runtime failed; see {}/dora.log",
                self.root.display()
            );
            assert!(
                start.elapsed() < Duration::from_secs(timeout),
                "Timed out at {name}: {:?}; session {}",
                self.read(name),
                self.root.display()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }
    fn request(&self, v: Value) {
        write_json(&self.root.join("session-command.json"), &v).unwrap();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("stop"), "stop");
        let start = Instant::now();
        while self.child.try_wait().ok().flatten().is_none()
            && start.elapsed() < Duration::from_secs(15)
        {
            thread::sleep(Duration::from_millis(50));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[test]
#[ignore = "requires Metal GPU, native runtime with physics, and native Dora CLI"]
fn native_gpu_dora_motion_three_cameras_and_recording() {
    let root = std::env::var_os("DOROBOT_SIM_ACCEPTANCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("dorobot-native-sim-{}", now_ns())));
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../data/adora_nano/adora_nano.urdf")
        .canonicalize()
        .unwrap();
    let executable = std::env::var_os("DOROBOT_TEST_RUNTIME")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_dorobot-nano-runtime").into());
    let child = Command::new(executable)
        .args(["sim", "--self-test", "--model"])
        .arg(&model)
        .arg("--output")
        .arg(&root)
        .spawn()
        .unwrap();
    let mut s = Session { child, root };
    println!("Native GPU acceptance artifacts: {}", s.root.display());
    s.wait("progress.json", |v| v["frame"].as_u64().is_some(), 300);
    s.request(json!({"id":1,"action":"record","task":"Native Rust acceptance: base, both arms, head and synchronized cameras"}));
    s.wait(
        "recording-status.json",
        |v| v["frames"].as_u64().unwrap_or(0) >= 95,
        300,
    );
    s.request(json!({"id":2,"action":"stop"}));
    let stopped = s.wait("recording-status.json", |v| v["phase"] == "review", 60);
    let count = stopped["frames"].as_u64().unwrap() as usize;
    s.request(json!({"id":3,"action":"save","destination":s.root.join("datasets")}));
    let saved = s.wait("recording-status.json", |v| v["phase"] == "saved", 60);
    let dataset = PathBuf::from(saved["dataset"].as_str().unwrap());
    let info: Value =
        serde_json::from_slice(&fs::read(dataset.join("meta/capture.json")).unwrap()).unwrap();
    let audit = fs::read_to_string(dataset.join("raw/frames.jsonl")).unwrap();
    assert_eq!(audit.lines().count(), count);
    let mut low = [f64::INFINITY; 37];
    let mut high = [f64::NEG_INFINITY; 37];
    let mut commanded = [false; 17];
    let mut previous_clock = None;
    for (i, line) in audit.lines().enumerate() {
        let hash: Value = serde_json::from_str(line).unwrap();
        let path = dataset.join(format!("raw/frame_{i:06}.npz"));
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(&path).unwrap())),
            hash["sha256"]
        );
        let mut raw = NpzReader::new(fs::File::open(path).unwrap()).unwrap();
        let observation: Array1<f64> = raw.by_name("observation").unwrap();
        assert!(
            (-0.1..0.3).contains(&observation[2]),
            "Base left the floor: {}",
            observation[2]
        );
        for (j, v) in observation
            .iter()
            .take(3)
            .chain(observation.iter().skip(7).take(34))
            .enumerate()
        {
            assert!(v.is_finite());
            low[j] = low[j].min(*v);
            high[j] = high[j].max(*v);
        }
        let velocity: Array1<f64> = raw.by_name("command_velocity").unwrap();
        for (seen, v) in commanded.iter_mut().zip(velocity) {
            *seen |= v.abs() > 0.01;
        }
        let clock: ndarray::Array0<i64> = raw.by_name("source_sim_time_ns").unwrap();
        if let Some(previous) = previous_clock {
            assert_eq!(clock[()] - previous, FRAME_NS as i64);
        }
        previous_clock = Some(clock[()]);
        for name in CAMERAS {
            let rgb: Array3<u8> = raw.by_name(&format!("{name}__rgb")).unwrap();
            let depth: Array2<f32> = raw.by_name(&format!("{name}__depth")).unwrap();
            let seg: Array2<i64> = raw.by_name(&format!("{name}__seg")).unwrap();
            assert_eq!(rgb.shape(), [HEIGHT, WIDTH, 3]);
            assert_eq!(depth.shape(), [HEIGHT, WIDTH]);
            assert_eq!(seg.shape(), [HEIGHT, WIDTH]);
            assert!(depth.iter().all(|v| v.is_finite()));
            assert!(depth.iter().any(|v| *v > 0.0 && *v < 49.0));
            assert!(seg.iter().any(|v| *v > 0));
            assert!(rgb.iter().max().unwrap() - rgb.iter().min().unwrap() > 20);
            if i == 0 {
                image::save_buffer(
                    s.root.join(format!("{name}.png")),
                    rgb.as_slice().unwrap(),
                    WIDTH as u32,
                    HEIGHT as u32,
                    image::ColorType::Rgb8,
                )
                .unwrap();
            }
        }
    }
    let spans: Vec<f64> = high.iter().zip(low).map(|(h, l)| h - l).collect();
    assert!(
        spans[0].hypot(spans[1]) > 0.005,
        "Base did not move: {spans:?}"
    );
    for (i, (name, span)) in info["joint_names"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&spans[3..20])
        .enumerate()
    {
        assert!(
            commanded[i],
            "Joint {name} never received a controller command"
        );
        assert!(spans[20 + i] > 0.01, "Joint {name} target did not change");
        assert!(*span > 0.005, "Joint {name} did not move: {span}");
    }
    for name in CAMERAS {
        let file = dataset.join(format!(
            "videos/chunk-000/observation.images.{name}/episode_000000.mp4"
        ));
        let mut decoder = Camera::open(&json!(file), FPS as f64).unwrap();
        for _ in 0..count {
            assert_eq!(decoder.rgb().unwrap().len(), WIDTH * HEIGHT * 3);
        }
        assert!(decoder.rgb().is_err());
    }
    let progress = s.read("progress.json").unwrap();
    assert_eq!(progress["root_pose_writes"], 0);
    write_json(&s.root.join("acceptance.json"), &json!({"passed":true,"frames":count,"cameras":3,"joints":17,"measured_spans":spans,"root_pose_writes":0,"dataset":dataset})).unwrap();
}
