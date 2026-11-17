use crate::protocol::*;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// Latest frame only for preview; the recorder has its own lossless Dora input.
pub struct Preview {
    slot: Arc<(Mutex<(u64, Vec<u8>)>, Condvar)>,
    stop: Arc<AtomicBool>,
    socket: std::path::PathBuf,
}
impl Preview {
    pub fn new(socket: &Path) -> Result<Self> {
        ensure!(!socket.exists(), "Preview socket already exists");
        let listener = UnixListener::bind(socket)?;
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let slot = Arc::new((Mutex::new((0, Vec::new())), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, end) = (slot.clone(), stop.clone());
        thread::spawn(move || {
            while !end.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut client, _)) => {
                        // macOS inherits the listener's nonblocking flag on accept.
                        // A full three-camera frame exceeds the socket send buffer;
                        // write_all must wait for the reader, bounded by its timeout.
                        if client.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let (s, end) = (s.clone(), end.clone());
                        thread::spawn(move || {
                            let _ = client.set_write_timeout(Some(Duration::from_secs(2)));
                            let mut seen = 0;
                            while !end.load(Ordering::Acquire) {
                                let (lock, notify) = &*s;
                                let mut data = lock.lock().unwrap();
                                if data.0 == seen {
                                    data = notify
                                        .wait_timeout(data, Duration::from_millis(200))
                                        .unwrap()
                                        .0;
                                }
                                if data.0 == seen {
                                    continue;
                                }
                                seen = data.0;
                                let bytes = data.1.clone();
                                drop(data);
                                if client.write_all(&bytes).is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            slot,
            stop,
            socket: socket.to_owned(),
        })
    }
    pub fn publish(&self, info: &Value, frame: &Frame) -> Result<()> {
        frame.validate(false)?;
        let obs = &frame.observation;
        let header = json!({"version":1,"session":info["session"],"sequence":frame.sequence,"sim_time_ns":frame.sim_time_ns,"fps":FPS,
            "joint_names":info["joint_names"],"joints":&obs[7..24],"targets":&obs[24..41],"base_position":&obs[..3],"base_quat_wxyz":&obs[3..7],
            "objects":info["objects"],"transforms":frame.transforms,"cameras":CAMERAS.map(|name|json!({"name":name,"width":WIDTH,"height":HEIGHT,"bytes":WIDTH*HEIGHT*4})),"phase":frame.phase,"finished":false});
        let json = serde_json::to_vec(&header)?;
        ensure!(json.len() < 2 * 1024 * 1024, "Oversized preview header");
        let mut bytes = Vec::with_capacity(4 + json.len() + WIDTH * HEIGHT * 12);
        bytes.extend((json.len() as u32).to_be_bytes());
        bytes.extend(json);
        for camera in &frame.cameras {
            for rgb in camera.rgb.chunks_exact(3) {
                bytes.extend([rgb[2], rgb[1], rgb[0], 255]);
            }
        }
        let (lock, notify) = &*self.slot;
        let mut slot = lock.lock().map_err(|_| anyhow::anyhow!("Preview mutex"))?;
        slot.0 += 1;
        slot.1 = bytes;
        notify.notify_all();
        Ok(())
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.slot.1.notify_all();
        let _ = fs::remove_file(&self.socket);
    }
}

pub fn run(
    config: &Config,
    node: &mut dora_node_api::DoraNode,
    events: &mut dora_node_api::EventStream,
) -> Result<()> {
    let hub = Preview::new(&config.socket)?;
    let mut info = None;
    crate::nodes::send(node, "viewer_ready", &Message::Ready)?;
    while let Some(e) = events.recv() {
        match e {
            dora_node_api::Event::Input { data, .. } => match crate::nodes::decode(&data)? {
                Message::Info(s) => {
                    info = Some(serde_json::from_str::<Value>(&s)?);
                }
                Message::Frame(f) => {
                    hub.publish(info.as_ref().context("Frame before scene info")?, &f)?
                }
                _ => {}
            },
            dora_node_api::Event::Stop(_) => break,
            dora_node_api::Event::Error(e) => anyhow::bail!(e),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, os::unix::net::UnixStream};
    #[test]
    fn full_camera_packet_survives_socket_backpressure() {
        let socket = std::env::temp_dir().join(format!("dorobot-preview-test-{}.sock", now_ns()));
        let hub = Preview::new(&socket).unwrap();
        let mut client = UnixStream::connect(&socket).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut observation = vec![0.0; 41];
        observation[3] = 1.0;
        let frame = Frame {
            sequence: 7,
            sim_time_ns: FRAME_NS,
            source_sim_time_ns: FRAME_NS,
            observation,
            transforms: Vec::new(),
            contacts: Vec::new(),
            physics_contacts: Vec::new(),
            cube_contact: false,
            command_velocity: vec![0.0; 17],
            phase: "Preview".into(),
            cameras: CAMERAS
                .into_iter()
                .enumerate()
                .map(|(i, name)| Sensor {
                    name: name.into(),
                    rgb: [i as u8 + 1, 20, 30].repeat(WIDTH * HEIGHT),
                    depth: Vec::new(),
                    seg: Vec::new(),
                    world_from_camera_gl: [[0.0; 4]; 4],
                    world_from_mount_link: [[0.0; 4]; 4],
                })
                .collect(),
        };
        hub.publish(
            &json!({"session":"transport-test","joint_names":[],"objects":[]}),
            &frame,
        )
        .unwrap();
        // Deliberately let the producer fill its small kernel buffer first.
        thread::sleep(Duration::from_millis(100));
        let mut size = [0; 4];
        client.read_exact(&mut size).unwrap();
        let mut header = vec![0; u32::from_be_bytes(size) as usize];
        client.read_exact(&mut header).unwrap();
        let header: Value = serde_json::from_slice(&header).unwrap();
        assert_eq!(header["sequence"], 7);
        for i in 0..3 {
            let mut bytes = vec![0; WIDTH * HEIGHT * 4];
            client.read_exact(&mut bytes).unwrap();
            assert!(bytes.chunks_exact(4).all(|p| p == [30, 20, i + 1, 255]));
        }
    }
}
