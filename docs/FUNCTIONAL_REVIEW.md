# DoRobot Studio setup and workflows

## Setup → Operate → Record → Replay

The sidebar follows the recording journey. **Robot setup** stays a setup page;
**Operate** holds live controls. Library and Evaluate are secondary destinations.
Changing pages preserves a simulation, recording or unsaved take. A capture banner
provides a return to Record when work is pending. Flexible panels and additional
Replay windows remain available.

- **Simulated Nano:** choose Simulation → Start simulation. Studio launches the
  Dora graph and opens Operate after physics and the three cameras are ready.
  First launch can take a few minutes to prepare GPU shaders. Model zero positions
  and limits replace servo calibration; Stop simulation followed by Start creates
  a fresh scene. Hold L1 to move, release to stop, and use R1 to switch control mode.
- **Collect simulated data:** Record → enter task and save folder → Record episode
  → Stop recording → Save episode → Open saved replay. Every take becomes its own
  LeRobot dataset. All three RGB videos, depth, segmentation, camera poses and
  intrinsics, measured joints, commands, scene motion and contact samples are kept.
  Capture is 5 fps in simulation time; the UI separately reports preview throughput.
  This is not a 30 fps real-time guarantee. Save waits for the complete sensor frame
  count. Discard removes the pending take while preserving saved datasets.
- **Real SO-100/101:** choose Real robot → SO-100 / SO-101 arms → enter follower and
  leader ports, camera indexes, task and dataset folder → Use this rig. Existing
  JSON profiles can also be loaded. Start calibration and follow the actual
  LeRobot prompts, pressing Continue only when requested. After completion, open
  Record. Leader control and capture start together. Stop → Save → Open saved replay.
  Missing ports and incomplete calibration block capture with recovery instructions.
- **Real Nano:** choose Real robot → Adora Nano · base only. Select its USB serial
  port and connect, then Open controls. Bus test uses a mock serial bus and does
  not move a simulated robot. This driver supports the mobile base only; arm/head
  calibration and Nano camera recording remain unavailable. Leaving physical
  controls stops the driver.

Simulation requires `uv`, Dora CLI 1.0.1 and an AISLE checkout beside Studio;
`DOROBOT_AISLE` and `DOROBOT_UV` can select other installations. Real arm capture
uses `DOROBOT_PYTHON` and the dependencies below. Profiles and capture preferences
are stored locally; restoring them never connects or drives hardware.

Workflow checks are in `tools/ux/workflow_journeys.py`; the fixed scoring rubric is
`tools/ux/workflow-rubric.json`. Physical robot calibration requires a separate
hardware acceptance test and is not certified by a desktop UX score.

The model-authored [desktop workflow review](../tools/ux/workflow-review.json)
scores **95/100**, with the deductions, repair rounds, actual-input checks and
sanitized evidence recorded per criterion. It covers the supported desktop flows
on the prepared workstation, not complete physical Nano support or a human
usability study. The saved simulation take was audited for all three videos,
17 measured joints, raw checksums, camera poses and camera metadata.

The subsequent [visual review](../tools/ux/visual-polish-review.json) scores
**92/100** against a separate rubric that prioritizes hierarchy, text density,
workspace and typography. It replaces the earlier review's visual assessment
after user feedback: guidance and diagnostics are now disclosures, simulation
has no explanatory side panel, and recording shows only the current actions.
The original flexible panels and additional native replay window remain intact.

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

Copy `tools/capture/rig.example.json` to a profile and set the two serial ports, calibration IDs, task, output dataset, and OpenCV camera indexes or paths. Relative output paths resolve against the profile directory. LeRobot motor setup is required before calibration. Use **Robot setup → Real robot → Load rig profile** to choose the profile, **Start calibration** to calibrate, then **Record → Stop → Save**. Alternatively start with `--rig /path/to/rig.json`. The supplied IDs and ports are examples and do not identify attached hardware.

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
