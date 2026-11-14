"""Hardware-free checks for profile validation and the capture protocol."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import worker

class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "rig.json"
        self.profile = json.loads(Path(__file__).with_name("rig.example.json").read_text())
    def tearDown(self):
        self.temp.cleanup()
    def write(self):
        self.path.write_text(json.dumps(self.profile))
    def test_valid_profile_and_distinct_ports(self):
        self.write()
        self.assertEqual(worker.load_profile(self.path)["robot_type"], "so100")
        self.profile["leader_port"] = self.profile["follower_port"]
        self.write()
        with self.assertRaises(ValueError): worker.load_profile(self.path)
    def test_invalid_camera_and_nonfinite_fps(self):
        self.profile["cameras"] = {"../../outside": 0}
        self.write()
        with self.assertRaises(ValueError): worker.load_profile(self.path)
        self.profile["cameras"] = {}
        self.profile["fps"] = float("nan")
        self.write()
        with self.assertRaises(ValueError): worker.load_profile(self.path)
    def test_failure_is_json_and_nonzero_without_opening_hardware(self):
        self.profile["robot_type"] = "invalid"
        self.write()
        p = subprocess.run([sys.executable, str(Path(worker.__file__)), "check", str(self.path)], capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual(json.loads(p.stdout)["event"], "error")
    def test_partial_connection_is_disconnected(self):
        device = mock.Mock()
        device.is_connected = False
        device.bus.is_connected = True
        worker.disconnect(device)
        device.bus.disconnect.assert_called_once()

class CaptureTests(unittest.TestCase):
    def test_real_video_frames_actions_stats_and_disconnect(self):
        try:
            import cv2
            import numpy as np
        except ImportError:
            self.skipTest("install tools/capture/requirements.txt for encoder integration test")
        import io
        from collections import OrderedDict
        names = ["shoulder_pan.pos", "shoulder_lift.pos", "elbow_flex.pos", "wrist_flex.pos", "wrist_roll.pos", "gripper.pos"]
        robot, leader = mock.Mock(), mock.Mock()
        robot.action_features = OrderedDict.fromkeys(names)
        robot.get_observation.return_value = {**dict.fromkeys(names, 0.25), "front": np.full((480, 640, 3), [32, 96, 160], dtype=np.uint8)}
        leader.get_action.return_value = dict.fromkeys(names, 90.0)
        robot.send_action.return_value = dict.fromkeys(names, 5.25)
        profile = {"fps":30, "cameras":{"front":0}, "max_episode_seconds":0.07}
        with tempfile.TemporaryDirectory() as td:
            output = io.StringIO()
            with mock.patch.object(worker, "devices", return_value=(robot, leader)), mock.patch.object(worker, "OUT", output), mock.patch.object(worker.select, "select", return_value=([], [], [])), mock.patch.object(worker.signal, "signal"):
                worker.record(profile, td)
            events = [json.loads(x) for x in output.getvalue().splitlines()]
            take = events[-1]
            self.assertEqual(take["event"], "take")
            frames = [json.loads(x) for x in Path(take["frames"]).read_text().splitlines()]
            self.assertGreaterEqual(len(frames), 3)
            self.assertTrue(all(f["action"] == [5.25]*6 for f in frames))
            self.assertTrue(all(a["timestamp"] < b["timestamp"] for a,b in zip(frames, frames[1:])))
            key = "observation.images.front"
            video = cv2.VideoCapture(take["videos"][key])
            self.assertTrue(video.isOpened())
            self.assertEqual(int(video.get(cv2.CAP_PROP_FRAME_COUNT)), len(frames))
            ok, pixels = video.read(); video.release()
            self.assertTrue(ok)
            self.assertEqual(pixels.shape, (480,640,3))
            self.assertEqual(take["camera_stats"][key]["count"], [len(frames)])
            self.assertAlmostEqual(take["camera_stats"][key]["mean"][0][0][0], 32/255, places=6)
            self.assertTrue(Path(td, "front.png").is_file())
        robot.disconnect.assert_called_once()
        leader.disconnect.assert_called_once()

if __name__ == "__main__": unittest.main()
