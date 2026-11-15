#!/usr/bin/env python3
"""SO-100/101 capture worker. JSON lines on stdout; diagnostics on stderr.

Hardware is opened only for an explicit capture/calibration request. Recordings
use LeRobot's normalized joint units and store the action actually sent after
LeRobot applies the configured relative-target limit.
"""
import argparse
import contextlib
import importlib
import json
import math
import os
from pathlib import Path
import select
import signal
import sys
import time

OUT = sys.stdout

def emit(event, **values):
    OUT.write(json.dumps(dict(event=event, **values), allow_nan=False) + "\n")
    OUT.flush()

def load_profile(path):
    p = json.loads(Path(path).read_text())
    if p.get("robot_type") not in ("so100", "so101"):
        raise ValueError("robot_type must be so100 or so101")
    for key in ("follower_port", "leader_port", "follower_id", "leader_id", "task", "dataset"):
        if not isinstance(p.get(key), str) or not p[key].strip():
            raise ValueError(f"profile requires {key}")
    if p["follower_port"] == p["leader_port"]:
        raise ValueError("leader and follower must use distinct ports")
    fps = p.get("fps", 30)
    if not isinstance(fps, (int, float)) or not math.isfinite(fps) or not 1 <= fps <= 60:
        raise ValueError("fps must be between 1 and 60")
    limit = p.get("max_relative_target", 5)
    if not isinstance(limit, (int, float)) or not math.isfinite(limit) or limit <= 0:
        raise ValueError("max_relative_target must be positive")
    duration = p.get("max_episode_seconds", 300)
    if not isinstance(duration, (int, float)) or not math.isfinite(duration) or not 0 < duration <= 3600:
        raise ValueError("max_episode_seconds must be between 0 and 3600")
    for key, value in p.get("cameras", {}).items():
        if not key or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in key):
            raise ValueError("camera names may contain letters, digits, underscores and hyphens")
        if not isinstance(value, (int, str)):
            raise ValueError("camera values must be OpenCV indexes or paths")
    return p

def classes(robot):
    stem = "SO100" if robot == "so100" else "SO101"
    # LeRobot unified these modules after 0.4.3; support both layouts.
    try:
        follower = importlib.import_module("lerobot.robots.so_follower")
        leader = importlib.import_module("lerobot.teleoperators.so_leader")
    except ModuleNotFoundError as e:
        if e.name not in ("lerobot.robots.so_follower", "lerobot.teleoperators.so_leader"):
            raise
        follower = importlib.import_module(f"lerobot.robots.{robot}_follower")
        leader = importlib.import_module(f"lerobot.teleoperators.{robot}_leader")
    return (getattr(follower, stem + "Follower"), getattr(follower, stem + "FollowerConfig"),
            getattr(leader, stem + "Leader"), getattr(leader, stem + "LeaderConfig"))

def devices(p, cameras=True):
    Follower, FConfig, Leader, LConfig = classes(p["robot_type"])
    configs = {}
    if cameras and p.get("cameras"):
        from lerobot.cameras.opencv.configuration_opencv import OpenCVCameraConfig
        configs = {key: OpenCVCameraConfig(index_or_path=value, width=640, height=480, fps=p.get("fps", 30))
                   for key, value in p["cameras"].items()}
    follower = Follower(FConfig(port=p["follower_port"], id=p["follower_id"], cameras=configs,
                               use_degrees=False, max_relative_target=p.get("max_relative_target", 5),
                               disable_torque_on_disconnect=True))
    leader = Leader(LConfig(port=p["leader_port"], id=p["leader_id"], use_degrees=False))
    return follower, leader

def disconnect(device):
    # Partial connect failures may leave an open bus while is_connected is false.
    try:
        if device.is_connected:
            device.disconnect()
        elif getattr(device, "bus", None) is not None and device.bus.is_connected:
            device.bus.disconnect()
    except Exception as e:
        print(f"disconnect failed: {e}", file=sys.stderr)

