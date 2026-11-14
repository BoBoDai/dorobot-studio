# Validate Studio with Adora Nano, Nexus, and AISLE

This workflow simulates Adora Nano's two arms in **Nexus**, records the motion
using AISLE's trace schema and offline exporter, converts it to a LeRobot-shaped
dataset, and exercises Studio's production dataset backend and video decoder.

The helpers in this directory are an integration probe. They use AISLE's existing
Python components; they do **not** add `adora_nano` to `harness rollout` or modify
AISLE's frozen scenes. A physical robot is not required.

```text
Nano URDF → Nexus physics → AISLE Arrow traces + raw images
         → AISLE offline export → LeRobot Parquet + MP4 → Studio checks
```

## What the run does

The bundled [Nano model](../../data/adora_nano/README.md) supplies the actual link
geometry, joint axes, limits, and grippers. Each arm has five arm joints plus a
gripper. The complete state has **17 channels**: 12 arm/gripper channels, two head
joints, and three wheels. Joint names map the URDF order to Nexus's internal order;
the grippers are not contiguous with their respective arm chains.

The base is fixed, and the head and wheel joints receive zero position targets.
Three episodes exercise the left arm, right arm, and both arms. At the default
settings, each episode contains 30 samples: **90 frames total, 10 fps, one 320×240
front camera**. Angles and position targets are in radians.

These are joint-motion checks. Their `success` labels describe the checks below,
not successful grasping or a completed manipulation task.

## Prerequisites and versions

- macOS on Apple Silicon for the tested **Metal** path.
- `uv` and Python 3.13.5, a current stable Rust toolchain, and Git.
- FFmpeg development libraries and `pkg-config` for Studio's real video decoder.
  On macOS: `brew install ffmpeg pkg-config`.
