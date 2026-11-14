"""Check converted rows and video against independent original AISLE traces."""

import json
from pathlib import Path

import cv2
import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq


def verify(root):
    source = root / "aisle-run"
    dataset = root / "lerobot"
    provenance = json.loads((dataset / "meta/aisle_provenance.json").read_text())
    manifest = json.loads((source / "manifest.json").read_text())
    info = json.loads((dataset / "meta/info.json").read_text())
    names = manifest["state_joint_names"]
    assert info["features"]["observation.state"]["names"] == names
    assert info["features"]["action"]["names"] == names

    def stream(topic):
        with pa.memory_map(str(source / "traces" / topic), "r") as file:
            return pa.ipc.open_stream(file).read_all().to_pylist()

    commands = stream("nano_limits__joint_cmd_safe.arrow")
    observations = {
        r["sim_time_ns"]: r["data"] for r in stream("nano_state__joint_state.arrow")
    }
    mappings = provenance["mapping"]
    count, max_pixel_mae = 0, 0.0
    for episode in range(3):
        table = pq.read_table(
            dataset / f"data/chunk-000/episode_{episode:06d}.parquet"
        ).to_pylist()
        cap = cv2.VideoCapture(
            str(
                dataset
                / f"videos/chunk-000/observation.images.front/episode_{episode:06d}.mp4"
            )
        )
        for frame_index, row in enumerate(table):
            mapping = mappings[count]
            command = commands[count]
            assert (mapping["episode"], mapping["frame"]) == (episode, frame_index)
            assert (
                mapping["camera_sim_time_ns"]
                == mapping["observation_sim_time_ns"]
                < command["sim_time_ns"]
            )
            assert mapping["command_sim_time_ns"] == command["sim_time_ns"]
            np.testing.assert_allclose(
                row["action"], command["data"], rtol=0, atol=1e-7
            )
            np.testing.assert_array_equal(
                row["observation.state"],
                observations[mapping["observation_sim_time_ns"]],
            )
            assert row["episode_index"] == episode and row["frame_index"] == frame_index
            assert row["index"] == count
            assert abs(row["timestamp"] - frame_index / 10) < 1e-9
            ok, image = cap.read()
            assert ok
            with np.load(
                source
                / f"traces/frames/overhead/{mapping['camera_sim_time_ns']:020d}.npz"
            ) as raw:
                error = np.abs(
                    cv2.cvtColor(image, cv2.COLOR_BGR2RGB).astype(float) - raw["rgb"]
                ).mean()
            assert error < 8.0, error
            max_pixel_mae = max(max_pixel_mae, float(error))
            count += 1
        assert not cap.read()[0]
        cap.release()
    assert count == len(commands) == info["total_frames"] == 90
    return dict(
        ok=True,
        checked_frames=count,
        joint_values_and_order_preserved=True,
        original_timestamps_preserved_in_provenance=True,
        no_future_frame_joins=True,
        video_frame_count_matches=True,
        maximum_video_rgb_mae=max_pixel_mae,
    )


if __name__ == "__main__":
    import sys

    root = Path(sys.argv[1])
    result = verify(root)
    (root / "artifact-verification.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )
    print(json.dumps(result))