def record(p, staging):
    import cv2
    import numpy as np
    camera_stats = {}
    root = Path(staging)
    root.mkdir(parents=True, exist_ok=True)
    robot, leader = devices(p)
    writers = {}
    stopping = False
    def stop(*_):
        nonlocal stopping
        stopping = True
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    fps = float(p.get("fps", 30))
    count = 0
    try:
        # Never enter a hidden interactive calibration from the Record button.
        if not robot.calibration or not leader.calibration:
            raise RuntimeError("Calibration is missing. Open Robot setup and calibrate the leader and follower first.")
        robot.connect(calibrate=False)
        leader.connect(calibrate=False)
        if not robot.is_calibrated or not leader.is_calibrated:
            raise RuntimeError("Motor calibration does not match this rig profile. Recalibrate before recording.")
        names = list(robot.action_features)
        videos = {}
        for key in p.get("cameras", {}):
            path = root / f"{key}.mp4"
            writer = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"mp4v"), fps, (640, 480))
            if not writer.isOpened():
                raise RuntimeError(f"Could not open video encoder for {key}")
            writers[key] = writer
            videos[f"observation.images.{key}"] = str(path)
        emit("ready", joints=names)
        started = time.perf_counter()
        with (root / "frames.jsonl").open("x") as frames:
            while not stopping:
                if select.select([sys.stdin], [], [], 0)[0]:
                    # EOF also stops: closing Studio must not leave the arm driven.
                    command = sys.stdin.readline()
                    if not command or command.strip() == "stop":
                        break
                before = time.perf_counter()
                obs = robot.get_observation()
                action = robot.send_action(leader.get_action())
                state = [float(obs[k]) for k in names]
                sent = [float(action[k]) for k in names]
                timestamp = before - started
                frame = dict(state=state, action=sent, timestamp=timestamp)
                if not all(math.isfinite(v) for v in state + sent):
                    raise ValueError("Non-finite motor reading; capture stopped")
                for key, writer in writers.items():
                    pixels = obs[key]
                    if tuple(pixels.shape) != (480, 640, 3):
                        raise ValueError(f"{key}: expected 640×480 RGB, got {pixels.shape}")
                    writer.write(cv2.cvtColor(pixels, cv2.COLOR_RGB2BGR))
                    sample = pixels[::8, ::8].reshape(-1, 3).astype(np.float64) / 255.0
                    summary = camera_stats.setdefault(key, {"min":np.ones(3), "max":np.zeros(3), "sum":np.zeros(3), "sumsq":np.zeros(3), "pixels":0})
                    summary["min"] = np.minimum(summary["min"], sample.min(axis=0))
                    summary["max"] = np.maximum(summary["max"], sample.max(axis=0))
                    summary["sum"] += sample.sum(axis=0)
                    summary["sumsq"] += (sample * sample).sum(axis=0)
                    summary["pixels"] += len(sample)
                    if count % 6 == 0:
                        tmp = root / f"{key}.tmp.png"
                        cv2.imwrite(str(tmp), cv2.cvtColor(pixels, cv2.COLOR_RGB2BGR))
                        tmp.replace(root / f"{key}.png")
                frames.write(json.dumps(frame, allow_nan=False) + "\n")
                if count % 6 == 0:
                    frames.flush()
                    emit("frame", **frame, count=count + 1,
                         previews={k: str(root / f"{k}.png") for k in writers})
                count += 1
                # Schedule against the clock; slow hardware produces measured drift.
                time.sleep(max(0, started + count / fps - time.perf_counter()))
                if before - started >= p.get("max_episode_seconds", 300):
                    break
    finally:
        for writer in writers.values():
            writer.release()
        disconnect(robot)
        disconnect(leader)
    if not count:
        raise RuntimeError("No frames captured")
    for key, path in videos.items():
        encoded = cv2.VideoCapture(path)
        try:
            if not encoded.isOpened() or int(encoded.get(cv2.CAP_PROP_FRAME_COUNT)) != count:
                raise RuntimeError(f"{key}: encoded video frame count does not match {count} motor samples")
        finally:
            encoded.release()
    image_stats = {}
    for key, s in camera_stats.items():
        mean = s["sum"] / s["pixels"]
        std = np.sqrt(np.maximum(0, s["sumsq"] / s["pixels"] - mean * mean))
        image_stats[f"observation.images.{key}"] = {"min":s["min"].reshape(3,1,1).tolist(), "max":s["max"].reshape(3,1,1).tolist(), "mean":mean.reshape(3,1,1).tolist(), "std":std.reshape(3,1,1).tolist(), "count":[count]}
    emit("take", frames=str(root / "frames.jsonl"), videos=videos, count=count, camera_stats=image_stats)

