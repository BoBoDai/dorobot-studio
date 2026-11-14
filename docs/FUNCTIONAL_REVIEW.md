# DoRobot Studio setup and workflows

## Completed workflows

| Area | Changes |
| --- | --- |
| Startup | Production file backend is the default. `--demo` and `ui_mock` explicitly select fixtures. A dataset directory and a parent library directory both open correctly. |
| SO-100 / SO-101 | Rig profiles use official LeRobot leader/follower drivers. Hardware opens only on calibration or Record. The calibration console runs in a PTY with Continue and cancellation. Recording requires saved, matching calibration. |
| Capture | Start, Stop, Save, Discard and Re-record reach a managed worker. Motors and cameras disconnect on completion and failure. State, the command actually sent after clipping, measured timestamps, MP4s, image statistics and live preview PNGs are recorded. Camera previews, measured traces and the URDF view update in Record. |
| Save and recovery | Compatible v2.1 datasets append without truncation. Every declared camera must produce video. An exclusive writer lock prevents simultaneous writers; a commit journal restores metadata and removes partial episode files when resuming after interruption. Completed unsaved takes have a profile sidecar for restart recovery. |
| Dataset format | Standard episode/task indexes, global indexes, scalar/vector features, JSON escaping, per-episode lengths and statistics are written. v2 and v3 readers use actual metadata, including shared Parquet indexes and individual camera video offsets. |
| Review | Accurate seeks decode past keyframes, drain delayed final frames, remove RGB row padding and clamp the final frame. Media reloads when switching datasets. Playback follows elapsed time. |
| Curation | Good/bad tags survive restart. Exclude preserves original episode bytes and offers Undo with Cmd/Ctrl-Z. Excluded episode tags survive other edits. Failed tag saves roll back the displayed state. |
| Hub | Native forms launch background download/upload through `huggingface_hub`; failures are visible. Downloads require an empty destination; newly created repositories are private. |

## Reproduce

Run commands from this repository. Install Rust, platform build tools, **FFmpeg 8.1 development libraries** and `pkg-config`. On macOS, `brew install ffmpeg pkg-config` provides these. With another package manager, set `PKG_CONFIG_PATH` to its `lib/pkgconfig`; the build embeds the discovered macOS library directory as an rpath.

```sh
python3.12 -m venv .venv-capture
.venv-capture/bin/pip install -r tools/capture/requirements.txt
export DOROBOT_PYTHON="$PWD/.venv-capture/bin/python"
cargo run -p dorobot-flex -- --dataset /path/to/dataset-or-library
```

Copy `tools/capture/rig.example.json` to a profile and set the two serial ports, calibration IDs, task, output dataset, and OpenCV camera indexes or paths. Relative output paths resolve against the profile directory. LeRobot motor setup is required before calibration. Use **New Recording Session** to choose the profile, **Hardware → Continue** to calibrate, then **Record → Stop → Save**. Alternatively start with `--rig /path/to/rig.json`. The supplied IDs and ports are examples and do not identify attached hardware.

Saved captures use LeRobot v2.1. For current LeRobot training, convert a copy with its official converter (the converter moves/rewrites directories):

```sh
.venv-capture/bin/python -m lerobot.scripts.convert_dataset_v21_to_v30 \
  --repo-id local/my-dataset --root /path/to/copy --push-to-hub false
cargo run -p dorobot-flex --bin check_dataset -- /path/to/copy
```

Hub authentication uses the normal Hugging Face login store. Push uploads the dataset including curation sidecars; exclusion does not physically filter training rows. Apply that sidecar in downstream dataset selection when training only on accepted episodes.

## Validation

```sh
cargo test --locked -p dorobot-flex --no-default-features
python3 -m unittest discover -s tools/capture -p 'test_*.py' -v
```

The no-default-features build supports URDF preview and dataset operations without
FFmpeg. To test video decoding, install FFmpeg development libraries and run
`DOROBOT_TEST_FFMPEG=/path/to/ffmpeg cargo test --workspace`.
The Python encoder integration test requires the capture environment.

## Remaining limits

No physical arm or cameras were supplied, so calibration, serial I/O timing, torque shutdown and the URDF joint-direction convention still require acceptance on the actual rig. The worker enforces a configurable relative-target limit; physical tests must verify that configuration for the setup.

Capture currently uses local OpenCV cameras at 640×480 and LeRobot normalized joint units; the console shows two camera panes. Existing v3 datasets can be reviewed but capture appends only to compatible v2.1 datasets. Corrupt or unsupported datasets report errors instead of fabricated rows. Abrupt power loss during an active recording does not make its incomplete video a completed take; only completed takes have restart recovery. Atomic file publication and the journal cover application interruption, not a guarantee against storage-device failure.

Hub transfers were not executed against an authenticated account. Eval has no connected policy runner and does not drive hardware. This review completes the capture/review software paths; it does not certify hardware operation or policy evaluation.

Driver/format references: [SO-100 setup](https://github.com/huggingface/lerobot/blob/main/docs/source/so100.mdx), [LeRobot dataset v3](https://huggingface.co/docs/lerobot/lerobot-dataset-v3).
