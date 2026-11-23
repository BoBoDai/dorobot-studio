use crate::protocol::{Command, Input};
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, path::Path};

pub const TIMEOUT_NS: u64 = 250_000_000;
pub const MODES: [&str; 4] = ["Drive", "Left arm", "Right arm", "Head"];
pub const SO101_JOINTS: [&str; 6] = [
    "shoulder_pan",
    "shoulder_lift",
    "elbow_flex",
    "wrist_flex",
    "wrist_roll",
    "gripper",
];

#[derive(Clone, Debug)]
pub struct Model {
    pub names: Vec<String>,
    pub limits: Vec<[f64; 2]>,
}
impl Model {
    pub fn read(path: &Path) -> Result<Self> {
        let robot = urdf_rs::read_file(path)?;
        let mut names = Vec::new();
        let mut limits = Vec::new();
        for j in robot.joints {
            if matches!(
                j.joint_type,
                urdf_rs::JointType::Fixed | urdf_rs::JointType::Floating
            ) {
                continue;
            }
            names.push(j.name);
            limits.push(if j.joint_type == urdf_rs::JointType::Continuous {
                [f64::NEG_INFINITY, f64::INFINITY]
            } else {
                [j.limit.lower, j.limit.upper]
            });
        }
        if names.len() == SO101_JOINTS.len() {
            limits = SO101_JOINTS
                .iter()
                .map(|name| {
                    let index = names
                        .iter()
                        .position(|joint| joint == name)
                        .ok_or_else(|| anyhow::anyhow!("Missing SO-101 joint {name}"))?;
                    let [lower, upper] = limits[index];
                    ensure!(
                        lower.is_finite() && upper.is_finite() && lower < upper,
                        "Invalid limits for SO-101 joint {name}"
                    );
                    Ok([lower, upper])
                })
                .collect::<Result<Vec<_>>>()?;
            names = SO101_JOINTS.map(str::to_owned).to_vec();
        } else {
            ensure!(names.len() == 17, "Unsupported model joint count");
        }
        Ok(Self { names, limits })
    }