def calibrate_child(p):
    robot, leader = devices(p, cameras=False)
    try:
        for device in (leader, robot):
            device.connect(calibrate=False)
            device.calibrate()
            device.disconnect()
    finally:
        disconnect(robot)
        disconnect(leader)

def readable_calibration_prompt(text):
    """Convert the PTY's rolling terminal output into readable UI instructions."""
    import re
    text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
    text = re.sub(r"\x1b\][^\x07]*(?:\x07|\x1b\\)", "", text)
    text = re.sub(r"\x1b[^\n]*$", "", text)
    return text.replace("\r", "\n").replace("\x08", "").strip()[-1200:]


def calibrate(profile):
    # LeRobot's range sweep uses a terminal to detect Enter; provide a PTY,
    # and relay it to Hardware's instructions and Continue button.
    import pty
    pid, fd = pty.fork()
    if pid == 0:
        os.execl(sys.executable, sys.executable, __file__, "calibrate-child", profile)
    cancelled = False
    console_text = ""
    def cancel(*_):
        nonlocal cancelled
        cancelled = True
    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    try:
        while not cancelled:
            ready, _, _ = select.select([fd, sys.stdin], [], [], 0.1)
            if sys.stdin in ready:
                line = sys.stdin.readline()
                if not line:
                    os.kill(pid, signal.SIGINT)
                    break
                os.write(fd, line.encode())
            if fd in ready:
                try:
                    data = os.read(fd, 8192)
                except OSError:
                    break
                if not data:
                    break
                console_text = (console_text + data.decode(errors="replace"))[-8192:]
                emit("prompt", message=readable_calibration_prompt(console_text))
    finally:
        os.close(fd)
        # Bound shutdown even if a serial read stops answering. The child is
        # not reaped until here, so its PID cannot have been reused.
        found, status = os.waitpid(pid, os.WNOHANG)
        if not found:
            os.kill(pid, signal.SIGINT)
            deadline = time.monotonic() + 3
            while not found and time.monotonic() < deadline:
                time.sleep(0.05)
                found, status = os.waitpid(pid, os.WNOHANG)
            if not found:
                os.kill(pid, signal.SIGKILL)
                _, status = os.waitpid(pid, 0)
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError("Calibration failed or was cancelled. " + readable_calibration_prompt(console_text)[-800:])
    emit("calibrated")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=("record", "calibrate", "calibrate-child", "check"))
    ap.add_argument("profile")
    ap.add_argument("staging", nargs="?")
    args = ap.parse_args()
    try:
        p = load_profile(args.profile)
        if args.mode == "calibrate-child":
            calibrate_child(p)
        else:
            with contextlib.redirect_stdout(sys.stderr):
                if args.mode == "record":
                    record(p, args.staging)
                elif args.mode == "calibrate":
                    calibrate(args.profile)
                else:
                    classes(p["robot_type"])
                    emit("checked")
    except Exception as e:
        emit("error", message=f"{type(e).__name__}: {e}")
        return 1
    return 0

if __name__ == "__main__":
    sys.exit(main())