- A Studio checkout containing this directory and `data/adora_nano/`.
- An [AISLE checkout](https://github.com/moxin-org/AISLE), preferably alongside
  Studio. If needed, from the Studio root: `git clone https://github.com/moxin-org/AISLE.git ../AISLE`.

The original reference run used AISLE commit
`845e31f5d1871905e69c9d838f90a32c62bd6476`, Studio commit
`f3365cf26066fb3fecbfe7495cfb3e8a3478003a`, `dimforge-nexus3d==0.2.1`, and
`opencv-python-headless==5.0.0.93`. The commands below pin those two overlay
packages and use AISLE's lockfile for its base dependencies. For exact source
reproduction, use that AISLE commit in a dedicated clone. This probe imports
internal AISLE helpers, so later AISLE changes may require an update.

The packaged workflow was rechecked on 2026-11-14 against Studio
`feabe3758f07bd9ddfd60f93e03666f6f05ff5e3`: all six model tests, three Nexus
episodes, 90 source-to-dataset rows, and Studio backend checks passed.

The Python commands do not require AISLE's full `sim` extra or a separate Nexus
source build. `uv` supplies the Nexus wheel in an overlay environment. This is
development evidence, not an attested AISLE benchmark environment.

## 1. Set paths and check the Nano contract

Start in the **Studio repository root**, not in this directory:

```sh
studio_root="$PWD"
aisle_root="$(cd ../AISLE && pwd)"  # change this if AISLE is elsewhere
run_root="$studio_root/tools/aisle-validation/runs/nano-bimanual-001"

uv run --project "$aisle_root" --frozen --python 3.13.5 \
  python -B -m unittest discover -s tools/aisle-validation \
  -p 'test_nano_validation.py' -v
```

The six tests check the real model's channel groups, name-based mapping, arm
isolation, head/wheel hold commands, trajectory endpoints, joint limits, and
invalid-input rejection. They do not launch simulation.

Use a **new `run_root` for each run**. The collector refuses an existing output
directory. Generated runs are ignored by Git.

## 2. Collect motion in Nexus and export through AISLE

```sh
uv run --project "$aisle_root" --frozen --python 3.13.5 \
  --with 'dimforge-nexus3d==0.2.1' \
  --with 'opencv-python-headless==5.0.0.93' \
  python -B tools/aisle-validation/nano_validation.py \
  --aisle "$aisle_root" \
  --model "$studio_root/data/adora_nano/adora_nano.urdf" \
  --backend metal --output "$run_root"
```

This calls AISLE's Nexus scene wrapper, runs the actual physics steps, writes
Arrow traces and raw RGB captures, invokes `tools/demo_dataset.py`, and creates
the Studio dataset. It never substitutes generated kinematic clips for a failed
physics run. Keep the default `--frames 30`: the supplied acceptance checkers
expect three 30-frame episodes.

The collector writes `result.json` and exits successfully only when all motion
checks pass. The per-episode checks require finite states, maximum tracking error
below 0.08 rad, joint-limit excess below 0.03 rad, and measured excursion above
0.03 rad for each commanded arm. Failed episode outcomes remain in the evidence.
The exporter refuses ineligible samples instead of silently dropping them.

`--backend webgpu` and `--backend cpu` are exposed for Nexus development, but
this guide's validation results cover Metal only.

## 3. Check the dataset against the original traces

```sh
uv run --project "$aisle_root" --frozen --python 3.13.5 \
  --with 'opencv-python-headless==5.0.0.93' \
  python -B tools/aisle-validation/verify_artifacts.py "$run_root"
```

This independently reads the original Arrow streams and the converted Parquet
and MP4 files. It checks all 90 rows for joint names, state/action values, episode
and frame indexes, timestamps, and video frame counts. It also compares decoded
video pixels with the original RGB captures, allowing for lossy video encoding.
The result is `artifact-verification.json`.

AISLE joins observations strictly before a command in simulation time. Parquet
timestamps are episode-relative command times; the paired camera and state
timestamps remain separately recorded in `lerobot/meta/aisle_provenance.json`.
Neither the converter nor the checker invents a future observation or silently
resamples an irregular recording.

## 4. Exercise Studio's production backend

```sh
uv run --project "$aisle_root" --frozen --python 3.13.5 \
  python -B tools/aisle-validation/check_studio.py \
  "$run_root/lerobot" --studio "$studio_root"
```

Use the freshly generated fixture: this step deliberately changes its curation
sidecars while testing tags and exclusion. It verifies:

| Function | Assertion |
| --- | --- |
| Library and episode loading | One dataset, three episodes, 90 frames, correct Nano model |
| Joint playback | All 17 state/action channels load; radians reach the URDF pose stream unchanged |
| Video playback | 18 seeks, including backward seeks and final frames, return correctly sized RGB frames |
| Transport | Seek, frame-step, and play/pause intents update backend state correctly |
| Curation | A tag survives reopening; exclusion/Undo preserves the original Parquet bytes |

The script temporarily installs `check_nano_aisle.rs` as a Cargo example in
`dorobot-flex/examples/`. It refuses to overwrite an existing example and removes
its temporary example and build directory after the run. It writes
`studio-verification.json` and `studio-integration.log` beside the dataset.

These checks call the same production backend used by the UI. They do not click
the desktop window, validate its visual layout, or exercise the Record/Eval
backends.

## 5. Open the recording in Studio

Run from the Studio root so the bundled Nano URDF resolves:

```sh
cargo run --locked -p dorobot-flex -- \
  --dataset "$run_root/lerobot" --screen play
```

Keep default features enabled for video playback. For the current JIT launcher,
the corresponding command is:

```sh
./tools/run_jit.sh --video --dataset "$run_root/lerobot" --screen play
```

Select each episode and inspect both arms, the front-camera video, and the
timeline. The fixture has one camera; an empty wrist-camera pane is expected.
The reader and robot pose stream contain all 17 channels. At the reference
revision, state/action plots display only the first six channels; the checker
reports `plotted_channels` to make that limitation visible.

## Artifacts and interpretation

```text
<run_root>/
├── aisle-run/                     Original traces, RGB arrays, model contract, outcomes
├── aisle-export/                  AISLE's verified offline demonstration export
├── lerobot/
│   ├── meta/info.json             17-channel names, fps, dimensions and paths
│   ├── meta/episodes.jsonl        Episode index
│   ├── meta/tasks.jsonl           Motion-check descriptions
│   ├── meta/aisle_provenance.json Source timestamps, hashes, mappings and outcomes
│   ├── data/chunk-000/            Per-episode Parquet
│   └── videos/chunk-000/          Front-camera MP4s
├── result.json                    Simulation checks and AISLE export coverage
├── artifact-verification.json     Source-to-dataset checks
├── studio-verification.json       Production Studio backend checks
└── studio-integration.log         Cargo build and decoder diagnostics
```

Check `ok: true` in **all three reports**. A passing simulation report alone does
not establish that Studio imported the dataset correctly.

The original Metal reference run passed all three episodes and all six contract
tests. AISLE exported all 90 commands as eligible samples, with no unassigned
commands or exclusions. Maximum tracking error was approximately **0.0178 rad**;
no measured joint-limit excess occurred. Studio preserved all 17 pose channels
and passed its 18 video seeks, transport, and curation checks. These are recorded
software-test results, not tolerances guaranteed on every engine or machine.

## Scope and troubleshooting

- The model uses nominal masses/inertias and AISLE's default URDF gains and
  damping. This run does not establish contact fidelity, grasping, bimanual task
  competence, mobile-base control, hardware calibration, or robot safety.
- `nano_limits__joint_cmd_safe` contains the probe's URDF-limit-checked targets;
  its name is not evidence that AISLE's normal dataflow budget guard was run.
- This is a Studio regression fixture, not a certified training dataset. The
  `training_samples` coverage field is AISLE's eligibility count, not a training
  quality verdict.
- Physical hardware, multiple cameras, Studio recording, policy Eval, Hub
  transfers, and desktop mouse/keyboard acceptance are outside these checks.
- If the collector reports an existing output directory, choose a new run name.
- If a Nexus API is missing, compare your AISLE revision and Nexus wheel with the
  versions above. Do not replace the physics run with a synthetic preview.
- If the Studio build cannot find FFmpeg, check `pkg-config --modversion libavcodec`
  and the [Studio setup guide](../../docs/FUNCTIONAL_REVIEW.md). A build without
  video support cannot validate this recording's video path.
- If the robot view is empty, launch from the Studio root and check that
  `data/adora_nano/adora_nano.urdf` and its relative mesh paths exist.
- If a Studio check fails, inspect `studio-integration.log`; the recording and
  source evidence remain available for diagnosis.
