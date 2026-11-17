use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Session {
    child: Child,
    root: PathBuf,
}
impl Session {
    fn start() -> Self {
        let root = std::env::temp_dir().join(format!(
            "dorobot-dora-test-{}-{}",
            std::process::id(),
            dorobot_nano_runtime::protocol::now_ns()
        ));
        let child = Command::new(env!("CARGO_BIN_EXE_dorobot-nano-runtime"))
            .args(["base", "--mock", "--self-test", "--output"])
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self { child, root }
    }
    fn status(&self) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.root.join("status.json")).ok()?).ok()
    }
    fn wait(&mut self, condition: impl Fn(&Value) -> bool, seconds: u64) -> Value {
        let start = Instant::now();
        loop {
            if let Some(s) = self.status() {
                if condition(&s) {
                    return s;
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Native Dora exited early"
            );
            assert!(
                start.elapsed() < Duration::from_secs(seconds),
                "Timed out: {:?}",
                self.status()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn stop(&mut self) {
        fs::write(self.root.join("stop"), b"stop").unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "Supervisor failed to stop"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let s = self.status().unwrap();
        assert_eq!(s["phase"], "stopped");
        assert_eq!(s["stop_confirmed"], true);
        assert_eq!(s["command_ticks"], serde_json::json!([0, 0, 0]));
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("stop"), b"stop");
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
#[ignore = "requires native dora-cli 1.0.1 on PATH or DOROBOT_DORA"]
fn native_dora_base_directions_and_graceful_stop() {
    let mut s = Session::start();
    s.wait(
        |v| {
            v["enabled"] == true
                && v["command_ticks"][0].as_i64().unwrap_or(0) < 0
                && v["command_ticks"][2].as_i64().unwrap_or(0) > 0
        },
        15,
    );
    s.wait(
        |v| {
            v["enabled"] == true
                && v["command_ticks"][0].as_i64().unwrap_or(0) < 0
                && v["command_ticks"][1].as_i64().unwrap_or(0) > 0
                && v["command_ticks"][2].as_i64().unwrap_or(0) < 0
        },
        3,
    );
    s.wait(
        |v| {
            v["enabled"] == true
                && v["command_ticks"]
                    .as_array()
                    .is_some_and(|a| a.iter().all(|n| n.as_i64().unwrap_or(0) > 0))
        },
        3,
    );
    s.wait(
        |v| v["enabled"] == false && v["acks"].as_u64().unwrap_or(0) > 300,
        4,
    );
    s.stop();
}
#[test]
#[ignore = "requires native dora-cli 1.0.1 on PATH or DOROBOT_DORA"]
fn stopped_input_process_triggers_independent_motor_watchdog() {
    let mut s = Session::start();
    s.wait(|v| v["enabled"] == true, 15);
    let node: Value =
        serde_json::from_slice(&fs::read(s.root.join("node-gamepad.json")).unwrap()).unwrap();
    let pid = node["pid"].as_i64().unwrap() as i32;
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
    let stopped = s.wait(
        |v| v["enabled"] == false && v["status"].as_str().unwrap_or("").contains("timed out"),
        2,
    );
    assert_eq!(stopped["command_ticks"], serde_json::json!([0, 0, 0]));
    assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(s.status().unwrap()["enabled"], false);
    s.stop();
}