    pub fn read_nano(path: &Path) -> Result<Self> {
        let model = Self::read(path)?;
        ensure!(
            model.names.len() == 17,
            "Nano must have 17 controllable joints"
        );
        Ok(model)
    }
}
pub fn axis(value: f64) -> Result<f64> {
    ensure!(
        value.is_finite() && value.abs() <= 1.01,
        "Invalid controller axis"
    );
    Ok(((value.abs() - 0.12).max(0.0) / 0.88).copysign(value))
}
pub fn wheel_rates(forward: f64, lateral: f64, turn: f64) -> [f64; 3] {
    // Same effective omni-wheel model as the accepted Python scene.
    let r = 0.034;
    let mut rates = [
        lateral / r - 3.4 * turn,
        (3.0_f64.sqrt() / 2.0 * forward - 0.5 * lateral) / r - 3.4 * turn,
        (-3.0_f64.sqrt() / 2.0 * forward - 0.5 * lateral) / r - 3.4 * turn,
    ];
    let scale = (5.0 / rates.iter().copied().map(f64::abs).fold(1e-9, f64::max)).min(1.0);
    for rate in &mut rates {
        *rate *= scale;
    }
    rates
}
pub struct Mapper {
    names: Vec<String>,
    mode: usize,
    paused: bool,
    ready: bool,
    connected: bool,
    previous: BTreeMap<String, bool>,
}
impl Mapper {
    pub fn new(names: Vec<String>) -> Self {
        Self {
            names,
            mode: 0,
            paused: false,
            ready: false,
            connected: false,
            previous: BTreeMap::new(),
        }
    }
    pub fn command(&mut self, input: &Input) -> Result<Command> {
        let mut axes = [0.0; 4];
        for (out, name) in axes
            .iter_mut()
            .zip(["left_x", "left_y", "right_x", "right_y"])
        {
            *out = axis(input.axes.get(name).copied().unwrap_or(0.0))?;
        }
        let held = input.button("enable");
        let neutral = axes.iter().all(|x| *x == 0.0)
            && ["up", "down", "left", "right", "open", "close"]
                .iter()
                .all(|b| !input.button(b));
        if !input.connected || !self.connected {
            self.ready = false;
        }
        if input.connected && !held && neutral {
            self.ready = true;
        }
        if input.button("pause") && !self.previous.get("pause").copied().unwrap_or(false) {
            self.paused = !self.paused;
            self.ready = false;
        }
        if input.button("mode") && !self.previous.get("mode").copied().unwrap_or(false) {
            self.mode = (self.mode + 1) % 4;
            self.ready = false;
        }
        let enabled = input.connected && self.ready && held && !self.paused;
        let mut velocity = vec![0.0; self.names.len()];
        let mut put = |name: &str, v: f64| -> Result<()> {
            let i = self
                .names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| anyhow::anyhow!("Unknown joint {name}"))?;
            velocity[i] = v;
            Ok(())
        };
        let buttons =
            |pos: &str, neg: &str| input.button(pos) as u8 as f64 - input.button(neg) as u8 as f64;
        if enabled {
            if self.mode == 0 {
                let forward = if axes[1] != 0.0 {
                    -axes[1]
                } else {
                    buttons("up", "down")
                };
                let turn = if axes[2] != 0.0 {
                    -axes[2]
                } else {
                    buttons("left", "right")
                };
                for (name, rate) in ["wheel_1_joint", "wheel_2_joint", "wheel_3_joint"]
                    .iter()
                    .zip(wheel_rates(forward * 0.12, 0.0, turn * 0.35))
                {
                    put(name, rate)?;
                }
            } else if self.mode <= 2 {
                let side = if self.mode == 1 { "left" } else { "right" };
                for (suffix, value) in [
                    "j1_yaw",
                    "j2_pitch",
                    "j3_pitch",
                    "j4_pitch",
                    "j5_roll",
                    "j6_gripper",
                ]
                .iter()
                .zip([
                    axes[0],
                    -axes[1],
                    -axes[3],
                    axes[2],
                    buttons("right", "left"),
                    buttons("close", "open"),
                ]) {
                    put(
                        &format!("{side}_{suffix}"),
                        value * if *suffix == "j6_gripper" { 0.5 } else { 0.3 },
                    )?;
                }
            } else {
                put("head_pan_joint", -axes[0] * 0.35)?;
                put("head_tilt_joint", -axes[1] * 0.35)?;
            }
        }
        self.connected = input.connected;
        self.previous = input.buttons.clone();
        let status = if !input.connected {
            "Controller disconnected"
        } else if self.paused {
            "Paused"
        } else if enabled && velocity.iter().any(|v| *v != 0.0) {
            "Moving"
        } else if enabled {
            "L1 held · move a stick or D-pad"
        } else if self.ready {
            "Hold L1 to move"
        } else {
            "Release L1 and center sticks"
        };
        Ok(Command {
            sequence: input.sequence,
            input_ns: input.input_ns,
            connected: input.connected,
            enabled,
            enable_held: held,
            mode: MODES[self.mode].into(),
            status: status.into(),
            velocities: velocity,
            guarded: false,
            neutral,
            controller: input.name.clone(),
        })
    }
}
pub struct Guard {
    names: Vec<String>,
    last: Vec<f64>,
    last_ns: Option<u64>,
    sequence: Option<u64>,
}
impl Guard {
    pub fn new(names: Vec<String>) -> Self {
        Self {
            last: vec![0.0; names.len()],
            names,
            last_ns: None,
            sequence: None,
        }
    }
    pub fn accept(&mut self, mut cmd: Command, now: u64) -> Result<Command> {
        ensure!(
            cmd.velocities.len() == self.names.len(),
            "Invalid joint command size"
        );
        ensure!(
            self.sequence.is_none_or(|s| cmd.sequence > s),
            "Reordered controller command"
        );
        for (name, v) in self.names.iter().zip(&cmd.velocities) {
            ensure!(
                v.is_finite()
                    && v.abs()
                        <= if name.starts_with("wheel_") {
                            5.0 + 1e-9
                        } else {
                            0.5 + 1e-9
                        },
                "Invalid joint velocity"
            );
        }
        self.sequence = Some(cmd.sequence);
        let fresh = now
            .checked_sub(cmd.input_ns)
            .is_some_and(|age| age <= TIMEOUT_NS);
        cmd.enabled = cmd.enabled && cmd.connected && fresh;
        let dt = self
            .last_ns
            .map_or(0.02, |n| (now.saturating_sub(n) as f64 * 1e-9).min(0.1));
        for ((name, prev), value) in self
            .names
            .iter()
            .zip(&mut self.last)
            .zip(&mut cmd.velocities)
        {
            let step = if name.starts_with("wheel_") {
                25.0
            } else {
                3.0
            } * dt;
            *value = if cmd.enabled {
                value.clamp(*prev - step, *prev + step)
            } else {
                0.0
            };
            *prev = *value;
        }
        self.last_ns = Some(now);
        cmd.guarded = true;
        if !fresh {
            cmd.status = "Input timed out".into();
        }
        Ok(cmd)
    }
}
pub fn active(cmd: Option<&Command>, now: u64) -> bool {
    cmd.is_some_and(|c| {
        c.guarded
            && c.enabled
            && c.connected
            && now
                .checked_sub(c.input_ns)
                .is_some_and(|age| age <= TIMEOUT_NS)
    })
}
pub fn integrate(targets: &mut [f64], velocity: &[f64], limits: &[[f64; 2]], dt: f64) {
    for ((q, v), [low, high]) in targets.iter_mut().zip(velocity).zip(limits) {
        *q = (*q + v * dt).clamp(*low, *high);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        Model::read(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/adora_nano/adora_nano.urdf"),
        )
        .unwrap()
    }
    fn input(seq: u64) -> Input {
        Input {
            sequence: seq,
            input_ns: seq * 20_000_000,
            connected: true,
            ..Default::default()
        }
    }
    #[test]
    fn so101_uses_leader_joint_order_with_its_own_limits() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/so101/so101_new_calib.urdf");
        let model = Model::read(&path).unwrap();
        assert_eq!(
            model.names,
            [
                "shoulder_pan",
                "shoulder_lift",
                "elbow_flex",
                "wrist_flex",
                "wrist_roll",
                "gripper"
            ]
        );
        assert_eq!(model.limits[0], [-1.91986, 1.91986]);
        assert_eq!(model.limits[5], [-0.174533, 1.74533]);
    }
    #[test]
    fn held_trigger_on_connect_cannot_move_until_release_and_neutral() {
        let mut m = Mapper::new(model().names);
        let mut i = input(0);
        i.buttons.insert("enable".into(), true);
        i.axes.insert("left_y".into(), -1.0);
        assert!(!m.command(&i).unwrap().enabled);
        i = input(1);
        assert!(!m.command(&i).unwrap().enabled);
        i = input(2);
        i.buttons.insert("enable".into(), true);
        i.axes.insert("left_y".into(), -1.0);
        assert!(m.command(&i).unwrap().velocities.iter().any(|x| *x != 0.0));
    }
    #[test]
    fn release_disconnect_timeout_and_bad_sequences_stop_or_reject() {
        let names = model().names;
        let mut m = Mapper::new(names.clone());
        let mut g = Guard::new(names);
        m.command(&input(0)).unwrap();
        let mut i = input(1);
        i.buttons.insert("enable".into(), true);
        i.axes.insert("left_y".into(), -1.0);
        let c = g.accept(m.command(&i).unwrap(), i.input_ns).unwrap();
        assert!(active(Some(&c), i.input_ns));
        assert!(!active(Some(&c), i.input_ns + TIMEOUT_NS + 1));
        assert!(g.accept(c.clone(), i.input_ns).is_err());
        i = input(2);
        let c = g.accept(m.command(&i).unwrap(), i.input_ns).unwrap();
        assert!(c.velocities.iter().all(|v| *v == 0.0));
        let mut c = c;
        c.sequence += 1;
        c.velocities[0] = f64::NAN;
        assert!(g.accept(c, 100_000_000).is_err());
    }
    #[test]
    fn both_arms_have_all_six_named_controls_and_equal_limits() {
        let model = model();
        for suffix in [
            "j1_yaw",
            "j2_pitch",
            "j3_pitch",
            "j4_pitch",
            "j5_roll",
            "j6_gripper",
        ] {
            let l = model
                .names
                .iter()
                .position(|n| n == &format!("left_{suffix}"))
                .unwrap();
            let r = model
                .names
                .iter()
                .position(|n| n == &format!("right_{suffix}"))
                .unwrap();
            assert_eq!(model.limits[l], model.limits[r]);
        }
    }
    #[test]
    fn target_integration_respects_joint_limits() {
        let mut targets = [0.95, -0.95];
        integrate(&mut targets, &[0.5, -0.5], &[[-1.0, 1.0]; 2], 1.0);
        assert_eq!(targets, [1.0, -1.0]);
        assert!(axis(f64::NAN).is_err());
        assert_eq!(axis(0.1).unwrap(), 0.0);
    }
}
