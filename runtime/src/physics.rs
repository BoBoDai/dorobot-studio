//! Native adapter for the pinned Nexus engine. Robot motor/readback mapping follows
//! dimforge/nexus ca2cc91 (MIT OR Apache-2.0), without its Python bindings.
use crate::{
    control::{Model, SO101_JOINTS},
    protocol::*,
};
use anyhow::{Context, Result, ensure};
use glamx::{Mat3, Pose3, Quat, Vec3};
use khal::backend::Backend;
use nexus_viewer3d::{BackendType, NexusViewer, VisualTexture, rapier::prelude::*};
use nexus3d::prelude::{NexusPipeline, NexusState, RbdCoupling};
use pollster::block_on;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, fs::File, path::Path};

#[derive(Deserialize)]
struct Spec {
    name: String,
    size: [f32; 3],
    position: [f32; 3],
    color: [f32; 4],
    fixed: bool,
    #[serde(default)]
    round: bool,
}
struct Joint {
    link: usize,
    axis: JointAxis,
    coordinate: usize,
    kp: f32,
    kv: f32,
    force: f32,
}
struct Camera {
    name: &'static str,
    id: usize,
    body: RigidBodyHandle,
    local: Pose3,
}
pub struct Simulation {
    state: NexusState,
    pipeline: NexusPipeline,
    viewer: NexusViewer,
    root: RigidBodyHandle,
    joints: Vec<Joint>,
    slots: Vec<u32>,
    bodies: Vec<RigidBodyHandle>,
    cameras: Vec<Camera>,
    pub model: Model,
    pub objects: Vec<Object>,
    pub camera_specs: Value,
    pub initial_objects: Vec<[f32; 3]>,
    pub targets: Vec<f64>,
    pub tick: u64,
    robot_ids: Vec<u32>,
    furniture_ids: Vec<u32>,
    cube_id: u32,
    link_ids: HashMap<String, u32>,
}
fn gpu<T, E: std::fmt::Debug>(r: std::result::Result<T, E>) -> Result<T> {
    r.map_err(|e| anyhow::anyhow!("Nexus: {e:?}"))
}
fn active_pairs(
    cs: &[nexus3d::rbd::queries::GpuIndexedContact],
    live: usize,
    tick: u64,
) -> Vec<[i64; 3]> {
    cs.iter()
        .take(live)
        .filter(|c| {
            c.contact
                .points_a
                .iter()
                .take(c.contact.len as usize)
                .any(|p| p.dist.is_finite() && p.dist <= 0.0015)
        })
        .map(|c| [tick as i64, c.bodies.x as i64, c.bodies.y as i64])
        .collect()
}
fn axis(n: usize) -> JointAxis {
    [
        JointAxis::LinX,
        JointAxis::LinY,
        JointAxis::LinZ,
        JointAxis::AngX,
        JointAxis::AngY,
        JointAxis::AngZ,
    ][n]
}
fn matrix(p: Pose3) -> [[f64; 4]; 4] {
    let c = p.to_mat4().to_cols_array_2d();
    std::array::from_fn(|i| std::array::from_fn(|j| c[j][i] as f64))
}
fn mount(position: [f32; 3], forward: Vec3, up: Vec3) -> Pose3 {
    let z = -forward.normalize();
    let x = up.cross(z).normalize();
    let y = z.cross(x);
    Pose3::from_parts(position.into(), Quat::from_mat3(&Mat3::from_cols(x, y, z)))
}
fn visual_pose(origin: &urdf_rs::Pose) -> Pose3 {
    Pose3::from_parts(
        Vec3::new(
            origin.xyz[0] as f32,
            origin.xyz[1] as f32,
            origin.xyz[2] as f32,
        ),
        Quat::from_rotation_z(origin.rpy[2] as f32)
            * Quat::from_rotation_y(origin.rpy[1] as f32)
            * Quat::from_rotation_x(origin.rpy[0] as f32),
    )
}
impl Simulation {
    pub fn actuator_settings(&self) -> Value {
        json!(self.model.names.iter().zip(&self.joints).map(|(name,j)|json!({"joint":name,"effort_limit":j.force,"stiffness":j.kp,"damping":j.kv})).collect::<Vec<_>>())
    }
    pub fn new(path: &Path) -> Result<Self> {
        Self::with_scene(path, None)
    }
    /// Experimental AISLE scene, supplied explicitly; the Studio default stays fixed.
    pub fn with_scene(path: &Path, scene: Option<&Path>) -> Result<Self> {
        Self::with_model(path, scene, Model::read_nano(path)?)
    }

