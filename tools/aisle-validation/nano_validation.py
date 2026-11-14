"""Fixed-base Nano simulation and AISLE -> Studio software-validation probe.

Uses AISLE's Nexus scene wrapper, Arrow trace schema and offline exporter.
This is an external integration probe, not a registered AISLE benchmark or a
hardware driver. Commands are joint-space motion checks, not manipulation tasks.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import math
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path


def load_model(path):
    root = ET.parse(path).getroot()
    joints = [j for j in root.findall("joint") if j.get("type") != "fixed"]
    names = [j.get("name") for j in joints]
    if len(names) != 17 or len(set(names)) != 17:
        raise ValueError("Expected the 17-channel Adora Nano model")
    groups = {
        side: [n for n in names if n.startswith(side + "_j")]
        for side in ("left", "right")
    }
    if any(len(g) != 6 for g in groups.values()):
        raise ValueError("Expected five joints and one gripper per arm")
    limits = {}
    for j in joints:
        lim = j.find("limit")
        limits[j.get("name")] = (
            (-math.inf, math.inf)
            if j.get("type") == "continuous"
            else (float(lim.get("lower")), float(lim.get("upper")))
        )
    return dict(
        names=names,
        limits=limits,
        held=[n for n in names if n not in groups["left"] + groups["right"]],
        **groups,
    )


def native_indices(wire_names, engine_names):
    if len(set(engine_names)) != len(engine_names) or set(wire_names) != set(
        engine_names
    ):
        raise ValueError("Simulator joints do not match the Nano model")
    return [engine_names.index(n) for n in wire_names]


def motion_target(model, mode, phase):
    if (
        mode not in ("left", "right", "both")
        or not math.isfinite(phase)
        or not 0 <= phase <= 1
    ):
        raise ValueError("Invalid motion or phase")
    active = model["left"] + model["right"] if mode == "both" else model[mode]
    pulse = math.sin(math.pi * phase) ** 2
    target = []
    for name in model["names"]:
        if name not in active:
            q = 0.0
        else:
            joint = int(name.split("_j")[1][0])
            amplitude = 0.25 if joint == 6 else 0.06 + 0.012 * joint
            direction = -1 if name.startswith("right_") and joint in (1, 5) else 1
            q = direction * amplitude * pulse
        low, high = model["limits"][name]
        if not low <= q <= high:
            raise ValueError(f"Target outside model limits: {name}")
        target.append(q)
    return target


def write_json(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True, allow_nan=False) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect(args, model):
    import numpy as np
    import pyarrow as pa
    from aisle.harness.trace_recorder import TRACE_SCHEMA
    from aisle.sim.nexus_backend import _ensure_nexus, _new_scene

    source = args.output / "aisle-run"
    frames_dir = source / "traces/frames/overhead"
    frames_dir.mkdir(parents=True)
    engine = _ensure_nexus(args.backend)
    scene = _new_scene(
        engine, {"sim": {"dt": 0.01, "gravity": [0, 0, -9.81]}}, 1, [0.5] * 3
    )
    robot = scene.add_urdf_robot(args.model, "adora_nano", True)
    scene.add_box("floor", [2, 2, 0.1], [0, 0, -0.0843], fixed=True)
    camera = scene.add_camera((320, 240), 55, pos=(1.2, -1.2, 1.0), lookat=(0, 0, 0.35))
    scene.build()
    by_offset = sorted((j.dofs_idx_local[0], j.name) for j in robot.joints if j.n_dofs)
    if any(j.n_dofs != 1 for j in robot.joints if j.n_dofs):
        raise ValueError("Expected one DOF per movable Nano joint")
    indices = native_indices(model["names"], [n for _, n in by_offset])
    grippers = [
        model["names"].index(side + "_j6_gripper") for side in ("left", "right")
    ]
    native_grippers = [indices[i] for i in grippers]
    rows = defaultdict(list)
    tick = 0

    def stamp():
        return tick * 10_000_000

    def record(topic, data=None, text=None):
        rows[topic].append(
            dict(
                sim_time_ns=stamp(),
                env_id=0,
                seq=len(rows[topic]),
                data=data,
                text=text,
            )
        )

    def step(count=1):
        nonlocal tick
        for _ in range(count):
            scene.step()
            tick += 1

    def state():
        q = np.asarray(robot.get_qpos(), dtype=np.float32)[indices]
        if q.shape != (17,) or not np.isfinite(q).all():
            raise ValueError("Simulator produced invalid joint state")
        return q

    def command(wire):
        native = np.empty(17, dtype=np.float32)
        native[indices] = wire
        robot.control_dofs_position(native)

    outcomes = []
    for episode, mode in enumerate(("left", "right", "both")):
        robot.set_qpos(np.zeros(17, dtype=np.float32))
        command(np.zeros(17, dtype=np.float32))
        record("nano_reset__reset_done", [1.0])
        peak_error, limit_excess = 0.0, 0.0
        measured, commanded = [], []
        for frame in range(args.frames):
            target = motion_target(model, mode, frame / (args.frames - 1))
            step()
            if frame == 0:
                record(
                    "nano_task__target_request",
                    text=json.dumps(
                        {
                            "instruction": f"Software validation: {mode} arm joint motion; fixed base; simulation only"
                        }
                    ),
                )
            # Gripper and observation precede the arm command in simulation
            # time, so AISLE's strictly-earlier join needs no invented ordering.
            robot.control_dofs_position(
                [target[i] for i in grippers], dofs_idx_local=native_grippers
            )
            record("nano_limits__gripper_cmd_safe", [target[i] for i in grippers])
            step()
            q = state()
            rgb = np.asarray(camera.render()[0], dtype=np.uint8)
            if rgb.shape != (240, 320, 3):
                raise ValueError(f"Unexpected rendered image shape: {rgb.shape}")
            np.savez_compressed(frames_dir / f"{stamp():020d}.npz", rgb=rgb)
            record("nano_camera__rgb_overhead")
            record("nano_state__joint_state", q.tolist())
            step()
            command(target)
            record("nano_limits__joint_cmd_safe", target)
            step(7)
            after = state()
            measured.append(after.tolist())
            commanded.append(target)
            peak_error = max(peak_error, float(np.max(np.abs(after - target))))
            for name, value in zip(model["names"], after):
                low, high = model["limits"][name]
                limit_excess = max(
                    limit_excess, low - float(value), float(value) - high
                )
        active_sides = ("left", "right") if mode == "both" else (mode,)
        excursion = np.ptp(np.asarray(measured), axis=0)
        moving = {
            side: float(max(excursion[model["names"].index(n)] for n in model[side]))
            for side in ("left", "right")
        }
        ok = (
            peak_error < 0.08
            and limit_excess < 0.03
            and all(moving[s] > 0.03 for s in active_sides)
        )
        outcomes.append(
            dict(
                episode=episode,
                env_id=0,
                status="success" if ok else "fail",
                task_family="nano-studio-motion-check",
                mode=mode,
                peak_tracking_error_rad=peak_error,
                max_joint_limit_excess_rad=limit_excess,
                measured_arm_excursion_rad=moving,
                acceptance="finite state, tracking error <0.08 rad, limit excess <0.03 rad, each commanded arm excursion >0.03 rad; software fixture only",
            )
        )
        print(json.dumps(outcomes[-1]), file=sys.stderr, flush=True)
    robot.set_qpos(np.zeros(17, dtype=np.float32))
    command(np.zeros(17, dtype=np.float32))
    record("nano_reset__reset_done", [1.0])
    for name, values in rows.items():
        table = pa.Table.from_pylist(values, schema=TRACE_SCHEMA)
        with (
            pa.OSFile(str(source / "traces" / (name + ".arrow")), "wb") as sink,
            pa.ipc.new_stream(sink, TRACE_SCHEMA) as writer,
        ):
            writer.write_table(table)
    (source / "episodes.jsonl").write_text(
        "".join(json.dumps(v) + "\n" for v in outcomes)
    )
    write_json(
        source / "manifest.json",
        dict(
            embodiment="adora_nano",
            sim_engine="nexus",
            sim_backend=args.backend,
            evidence_kind="simulated_joint_motion_software_fixture",
            hardware_connected=False,
            fixed_base=True,
            head_and_wheels="zero position targets",
            state_joint_names=model["names"],
            units="radians",
            physics_dt_s=0.01,
            capture_fps=10,
            source_model=str(args.model),
            model_sha256=digest(args.model),
            nexus_version=importlib.metadata.version("dimforge-nexus3d"),
            aisle_modules={
                p: digest(args.aisle / p)
                for p in (
                    "src/aisle/sim/nexus_backend.py",
                    "src/aisle/sim/nexus_physics.toml",
                    "src/aisle/harness/trace_recorder.py",
                    "tools/demo_dataset.py",
                )
            },
            command_validation="URDF-limit checked probe targets; not a certified AISLE graph or benchmark",
            model_dynamics="nominal inertias; AISLE default URDF gains/damping; not hardware calibrated",
        ),
    )
    contract = dict(
        embodiment="adora_nano",
        state=dict(names=model["names"], units=["rad"] * 17, frame="joint"),
        forwarded=dict(
            names=model["names"],
            units=["rad"] * 17,
            frame="joint",
            semantics="position_target",
        ),
        gripper=dict(
            names=[model["names"][i] for i in grippers],
            units=["rad"] * 2,
            frame="joint",
            semantics="position_target",
        ),
        training_action="full_forwarded_position_targets_17",
    )
    write_json(source / "contract.json", contract)
    return source, contract, outcomes


def to_lerobot(export, destination, model):
    import cv2
    import numpy as np
    import pyarrow as pa
    import pyarrow.parquet as pq
    from demo_dataset import read_dataset

    data = read_dataset(export)
    camera_key = "observation.images.front"
    meta = destination / "meta"
    meta.mkdir(parents=True)
    episodes, tasks, mapping = [], [], []
    total = 0
    for index, episode in enumerate(data["episodes"]):
        samples = episode["samples"]
        if not samples or any(not s["eligible"] for s in samples):
            raise ValueError(
                "Refusing to hide ineligible AISLE samples in the Studio export"
            )
        width = len(model["names"])
        states = np.asarray([s["state"]["data"] for s in samples], dtype=np.float32)
        actions = np.asarray(
            [s["forwarded"]["data"] for s in samples], dtype=np.float32
        )
        first = samples[0]["forwarded"]["sim_time_ns"]
        times = [(s["forwarded"]["sim_time_ns"] - first) / 1e9 for s in samples]
        if not np.allclose(times, np.arange(len(samples)) / 10, rtol=0, atol=1e-9):
            raise ValueError(
                "This probe requires a regular 10 Hz recording; no silent resampling"
            )
        task = samples[0]["instruction"]["text"]
        table = pa.table(
            {
                "observation.state": pa.FixedSizeListArray.from_arrays(
                    pa.array(states.ravel(), type=pa.float32()), width
                ),
                "action": pa.FixedSizeListArray.from_arrays(
                    pa.array(actions.ravel(), type=pa.float32()), width
                ),
                "timestamp": pa.array(times, type=pa.float64()),
                "frame_index": pa.array(range(len(samples)), type=pa.int64()),
                "episode_index": pa.array([index] * len(samples), type=pa.int64()),
                "index": pa.array(range(total, total + len(samples)), type=pa.int64()),
                "task_index": pa.array([index] * len(samples), type=pa.int64()),
            }
        )
        parquet = destination / f"data/chunk-000/episode_{index:06d}.parquet"
        parquet.parent.mkdir(parents=True, exist_ok=True)
        pq.write_table(table, parquet, compression="snappy")
        video_path = (
            destination / f"videos/chunk-000/{camera_key}/episode_{index:06d}.mp4"
        )
        video_path.parent.mkdir(parents=True, exist_ok=True)
        video = cv2.VideoWriter(
            str(video_path), cv2.VideoWriter_fourcc(*"mp4v"), 10, (320, 240)
        )
        if not video.isOpened():
            raise ValueError("Video encoder unavailable")
        try:
            for frame, sample in enumerate(samples):
                ts = sample["frame"]["sim_time_ns"]
                source = export / f"source/traces/frames/overhead/{ts:020d}.npz"
                with np.load(source, allow_pickle=False) as raw:
                    video.write(cv2.cvtColor(raw["rgb"], cv2.COLOR_RGB2BGR))
                mapping.append(
                    dict(
                        episode=index,
                        frame=frame,
                        observation_sim_time_ns=sample["state"]["sim_time_ns"],
                        camera_sim_time_ns=ts,
                        command_sim_time_ns=sample["forwarded"]["sim_time_ns"],
                        source_frame_sha256=digest(source),
                        source_command=sample["forwarded"]["source"],
                        source_command_row=sample["forwarded"]["row"],
                    )
                )
        finally:
            video.release()
        decoder = cv2.VideoCapture(str(video_path))
        decoded = 0
        while decoder.read()[0]:
            decoded += 1
        decoder.release()
        if decoded != len(samples):
            raise ValueError("Video frame count disagrees with Parquet")
        episodes.append(dict(episode_index=index, tasks=[task], length=len(samples)))
        tasks.append(dict(task_index=index, task=task))
        total += len(samples)
    features = {
        k: dict(dtype="float32", shape=[17], names=model["names"])
        for k in ("observation.state", "action")
    }
    features.update(
        {
            k: dict(
                dtype="float64" if k == "timestamp" else "int64", shape=[1], names=None
            )
            for k in (
                "timestamp",
                "frame_index",
                "episode_index",
                "index",
                "task_index",
            )
        }
    )
    features[camera_key] = dict(
        dtype="video",
        shape=[240, 320, 3],
        names=["height", "width", "channel"],
        info={
            "video.fps": 10,
            "video.codec": "mpeg4",
            "video.pix_fmt": "yuv420p",
            "video.is_depth_map": False,
            "has_audio": False,
        },
    )
    write_json(
        meta / "info.json",
        dict(
            codebase_version="v2.1",
            robot_type="adora_nano",
            fps=10,
            total_episodes=len(episodes),
            total_frames=total,
            total_tasks=len(tasks),
            total_videos=len(episodes),
            total_chunks=1,
            chunks_size=1000,
            splits={"train": f"0:{len(episodes)}"},
            data_path="data/chunk-{episode_chunk:03d}/episode_{episode_index:06d}.parquet",
            video_path="videos/chunk-{episode_chunk:03d}/{video_key}/episode_{episode_index:06d}.mp4",
            features=features,
        ),
    )
    for filename, values in (("episodes.jsonl", episodes), ("tasks.jsonl", tasks)):
        (meta / filename).write_text("".join(json.dumps(v) + "\n" for v in values))
    write_json(
        meta / "aisle_provenance.json",
        dict(
            kind="physics_simulation_software_validation_fixture",
            hardware_connected=False,
            units="radians",
            joint_order=model["names"],
            source_manifest_sha256=digest(export / "manifest.json"),
            outcomes=[e["outcome"] for e in data["episodes"]],
            mapping=mapping,
            limitations=[
                "not a manipulation benchmark",
                "one camera",
                "fixed base",
                "nominal model dynamics",
                "no hardware actuation",
            ],
        ),
    )
    return dict(episodes=len(episodes), frames=total, channels=17, cameras=1, fps=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--aisle",
        type=Path,
        default=Path(__file__).resolve().parents[2].parent / "AISLE",
    )
    parser.add_argument(
        "--model",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "data/adora_nano/adora_nano.urdf",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--backend", choices=["metal", "webgpu", "cpu"], default="metal"
    )
    parser.add_argument("--frames", type=int, default=30)
    args = parser.parse_args()
    if args.frames < 3 or args.output.exists():
        parser.error("Use at least three frames and a new output directory")
    args.aisle, args.model, args.output = (
        p.resolve() for p in (args.aisle, args.model, args.output)
    )
    sys.path.insert(0, str(args.aisle / "tools"))
    from demo_dataset import export_dataset

    model = load_model(args.model)
    source, contract, outcomes = collect(args, model)
    export = args.output / "aisle-export"
    exported = export_dataset(source, export, contract=contract)
    dataset = to_lerobot(export, args.output / "lerobot", model)
    result = dict(
        ok=all(o["status"] == "success" for o in outcomes),
        scope="external Nano/AISLE/Nexus/Studio integration probe",
        output=str(args.output),
        dataset=dataset,
        aisle_export_coverage=exported["coverage"],
        outcomes=outcomes,
    )
    write_json(args.output / "result.json", result)
    print(json.dumps(result, allow_nan=False))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
