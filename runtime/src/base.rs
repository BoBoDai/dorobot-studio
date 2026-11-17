//! Feetech omni-base driver. Vendor ticks are commands, never measured odometry.
use crate::{
    control::{TIMEOUT_NS, axis},
    protocol::*,
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub const IDS: [u8; 3] = [13, 14, 15];
pub fn packet(id: u8, instruction: u8, params: &[u8]) -> Vec<u8> {
    let mut p = vec![255, 255, id, (params.len() + 2) as u8, instruction];
    p.extend(params);
    p.push(!p[2..].iter().fold(0u8, |s, v| s.wrapping_add(*v)));
    p
}
pub fn speed_word(value: f64) -> Result<u16> {
    ensure!(
        value.is_finite() && value.abs() <= 1500.0,
        "Invalid motor speed"
    );
    Ok(value.abs() as u16 | if value < 0.0 { 0x8000 } else { 0 })
}
pub fn mix(vx: f64, vy: f64, omega: f64) -> [f64; 3] {
    let mut w = [
        -0.866 * vx + 0.5 * vy + omega,
        -vy + omega,
        0.866 * vx + 0.5 * vy + omega,
    ];
    let scale = w.iter().map(|v| v.abs() / 300.0).fold(1.0, f64::max);
    w.iter_mut().for_each(|v| *v /= scale);
    w
}
#[derive(Default)]
pub struct BaseMapper {
    ready: bool,
    connected: bool,
    paused: bool,
    pause: bool,
}
impl BaseMapper {
    pub fn command(&mut self, i: &Input) -> Result<Command> {
        let a = [
            axis(*i.axes.get("left_x").unwrap_or(&0.0))?,
            axis(*i.axes.get("left_y").unwrap_or(&0.0))?,
            axis(*i.axes.get("right_x").unwrap_or(&0.0))?,
        ];
        let held = i.button("enable");
        let pause = i.button("pause");
        let neutral = a.iter().all(|v| *v == 0.0);
        if !i.connected || !self.connected {
            self.ready = false;
        }
        if i.connected && !held && neutral {
            self.ready = true;
        }
        if pause && !self.pause {
            self.paused = !self.paused;
            self.ready = false;
        }
        let enabled = i.connected && self.ready && held && !self.paused;
        self.connected = i.connected;
        self.pause = pause;
        let wheels = if enabled {
            mix(-a[1] * 300.0, -a[0] * 300.0, -a[2] * 300.0)
        } else {
            [0.0; 3]
        };
        let status = if !i.connected {
            "Controller disconnected"
        } else if self.paused {
            "Paused"
        } else if enabled && wheels.iter().any(|v| *v != 0.0) {
            "Moving"
        } else if enabled {
            "L1 held · move sticks"
        } else if self.ready {
            "Hold L1 to drive"
        } else {
            "Release L1 and center sticks"
        };
        Ok(Command {
            sequence: i.sequence,
            input_ns: i.input_ns,
            connected: i.connected,
            enabled,
            enable_held: held,
            mode: "Drive".into(),
            status: status.into(),
            velocities: wheels.to_vec(),
            guarded: false,
            neutral,
            controller: i.name.clone(),
        })
    }
}
#[derive(Default)]
pub struct BaseGuard {
    sequence: Option<u64>,
    last_ns: Option<u64>,
    last: [f64; 3],
}
impl BaseGuard {
    pub fn accept(&mut self, mut c: Command, now: u64) -> Result<Command> {
        validate(&c)?;
        ensure!(
            self.sequence.is_none_or(|s| c.sequence > s),
            "Reordered base command"
        );
        self.sequence = Some(c.sequence);
        c.enabled = c.enabled
            && c.connected
            && c.enable_held
            && now
                .checked_sub(c.input_ns)
                .is_some_and(|age| age <= TIMEOUT_NS);
        let dt = self
            .last_ns
            .map_or(0.02, |n| now.saturating_sub(n) as f64 * 1e-9)
            .min(0.1);
        for (v, last) in c.velocities.iter_mut().zip(&mut self.last) {
            *v = if c.enabled {
                v.clamp(*last - 600.0 * dt, *last + 600.0 * dt)
            } else {
                0.0
            };
            *last = *v;
        }
        self.last_ns = Some(now);
        c.guarded = true;
        Ok(c)
    }
}
fn validate(c: &Command) -> Result<()> {
    ensure!(
        c.velocities.len() == 3
            && c.velocities
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 300.000001),
        "Invalid base command"
    );
    Ok(())
}
trait Transport: Send {
    fn clear(&mut self) -> Result<()>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn read(&mut self, data: &mut [u8]) -> Result<()>;
}
struct Serial(Box<dyn serialport::SerialPort>);
impl Transport for Serial {
    fn clear(&mut self) -> Result<()> {
        self.0.clear(serialport::ClearBuffer::Input)?;
        Ok(())
    }
    fn write(&mut self, d: &[u8]) -> Result<()> {
        self.0.write_all(d)?;
        Ok(())
    }
    fn read(&mut self, d: &mut [u8]) -> Result<()> {
        self.0.read_exact(d)?;
        Ok(())
    }
}
#[derive(Default)]
struct Mock {
    rx: VecDeque<u8>,
    writes: Vec<Vec<u8>>,
    fault: Option<&'static str>,
}
impl Transport for Mock {
    fn clear(&mut self) -> Result<()> {
        self.rx.clear();
        Ok(())
    }
    fn write(&mut self, d: &[u8]) -> Result<()> {
        ensure!(
            d.len() >= 7
                && d[..2] == [255, 255]
                && d.len() == d[3] as usize + 4
                && d[4] == 3
                && d[2..].iter().fold(0u8, |s, v| s.wrapping_add(*v)) == 255,
            "Bad motor packet"
        );
        self.writes.push(d.to_vec());
        let mut p = packet(
            if self.fault == Some("wrong_id") {
                d[2] + 1
            } else {
                d[2]
            },
            if self.fault == Some("motor_error") {
                4
            } else {
                0
            },
            &[],
        );
        if self.fault == Some("checksum") {
            *p.last_mut().unwrap() ^= 1;
        }
        if self.fault != Some("timeout") {
            self.rx.extend(p);
        }
        Ok(())
    }
    fn read(&mut self, d: &mut [u8]) -> Result<()> {
        for byte in d {
            *byte = self
                .rx
                .pop_front()
                .context("Motor acknowledgement timed out")?;
        }
        Ok(())
    }
}
struct Bus<T: Transport> {
    io: T,
    acks: u64,
    torque: bool,
}
impl<T: Transport> Bus<T> {
    fn write(&mut self, id: u8, reg: u8, value: u16, width: usize) -> Result<()> {
        let mut params = vec![reg];
        params.extend(&value.to_le_bytes()[..width]);
        self.io.clear()?;
        self.io.write(&packet(id, 3, &params))?;
        let mut ack = [0; 6];
        self.io
            .read(&mut ack)
            .with_context(|| format!("Motor {id} acknowledgement"))?;
        ensure!(
            ack[..4] == [255, 255, id, 2]
                && ack[2..].iter().fold(0u8, |s, v| s.wrapping_add(*v)) == 255,
            "Motor {id}: invalid acknowledgement"
        );
        ensure!(ack[4] == 0, "Motor {id}: error status {}", ack[4]);
        self.acks += 1;
        Ok(())
    }
    fn prepare(&mut self) -> Result<()> {
        self.stop()?;
        for id in IDS {
            self.write(id, 55, 0, 1)?;
            self.write(id, 33, 1, 1)?;
            self.write(id, 55, 1, 1)?;
        }
        Ok(())
    }
    fn drive(&mut self, speeds: &[f64], enabled: bool) -> Result<()> {
        ensure!(speeds.len() == 3, "Expected three wheels");
        let words = speeds
            .iter()
            .map(|v| speed_word(*v))
            .collect::<Result<Vec<_>>>()?;
        if enabled && !self.torque {
            for id in IDS {
                self.write(id, 46, 0, 2)?;
            }
            for id in IDS {
                self.write(id, 40, 1, 1)?;
            }
            self.torque = true;
        }
        for (id, word) in IDS.into_iter().zip(words) {
            self.write(id, 46, if enabled { word } else { 0 }, 2)?;
        }
        if !enabled && self.torque {
            for id in IDS {
                self.write(id, 40, 0, 1)?;
            }
            self.torque = false;
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for (reg, width) in [(46, 2), (40, 1)] {
            for id in IDS {
                if let Err(e) = self.write(id, reg, 0, width) {
                    errors.push(e.to_string());
                }
            }
        }
        self.torque = false;
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
}
impl Transport for Box<dyn Transport> {
    fn clear(&mut self) -> Result<()> {
        (**self).clear()
    }
    fn write(&mut self, d: &[u8]) -> Result<()> {
        (**self).write(d)
    }
    fn read(&mut self, d: &mut [u8]) -> Result<()> {
        (**self).read(d)
    }
}
fn heartbeat(c: &Config) -> bool {
    let alive = |pid: u32| unsafe { libc::kill(pid as i32, 0) == 0 };
    if !alive(c.supervisor_pid) || c.owner_pid.is_some_and(|pid| !alive(pid)) {
        return false;
    }
    c.owner_pid.is_none()
        || std::fs::metadata(c.output.join("lease"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age < Duration::from_millis(1250))
}
fn wall_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
pub fn run(
    config: &Config,
    node: &mut dora_node_api::DoraNode,
    events: &mut dora_node_api::EventStream,
) -> Result<()> {
    use dora_node_api::Event;
    let io: Box<dyn Transport> = if config.mock {
        Box::new(Mock::default())
    } else {
        Box::new(Serial(
            serialport::new(
                config.port.as_ref().context("Serial port required")?,
                1_000_000,
            )
            .timeout(Duration::from_millis(20))
            .open()?,
        ))
    };
    let latest = Arc::new(Mutex::new(None::<Command>));
    let stopped = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(json!({"phase":"starting"})));
    let signal_stop = stopped.clone();
    ctrlc::set_handler(move || signal_stop.store(true, Ordering::Release))?;
    let (input, end, published, c) = (
        latest.clone(),
        stopped.clone(),
        status.clone(),
        config.clone(),
    );
    let worker = thread::spawn(move || -> Result<()> {
        let mut bus = Bus {
            io,
            acks: 0,
            torque: false,
        };
        let mut rearm = true;
        let outcome = (|| -> Result<()> {
            bus.prepare()?;
            while !end.load(Ordering::Acquire) {
                ensure!(heartbeat(&c), "Studio or supervisor heartbeat lost");
                let command = input.lock().unwrap().clone();
                let now = now_ns();
                let fresh = command.as_ref().is_some_and(|v| {
                    now.checked_sub(v.input_ns)
                        .is_some_and(|age| age <= TIMEOUT_NS)
                });
                if !fresh || command.as_ref().is_none_or(|v| !v.connected) {
                    rearm = true;
                }
                if fresh
                    && command
                        .as_ref()
                        .is_some_and(|v| v.connected && !v.enable_held && v.neutral)
                {
                    rearm = false;
                }
                let enabled = fresh
                    && !rearm
                    && command
                        .as_ref()
                        .is_some_and(|v| v.guarded && v.enabled && v.enable_held && v.connected);
                let speeds = if enabled {
                    command.as_ref().unwrap().velocities.clone()
                } else {
                    vec![0.0; 3]
                };
                bus.drive(&speeds, enabled)?;
                let message = if !fresh {
                    "Input timed out · release L1"
                } else if rearm {
                    "Release L1 and center sticks"
                } else {
                    command.as_ref().unwrap().status.as_str()
                };
                let value = json!({"phase":"ready","mock":c.mock,"enabled":enabled,"status":message,"controller":command.as_ref().map(|v|&v.controller),"command_ticks":speeds.iter().map(|v|*v as i64).collect::<Vec<_>>(),"motor_ids":IDS,"acks":bus.acks,"input_age_ms":command.as_ref().map(|v|now.saturating_sub(v.input_ns) as f64*1e-6),"measured_feedback":false,"updated_ns":wall_ns()});
                write_json(&c.output.join("status.json"), &value)?;
                *published.lock().unwrap() = value;
                thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        })();
        let stop = bus.stop();
        let error = outcome
            .as_ref()
            .err()
            .map(ToString::to_string)
            .or_else(|| stop.as_ref().err().map(ToString::to_string));
        let final_status = json!({"phase":if error.is_some(){"fault"}else{"stopped"},"mock":c.mock,"enabled":false,"status":error.clone().unwrap_or("Stopped · torque disabled".into()),"stop_confirmed":stop.is_ok(),"command_ticks":[0,0,0],"acks":bus.acks,"updated_ns":wall_ns()});
        write_json(&c.output.join("status.json"), &final_status)?;
        *published.lock().unwrap() = final_status;
        outcome.and(stop)
    });
    let result = (|| -> Result<()> {
        while !stopped.load(Ordering::Acquire) {
            let Some(event) = events.recv_timeout(Duration::from_millis(100)) else {
                break;
            };
            if stopped.load(Ordering::Acquire) {
                break;
            }
            match event {
                Event::Input { id, data, .. } => {
                    if id.as_str() == "command" {
                        let Message::Command(c) = crate::nodes::decode(&data)? else {
                            anyhow::bail!("Expected guarded base command")
                        };
                        validate(&c)?;
                        ensure!(c.guarded, "Unguarded base command");
                        let mut latest = latest.lock().unwrap();
                        ensure!(
                            latest.as_ref().is_none_or(|p| c.sequence > p.sequence),
                            "Reordered driver command"
                        );
                        *latest = Some(c);
                    } else {
                        let value = status.lock().unwrap().clone();
                        crate::nodes::send(node, "status", &Message::Info(value.to_string()))?;
                        ensure!(
                            value["phase"] != "fault",
                            "Base driver fault: {}",
                            value["status"]
                        );
                    }
                }
                Event::Stop(_) => break,
                Event::Error(e) if e.starts_with("Timeout event stream error:") => continue,
                Event::Error(e) => anyhow::bail!(e),
                _ => {}
            }
        }
        Ok(())
    })();
    stopped.store(true, Ordering::Release);
    let shutdown = worker
        .join()
        .map_err(|_| anyhow::anyhow!("Motor watchdog panicked"))?;
    result.and(shutdown)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vendor_directions_and_signed_words() {
        assert_eq!(speed_word(-300.0).unwrap(), 0x8000 | 300);
        assert!(speed_word(f64::NAN).is_err());
        let f = mix(300.0, 0.0, 0.0);
        assert!(f[0] < 0.0 && f[1] == 0.0 && f[2] > 0.0);
        let s = mix(0.0, 300.0, 0.0);
        assert_eq!(s, [150.0, -300.0, 150.0]);
        assert_eq!(mix(0.0, 0.0, 300.0), [300.0; 3]);
    }
    #[test]
    fn acknowledgement_faults_never_confirm_stop() {
        for fault in ["timeout", "checksum", "wrong_id", "motor_error"] {
            let mut b = Bus {
                io: Mock {
                    fault: Some(fault),
                    ..Default::default()
                },
                acks: 0,
                torque: true,
            };
            assert!(b.stop().is_err());
            assert_eq!(b.io.writes.len(), 6);
            assert_eq!(b.acks, 0);
        }
    }
    #[test]
    fn speed_zero_precedes_torque_enable() {
        let mut b = Bus {
            io: Mock::default(),
            acks: 0,
            torque: false,
        };
        b.prepare().unwrap();
        b.io.writes.clear();
        b.drive(&[-300.0, 0.0, 300.0], true).unwrap();
        assert_eq!(b.io.writes.len(), 9);
        for p in &b.io.writes[..3] {
            assert_eq!(&p[5..8], &[46, 0, 0]);
        }
        for p in &b.io.writes[3..6] {
            assert_eq!(&p[5..7], &[40, 1]);
        }
        b.stop().unwrap();
        assert!(!b.torque);
    }
}
