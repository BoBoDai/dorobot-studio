# DoRobot Studio setup and workflows

For step-by-step operating instructions, screenshots and troubleshooting, use
the [user guide](USER_GUIDE.md). This document records implementation and
validation details.

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
  native calibration prompts, pressing Continue only when requested. After completion, open
  Record. Leader control and capture start together. Stop → Save → Open saved replay.
  Missing ports and incomplete calibration block capture with recovery instructions.
- **Real Nano:** choose Real robot → Adora Nano · base only. Select its USB serial
  port and connect, then Open controls. Bus test uses a mock serial bus and does
  not move a simulated robot. This driver supports the mobile base only; arm/head
  calibration and Nano camera recording remain unavailable. Leaving physical
  controls stops the driver.

The standalone RC includes the native Rust runtime, Dora CLI 1.0.1, Nexus and Nano
assets. It does not require Python, uv or a separate AISLE installation. Source
builds follow the [native runtime guide](../runtime/README.md); use
`DOROBOT_NATIVE_RUNTIME` to select another native executable. Profiles and capture
preferences are stored locally; restoring them never connects or drives hardware.

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
| SO-100 / SO-101 | The Rust worker implements the LeRobot v0.4.3 Feetech register/calibration conventions. Hardware opens only on calibration or Record. The calibration console supports Continue and cancellation. Recording requires saved, matching calibration. |
| Capture | Start, Stop, Save, Discard and Re-record reach a managed worker. Motors and cameras disconnect on completion and failure. State, the command actually sent after clipping, measured timestamps, MP4s, image statistics and live preview PNGs are recorded. Camera previews, measured traces and the URDF view update in Record. |
| Save and recovery | Compatible v2.1 datasets append without truncation. Every declared camera must produce video. An exclusive writer lock prevents simultaneous writers; a commit journal restores metadata and removes partial episode files when resuming after interruption. Completed unsaved takes have a profile sidecar for restart recovery. |
| Dataset format | Standard episode/task indexes, global indexes, scalar/vector features, JSON escaping, per-episode lengths and statistics are written. v2 and v3 readers use actual metadata, including shared Parquet indexes and individual camera video offsets. |
| Review | Accurate seeks decode past keyframes, drain delayed final frames, remove RGB row padding and clamp the final frame. Media reloads when switching datasets. Playback follows elapsed time. |
| Curation | Good/bad tags survive restart. Exclude preserves original episode bytes and offers Undo with Cmd/Ctrl-Z. Excluded episode tags survive other edits. Failed tag saves roll back the displayed state. |
| Hub | Native forms launch Rust HTTP download/upload using Hub commit/LFS APIs; failures are visible. Downloads require an empty destination and publish only after completion; newly created repositories are private. |

## Reproduce

The packaged app needs no development dependencies. For source builds, install
Rust, platform build tools, FFmpeg 8 development libraries and `pkg-config`.
The [release guide](../tools/release/README.md) pins FFmpeg 8.0.3 and the shader
toolchain. Set `PKG_CONFIG_PATH` to the selected library's `lib/pkgconfig`.

```sh
cargo build --manifest-path runtime/Cargo.toml --release --locked
cargo run -p dorobot-flex -- --dataset /path/to/dataset-or-library
```

Copy `tools/capture/rig.example.json` to a profile and set the two serial ports, calibration IDs, task, output dataset, and AVFoundation camera indexes or video paths. Relative output paths resolve against the profile directory. Motors need the expected IDs and baud rate before calibration. Use **Robot setup → Real robot → Load rig profile** to choose the profile, **Start calibration** to calibrate, then **Record → Stop → Save**. Alternatively start with `--rig /path/to/rig.json`. The supplied IDs and ports are examples and do not identify attached hardware.

Saved captures use LeRobot v2.1. Training tools that require v3 need an external
conversion of a copy; Studio can review both formats. Check a dataset with:

```sh
cargo run -p dorobot-flex --bin check_dataset -- /path/to/copy
```

Hub authentication uses the normal Hugging Face login store. Push uploads the dataset including curation sidecars; exclusion does not physically filter training rows. Apply that sidecar in downstream dataset selection when training only on accepted episodes.

## Validation

```sh
cargo test --locked -p dorobot-flex --no-default-features
cargo test --manifest-path runtime/Cargo.toml --locked
cargo test --manifest-path runtime/Cargo.toml --test dora_base -- --ignored --test-threads=1
cargo test --manifest-path runtime/Cargo.toml --test dora_sim -- --ignored --nocapture
```

The no-default-features build supports URDF preview and dataset operations without
FFmpeg. To test video decoding, install FFmpeg development libraries and run
`DOROBOT_TEST_FFMPEG=/path/to/ffmpeg cargo test --workspace`.
The native runtime tests cover encoding/decoding and raw recording. The opt-in
process tests require native Dora; the simulation test also requires a Metal GPU.
Packaged UI acceptance is documented in the release guide. Historical Python
validation tools remain available as references and are not application workers.

## Remaining limits

No physical arm or cameras were supplied, so calibration, serial I/O timing, torque shutdown and the URDF joint-direction convention still require acceptance on the actual rig. The worker enforces a configurable relative-target limit; physical tests must verify that configuration for the setup.

Physical arm capture currently uses native FFmpeg/AVFoundation cameras at 640×480 and LeRobot normalized joint units; the console shows two camera panes. Simulated Nano captures all three cameras. Existing v3 datasets can be reviewed but capture appends only to compatible v2.1 datasets. Corrupt or unsupported datasets report errors instead of fabricated rows. Abrupt power loss during an active recording does not make its incomplete video a completed take; only completed takes have restart recovery. Atomic file publication and the journal cover application interruption, not a guarantee against storage-device failure.

Hub transfers were not executed against an authenticated account. Eval has no connected policy runner and does not drive hardware. This review completes the capture/review software paths; it does not certify hardware operation or policy evaluation.

Driver/format references: [SO-100 setup](https://github.com/huggingface/lerobot/blob/main/docs/source/so100.mdx), [LeRobot dataset v3](https://huggingface.co/docs/lerobot/lerobot-dataset-v3).
