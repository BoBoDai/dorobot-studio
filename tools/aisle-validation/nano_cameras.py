"""Three link-mounted Nano sensors; nominal simulation optics, not calibration.

The URDF has camera housings but no optical frames or lens intrinsics. Keep the
assumed optical transforms explicit, and record the realized world pose at every
capture. All sensors are rendered without advancing physics between them.
"""

import math

import numpy as np


def create_rig(scene, robot, resolution=(640, 480)):
    from aisle.sim.nexus_backend import pos_lookat_up_to_transform

    definitions = [
        ("head", "head_tilt", "overhead", [0.03, 0.018, 0], [0, 1, 0], [1, 0, 0]),
        (
            "left_wrist",
            "left_wrist_cam",
            "left_wrist",
            [0.073, 0, 0],
            [1, 0, 1],
            [0, 0, -1],
        ),
        (
            "right_wrist",
            "right_wrist_cam",
            "right_wrist",
            [0.073, 0, 0],
            [1, 0, 1],
            [0, 0, -1],
        ),
    ]
    rig, specs = {}, {}
    width, height = resolution
    fov = 70.0
    focal = height / (2 * math.tan(math.radians(fov) / 2))
    for name, link, directory, position, forward, up in definitions:
        offset = pos_lookat_up_to_transform(
            position, np.asarray(position) + forward, up
        )
        camera = scene.add_camera((width, height), fov)
        camera.attach(robot.get_link(link), offset)
        rig[name] = camera
        specs[name] = dict(
            video_key=f"observation.images.{name}",
            source_directory=directory,
            parent_link=link,
            resolution=[width, height],
            fps=10,
            vertical_fov_deg=fov,
            intrinsics=[[focal, 0, width / 2], [0, focal, height / 2], [0, 0, 1]],
            link_from_camera_gl=offset.tolist(),
            camera_convention="OpenGL: +X right, +Y up, -Z forward",
            optical_calibration="nominal simulation assumption; not measured hardware calibration",
            channels=[
                "rgb",
                "depth",
                "seg",
                "world_from_camera_gl",
                "world_from_mount_link",
            ],
            depth_units="metres, axial camera depth; far plane for background",
            segmentation="simulator entity IDs; -1 background; not a hardware sensor",
        )
    return rig, specs


def read_rig(rig, specs, stamp):
    """Yield validated sensor arrays at one tick, without choosing a transport."""
    from aisle.sim.nexus_backend import quat_wxyz_to_matrix

    for name, camera in rig.items():
        rgb, depth, seg, _ = camera.render(rgb=True, depth=True, segmentation=True)
        rgb = np.asarray(rgb, dtype=np.uint8)
        depth = np.asarray(depth, dtype=np.float32)
        seg = np.asarray(seg, dtype=np.int64)
        w, h = specs[name]["resolution"]
        if rgb.shape != (h, w, 3) or depth.shape != (h, w) or seg.shape != (h, w):
            raise ValueError(f"Invalid capture dimensions for {name}")
        pose = camera.transform
        link = camera._attached_link
        parent_pose = np.eye(4)
        parent_pose[:3, :3] = quat_wxyz_to_matrix(np.asarray(link.get_quat()))
        parent_pose[:3, 3] = np.asarray(link.get_pos())
        if not np.allclose(
            pose, parent_pose @ specs[name]["link_from_camera_gl"], atol=1e-5
        ):
            raise ValueError(
                f"Camera attachment does not match measured link pose: {name}"
            )
        if not np.isfinite(depth).all() or not np.isfinite(pose).all():
            raise ValueError(f"Invalid depth or camera pose for {name}")
        yield (
            name,
            dict(
                rgb=rgb,
                depth=depth,
                seg=seg,
                world_from_camera_gl=pose,
                world_from_mount_link=parent_pose,
                sim_time_ns=np.int64(stamp),
            ),
        )


def capture_rig(rig, specs, root, stamp):
    for name, arrays in read_rig(rig, specs, stamp):
        directory = root / specs[name]["source_directory"]
        directory.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(directory / f"{stamp:020d}.npz", **arrays)