    pub fn with_so101_scene(path: &Path, scene: &Path) -> Result<Self> {
        let model = Model::read(path)?;
        ensure!(
            model.names.len() == SO101_JOINTS.len(),
            "Expected a six-joint SO-101 URDF"
        );
        Self::with_model(path, Some(scene), model)
    }

    fn with_model(path: &Path, scene: Option<&Path>, model: Model) -> Result<Self> {
        let so101 = model.names.len() == SO101_JOINTS.len();
        let specs: Vec<Spec> = if let Some(scene) = scene {
            serde_json::from_slice(&std::fs::read(scene)?)
                .with_context(|| format!("Invalid scene {}", scene.display()))?
        } else {
            serde_json::from_str(include_str!("../assets/living-room.json"))?
        };
        ensure!(!specs.is_empty(), "Empty scene");
        let mut names = std::collections::HashSet::new();
        for spec in &specs {
            ensure!(
                !spec.name.is_empty()
                    && names.insert(&spec.name)
                    && spec.size.iter().all(|v| v.is_finite() && *v > 0.)
                    && spec
                        .position
                        .iter()
                        .chain(&spec.color)
                        .all(|v| v.is_finite())
                    && (!spec.round || spec.size[0] == spec.size[1]),
                "Invalid scene object {}",
                spec.name
            );
        }
        if so101 {
            for (name, fixed) in [("floor", true), ("cube_6", false), ("plate", true)] {
                ensure!(
                    specs
                        .iter()
                        .any(|spec| spec.name == name && spec.fixed == fixed),
                    "SO-101 scene requires a {} object named {name}",
                    if fixed { "fixed" } else { "dynamic" }
                );
            }
        }

        #[cfg(target_os = "macos")]
        let backend = BackendType::Metal;
        #[cfg(not(target_os = "macos"))]
        let backend = BackendType::Gpu;
        let mut viewer = block_on(NexusViewer::new_headless_with_size(Vec::new(), 64, 64))
            .with_backend(backend)
            .with_deterministic(true);
        if so101 {
            // Headless captures may return before rendering. Warm this lazy
            // manager while Kiss3D's texture cache is still available.
            kiss3d::resource::MaterialManager2d::get_global_manager(|_| ());
        }
        viewer.init_backend();
        viewer.add_directional_light(Vec3::new(0.3, -0.4, -1.0));
        viewer.set_sensor_rendering(true);
        viewer.set_sensor_antialiasing(if so101 { 1 } else { 4 });
        viewer.set_sensor_shadow_softness(1.0);
        viewer.set_sensor_shadow_range(2.5, 20.0);
        viewer.set_sensor_shadow_resolution(if so101 { 1024 } else { 4096 }, 4);
        let mut state = NexusState::default();
        state.set_rbd_timestep(DT, 2);
        let mut params = state
            .rbd_sim_params(0)
            .context("Missing physics environment")?;
        params.contact_natural_frequency = 240.0;
        params.static_contact_natural_frequency = 480.0;
        params.normalized_allowed_linear_error = 0.0005;
        params.num_internal_pgs_iterations = 8;
        params.friction_in_bias_pass = 1;
        state.set_rbd_sim_params(0, params);
        state.set_rbd_implicit_coriolis(viewer.backend(), false);
        state.set_rbd_substep_refresh(false, true);
        state.set_deterministic(viewer.backend(), true);
        let mut bodies = Vec::new();
        let mut objects = Vec::new();
        for spec in &specs {
            let rb = if spec.fixed {
                RigidBodyBuilder::fixed()
            } else {
                RigidBodyBuilder::dynamic()
            };
            let shape = if spec.round {
                ColliderBuilder::cylinder(spec.size[2] / 2.0, spec.size[0] / 2.0)
                    .rotation(Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0))
            } else {
                ColliderBuilder::cuboid(spec.size[0] / 2.0, spec.size[1] / 2.0, spec.size[2] / 2.0)
            };
            let pose = shape.position;
            let collider = shape
                .density(500.0)
                .friction(if so101 { 0.9 } else { 0.7 })
                .friction_combine_rule(CoefficientCombineRule::Max)
                .build();
            let shape = collider.shared_shape().clone();
            let body = state.insert_rigid_body(
                rb.translation(Vec3::from(spec.position)).build(),
                collider,
                RbdCoupling::None,
            );
            viewer.insert_visual_mesh_textured(
                0,
                body,
                &shape,
                pose,
                spec.color,
                None,
                None,
                VisualTexture::None,
                None,
            );
            bodies.push(body);
            objects.push(Object {
                name: spec.name.clone(),
                size: spec.size,
                color: spec.color,
            });
        }
        let options = rapier3d_urdf::UrdfLoaderOptions {
            make_roots_fixed: so101,
            mesh_converter: Some(MeshConverter::ConvexHull),
            squeeze_empty_fixed_links: false,
            ..Default::default()
        };
        let (robot, urdf) = rapier3d_urdf::UrdfRobot::from_file(path, options, None)?;
        let world = state.rbd_world_mut(0);
        let handles = robot.insert_using_multibody_joints(
            &mut world.bodies,
            &mut world.colliders,
            &mut world.multibody_joints,
            rapier3d_urdf::UrdfMultibodyOptions::DISABLE_SELF_CONTACTS,
        );
        for (handle, link) in handles.links.iter().zip(&urdf.links) {
            ensure!(
                link.collision.is_empty() || !handle.colliders.is_empty(),
                "Collision geometry was not loaded for {}; enable the URDF mesh format",
                link.name
            );
        }
        let links: HashMap<_, _> = handles
            .links
            .iter()
            .zip(&urdf.links)
            .map(|(h, l)| (l.name.clone(), h.body))
            .collect();
        let joint_names: HashMap<_, _> = handles
            .joints
            .iter()
            .zip(&urdf.joints)
            .map(|(h, j)| (h.link2, j.name.clone()))
            .collect();
        let root = handles.links.first().context("Empty URDF")?.body;
        let mb_id = world
            .multibody_joints
            .rigid_body_link(root)
            .context("Missing multibody")?
            .multibody;
        let mb = world
            .multibody_joints
            .get_multibody_mut(mb_id)
            .context("Missing multibody")?;
        mb.armature_mut().fill(0.01);
        mb.damping_mut().fill(0.1);
        let mut mapping = HashMap::new();
        let mut robot_bodies = Vec::new();
        for (index, link) in mb.links().enumerate() {
            robot_bodies.push(link.rigid_body_handle());
            if let Some(name) = joint_names.get(&link.rigid_body_handle()) {
                let free: Vec<_> = (0..6)
                    .filter(|i| link.joint().data.locked_axes.bits() & (1 << i) == 0)
                    .collect();
                if free.is_empty() {
                    continue;
                }
                ensure!(free.len() == 1, "Robot joint {name} must have one DoF");
                let a = axis(free[0]);
                let motor = link.joint().data.motor(a);
                mapping.insert(
                    name.clone(),
                    Joint {
                        link: index,
                        axis: a,
                        coordinate: free[0],
                        kp: motor.map_or(100.0, |m| {
                            if m.stiffness > 0.0 {
                                m.stiffness
                            } else {
                                100.0
                            }
                        }),
                        kv: motor.map_or(10.0, |m| if m.damping > 0.0 { m.damping } else { 10.0 }),
                        force: motor.map_or(f32::INFINITY, |m| m.max_force),
                    },
                );
            }
        }
        let mut joints: Vec<Joint> = model
            .names
            .iter()
            .map(|n| {
                mapping
                    .remove(n)
                    .with_context(|| format!("Missing motor {n}"))
            })
            .collect::<Result<Vec<_>>>()?;
        if scene.is_some() {
            // Rapier's URDF importer does not apply effort limits. The AISLE
            // profile enforces them explicitly, including the 1 Nm grippers.
            for (name, joint) in model.names.iter().zip(&mut joints) {
                let spec = urdf
                    .joints
                    .iter()
                    .find(|j| j.name == *name)
                    .context("Motor URDF limit")?;
                ensure!(
                    spec.limit.effort.is_finite() && spec.limit.effort > 0.,
                    "Invalid effort limit"
                );
                joint.force = spec.limit.effort as f32;
                if name.ends_with("j6_gripper") {
                    joint.kp = 10.;
                    joint.kv = 0.5;
                }
            }
        }
        // The authored STL visuals preserve the orange arms, black lid and screen.
        let materials: HashMap<_, _> = urdf
            .materials
            .iter()
            .filter_map(|m| {
                m.color
                    .as_ref()
                    .map(|c| (m.name.clone(), c.rgba.0.map(|v| v as f32)))
            })
            .collect();
        for link in &urdf.links {
            for visual in &link.visual {
                let color = visual
                    .material
                    .as_ref()
                    .and_then(|m| {
                        m.color
                            .as_ref()
                            .map(|c| c.rgba.0.map(|v| v as f32))
                            .or_else(|| materials.get(&m.name).copied())
                    })
                    .unwrap_or([0.7, 0.7, 0.7, 1.0]);
                let shape = match &visual.geometry {
                    urdf_rs::Geometry::Mesh { filename, scale } => {
                        let filename = filename
                            .strip_prefix("package://adora_nano/")
                            .unwrap_or(filename);
                        let mut file =
                            File::open(path.parent().context("Model directory")?.join(filename))?;
                        let mesh = stl_io::read_stl(&mut file)?;
                        let scale = scale.as_ref().map(|v| v.0).unwrap_or([1.0; 3]);
                        let vertices = mesh
                            .vertices
                            .iter()
                            .map(|v| {
                                Vec3::new(
                                    v[0] * scale[0] as f32,
                                    v[1] * scale[1] as f32,
                                    v[2] * scale[2] as f32,
                                )
                            })
                            .collect();
                        let indices = mesh
                            .faces
                            .iter()
                            .filter(|f| {
                                f.vertices[0] != f.vertices[1]
                                    && f.vertices[1] != f.vertices[2]
                                    && f.vertices[0] != f.vertices[2]
                            })
                            .map(|f| f.vertices.map(|i| i as u32))
                            .collect();
                        SharedShape::trimesh(vertices, indices)?
                    }
                    urdf_rs::Geometry::Box { size } => SharedShape::cuboid(
                        size[0] as f32 / 2.0,
                        size[1] as f32 / 2.0,
                        size[2] as f32 / 2.0,
                    ),
                    urdf_rs::Geometry::Sphere { radius } => SharedShape::ball(*radius as f32),
                    _ => anyhow::bail!("Unsupported robot visual geometry"),
                };
                viewer.insert_visual_mesh_textured(
                    0,
                    links[&link.name],
                    &shape,
                    visual_pose(&visual.origin),
                    color,
                    None,
                    None,
                    VisualTexture::None,
                    None,
                );
            }
        }
        gpu(block_on(state.finalize(viewer.backend())))?;
        state.set_rbd_gravity(viewer.backend(), [0.0, 0.0, -9.81]);
        let q = state
            .multibody_joint_positions(0, root)
            .context("Initial robot coordinates")?;
        gpu(state.set_multibody_joint_positions(viewer.backend(), 0, root, &q))?;
        let (_, _, slots) = state
            .multibody_link_slots(0, root)
            .context("GPU link map")?;
        let slots = slots.into_iter().map(|(_, s)| s).collect();
        for (i, body) in bodies.iter().chain(&robot_bodies).enumerate() {
            viewer.set_body_segmentation_id(0, *body, i as u32 + 1);
        }
        let furniture_ids = bodies
            .iter()
            .zip(&specs)
            .filter(|(_, s)| s.fixed && s.name != "floor")
            .map(|(b, _)| state.rigid_body_gpu_index(0, *b).unwrap())
            .collect();
        let robot_ids = robot_bodies
            .iter()
            .map(|b| state.rigid_body_gpu_index(0, *b).unwrap())
            .collect();
        let cube_id = state
            .rigid_body_gpu_index(
                0,
                bodies[objects
                    .iter()
                    .position(|o| o.name == "cube_6")
                    .context("Scene needs tracked cube_6")?],
            )
            .unwrap();
        let mut cameras = Vec::new();
        let mut camera_specs = serde_json::Map::new();
        let nano_mounts = [
            (
                "head",
                "head_tilt",
                "overhead",
                mount([0.03, 0.018, 0.0], Vec3::Y, Vec3::X),
            ),
            (
                "left_wrist",
                "left_wrist_cam",
                "left_wrist",
                mount([0.073, 0.0, 0.0], Vec3::new(1.0, 0.0, 1.0), -Vec3::Z),
            ),
            (
                "right_wrist",
                "right_wrist_cam",
                "right_wrist",
                mount([0.073, 0.0, 0.0], Vec3::new(1.0, 0.0, 1.0), -Vec3::Z),
            ),
        ];
        let so101_mount = [(
            "front",
            "base_link",
            "front",
            mount([0.50, -0.42, 0.49], Vec3::new(-0.34, 0.43, -0.39), Vec3::Z),
        )];
        let mounts = if so101 {
            &so101_mount[..]
        } else {
            &nano_mounts[..]
        };
        let camera_fps = if so101 { 20 } else { FPS };
        for &(name, link, directory, local) in mounts {
            let body = *links
                .get(link)
                .with_context(|| format!("Missing camera mount {link}"))?;
            let id = block_on(viewer.add_sensor_camera(
                WIDTH as u32,
                HEIGHT as u32,
                70_f32.to_radians(),
                0.01,
                50.0,
            ));
            let sensor = viewer.sensor_camera_mut(id).context("Missing sensor")?;
            sensor.set_ambient(1.0);
            sensor.set_ambient_color([0.6; 3]);
            sensor.set_background_color([0.07, 0.08, 0.12, 1.0]);
            viewer.attach_sensor_camera(id, 0, links[link], local, &state);
            let focal = HEIGHT as f64 / (2.0 * 35_f64.to_radians().tan());
            camera_specs.insert(name.into(),json!({"video_key":format!("observation.images.{name}"),"source_directory":directory,"parent_link":link,"resolution":[WIDTH,HEIGHT],"fps":camera_fps,"vertical_fov_deg":70,"intrinsics":[[focal,0.0,320.0],[0.0,focal,240.0],[0.0,0.0,1.0]],"link_from_camera_gl":matrix(local),"camera_convention":"OpenGL: +X right, +Y up, -Z forward","optical_calibration":"nominal simulation assumption; not measured hardware calibration","channels":["rgb","depth","seg","world_from_camera_gl","world_from_mount_link"],"depth_units":"metres, axial camera depth; far plane for background","segmentation":"simulator entity IDs; -1 background; not a hardware sensor"}));
            cameras.push(Camera {
                name,
                id,
                body,
                local,
            });
        }
        let link_ids = links
            .iter()
            .map(|(name, body)| (name.clone(), state.rigid_body_gpu_index(0, *body).unwrap()))
            .collect();
        let joint_count = model.names.len();
        let mut out = Self {
            state,
            pipeline: NexusPipeline::default(),
            viewer,
            root,
            joints,
            slots,
            bodies,
            cameras,
            model,
            objects,
            camera_specs: Value::Object(camera_specs),
            initial_objects: Vec::new(),
            targets: vec![0.0; joint_count],
            tick: 0,
            robot_ids,
            furniture_ids,
            cube_id,
            link_ids,
        };
        let parked: Vec<f64> = out
            .model
            .names
            .iter()
            .map(|n| match n.as_str() {
                "gripper" if so101 => 0.6,
                "head_pan_joint" => 1.5,
                "head_tilt_joint" => -0.22,
                "left_j1_yaw" | "right_j1_yaw" => 0.2,
                "left_j3_pitch" | "right_j3_pitch" => 0.35,
                "left_j4_pitch" | "right_j4_pitch" => -1.0,
                _ => 0.0,
            })
            .collect();
        for step in 0..250 {
            out.targets = parked
                .iter()
                .map(|v| v * ((step + 1) as f64 / 150.0).min(1.0))
                .collect();
            out.step(false)?;
        }
        out.tick = 0;
        out.targets = parked;
        let poses = block_on(out.state.read_rigid_body_poses(out.viewer.backend()));
        out.initial_objects = out
            .bodies
            .iter()
            .map(|b| {
                poses[out.state.rigid_body_gpu_index(0, *b).unwrap() as usize]
                    .translation
                    .to_array()
            })
            .collect();
        Ok(out)
    }

    pub fn set_so101_joint_positions(&mut self, positions: [f64; 6]) -> Result<()> {
        ensure!(
            self.joints.len() == SO101_JOINTS.len() && positions.iter().all(|q| q.is_finite()),
            "Expected six finite SO-101 joint positions"
        );
        let targets = positions
            .iter()
            .zip(&self.model.limits)
            .map(|(position, [lo, hi])| position.clamp(*lo, *hi))
            .collect::<Vec<_>>();
        // Rapier consumes coordinates in assembly-link order, which need not
        // match the named LeRobot command protocol.
        let mut assembly = self
            .joints
            .iter()
            .zip(&targets)
            .map(|(joint, position)| (joint.link, *position as f32))
            .collect::<Vec<_>>();
        assembly.sort_unstable_by_key(|(link, _)| *link);
        let coordinates = assembly
            .into_iter()
            .map(|(_, position)| position)
            .collect::<Vec<_>>();
        gpu(self.state.set_multibody_joint_positions(
            self.viewer.backend(),
            0,
            self.root,
            &coordinates,
        ))?;
        self.targets = targets;
        Ok(())
    }

    pub fn reset_tabletop_cube(&mut self) -> Result<()> {
        let index = self
            .objects
            .iter()
            .position(|object| object.name == "cube_6")
            .context("Cube is missing from the tabletop")?;
        let cube = self.bodies[index];
        let mut position = Vec3::from(self.initial_objects[index]);
        position.z += 0.002;
        let backend = self.viewer.backend();
        gpu(self.state.set_rigid_body_pose(
            backend,
            0,
            cube,
            Pose3::translation(position.x, position.y, position.z),
        ))?;
        gpu(self
            .state
            .set_rigid_body_velocity(backend, 0, cube, Vec3::ZERO, Vec3::ZERO))?;
        Ok(())
    }

    pub fn step(&mut self, contacts: bool) -> Result<Vec<[i64; 3]>> {
        gpu(self
            .state
            .control_multibody_motors_env(self.viewer.backend(), 0, |world| {
                let mb_id = world
                    .multibody_joints
                    .rigid_body_link(self.root)
                    .unwrap()
                    .multibody;
                let mb = world.multibody_joints.get_multibody_mut(mb_id).unwrap();
                for (j, q) in self.joints.iter().zip(&self.targets) {
                    let data = &mut mb.link_mut(j.link).unwrap().joint.data;
                    data.set_motor_model(j.axis, MotorModel::ForceBased);
                    data.set_motor_position(j.axis, *q as f32, j.kp, j.kv);
                    data.set_motor_max_force(j.axis, j.force);
                }
            }))?;
        gpu(block_on(self.pipeline.simulate(
            self.viewer.backend(),
            &mut self.state,
            None,
        )))?;
        self.tick += 1;
        if !contacts {
            return Ok(Vec::new());
        }
        let rbd = self.state.rbd.as_ref().context("Rigid body state")?;
        let cs: Vec<nexus3d::rbd::queries::GpuIndexedContact> = gpu(block_on(
            self.viewer.backend().slow_read_vec(rbd.contacts().buffer()),
        ))?;
        let plan = gpu(block_on(
            self.viewer
                .backend()
                .slow_read_vec(rbd.contact_plan_buffer().buffer()),
        ))?;
        let live = plan.first().context("Missing contact plan")?.bound as usize;
        ensure!(live <= cs.len(), "Contact plan exceeds buffer");
        Ok(active_pairs(&cs, live, self.tick))
    }
    pub fn measured(&self) -> Result<Vec<f64>> {
        let ws = block_on(self.state.read_multibody_links(self.viewer.backend()));
        self.joints
            .iter()
            .map(|j| {
                ws.get(self.slots[j.link] as usize)
                    .map(|w| w.coords[j.coordinate] as f64)
                    .context("GPU joint readback missing")
            })
            .collect()
    }
    /// Independent contact evidence. Both physical fingers must contact the cube.
    pub fn gripper_contact(&self, side: &str, contacts: &[[i64; 3]]) -> bool {
        let mut ticks = HashMap::<i64, u8>::new();
        for (i, finger) in ["finger_a", "finger_b"].iter().enumerate() {
            if let Some(id) = self.link_ids.get(&format!("{side}_{finger}")) {
                for c in contacts {
                    if (c[1] as u32 == *id && c[2] as u32 == self.cube_id)
                        || (c[2] as u32 == *id && c[1] as u32 == self.cube_id)
                    {
                        *ticks.entry(c[0]).or_default() |= 1 << i;
                    }
                }
            }
        }
        ticks.values().any(|mask| *mask == 3)
    }
    pub fn frame(
        &mut self,
        sequence: u64,
        recorded: bool,
        contacts: Vec<[i64; 3]>,
        velocity: Vec<f64>,
        phase: String,
    ) -> Result<Frame> {
        gpu(block_on(self.viewer.sync(&mut self.state, None)))?;
        let base = self
            .viewer
            .cached_body_pose(&self.state, 0, self.root)
            .context("Base readback")?;
        let mut observation: Vec<f64> = base.translation.to_array().map(f64::from).to_vec();
        observation.extend(
            [
                base.rotation.w,
                base.rotation.x,
                base.rotation.y,
                base.rotation.z,
            ]
            .map(f64::from),
        );
        observation.extend(self.measured()?);
        observation.extend(&self.targets);
        let transforms = self
            .bodies
            .iter()
            .map(|b| {
                self.viewer
                    .cached_body_pose(&self.state, 0, *b)
                    .map(|p| p.to_mat4().to_cols_array())
                    .context("Object readback")
            })
            .collect::<Result<_>>()?;
        let mut cameras = Vec::new();
        for c in &self.cameras {
            let rgb = block_on(self.viewer.render_sensor_rgb(c.id)).context("RGB render")?;
            let depth = if recorded {
                self.viewer
                    .render_sensor_depth(c.id)
                    .context("Depth render")?
                    .into_iter()
                    .map(|v| if v == 0.0 { 50.0 } else { v })
                    .collect()
            } else {
                Vec::new()
            };
            let seg = if recorded {
                self.viewer
                    .render_sensor_segmentation(c.id)
                    .context("Segmentation render")?
                    .into_iter()
                    .map(|v| if v == 0 { -1 } else { v as i64 })
                    .collect()
            } else {
                Vec::new()
            };
            let parent = self
                .viewer
                .cached_body_pose(&self.state, 0, c.body)
                .context("Camera mount readback")?;
            cameras.push(Sensor {
                name: c.name.into(),
                rgb,
                depth,
                seg,
                world_from_camera_gl: matrix(parent * c.local),
                world_from_mount_link: matrix(parent),
            });
        }
        let cube_contact = contacts.iter().any(|c| {
            (c[1] as u32 == self.cube_id && self.robot_ids.contains(&(c[2] as u32)))
                || (c[2] as u32 == self.cube_id && self.robot_ids.contains(&(c[1] as u32)))
        });
        let frame = Frame {
            sequence,
            sim_time_ns: if recorded {
                (sequence + 1) * FRAME_NS
            } else {
                self.tick * 10_000_000
            },
            source_sim_time_ns: self.tick * 10_000_000,
            observation,
            transforms,
            cameras,
            contacts: contacts
                .iter()
                .filter(|c| {
                    (self.robot_ids.contains(&(c[1] as u32))
                        && self.furniture_ids.contains(&(c[2] as u32)))
                        || (self.robot_ids.contains(&(c[2] as u32))
                            && self.furniture_ids.contains(&(c[1] as u32)))
                })
                .map(|c| [c[1], c[2]])
                .collect(),
            physics_contacts: contacts,
            cube_contact,
            command_velocity: velocity,
            phase,
        };
        if self.joints.len() == SO101_JOINTS.len() {
            ensure!(
                frame.observation.len() == 19 && frame.observation.iter().all(|v| v.is_finite()),
                "Invalid SO-101 observation"
            );
            ensure!(
                frame.cameras.len() == 1
                    && frame.cameras[0].name == "front"
                    && frame.cameras[0].rgb.len() == WIDTH * HEIGHT * 3,
                "Expected one 640×480 SO-101 front RGB frame"
            );
        } else {
            frame.validate(recorded)?;
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod contact_tests {
    use super::*;
    /// TC-8: allocated GPU tail slots and separated candidates are not contacts.
    #[test]
    fn ignores_stale_contact_capacity_and_positive_separation() {
        let mut c = nexus3d::rbd::queries::GpuIndexedContact::default();
        c.contact.len = 1;
        c.bodies.x = 2;
        c.bodies.y = 7;
        let mut separated = c;
        separated.contact.points_a[0].dist = 0.1;
        assert_eq!(active_pairs(&[c, separated, c], 2, 8), vec![[8, 2, 7]]);
        assert!(active_pairs(&[c], 0, 8).is_empty());
    }
}
