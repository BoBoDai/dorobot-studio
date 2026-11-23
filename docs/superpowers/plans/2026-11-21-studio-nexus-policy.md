# Studio Nexus Policy Workflow Implementation Plan

> **For agentic workers:** Verify each subsystem with Superpowers test-driven development. Use the interfaces below to integrate the native runtime, LeRobot worker, shared backend, and Makepad panels.

**Goal** Run the approved SO-101 collect, review, train, and infer path inside Studio.

**Architecture** Add one native policy plugin, extend the existing control and collection plugins, and keep one local workflow supervisor in Studio core. A Rust Nexus runner owns physics and a local LeRobot worker owns episode serialization and policy work.

**Spec** [Studio Nexus workflow design](../specs/2026-11-21-studio-nexus-policy-design.md)

## Global constraints

- Preserve existing Nano scenes, physical capture, playback and live mirror.
- Use the checked-in SO-101 URDF, one 640×480 RGB camera, and a cube-and-plate Nexus scene. Advance five 0.01 s physics steps per sample (20 Hz sim time). State and action use the same six named axes in degrees and percentage gripper opening in LeRobot.
- Data source Nexus simulation, execution target local, first recipe ACT. Show job execution outcome apart from task success.
- All paths come from per-workspace settings or application resource discovery. The command only opens the specified calibrated leader. A model rollout opens no serial port.

## Interfaces

The Rust runner is `dorobot-so101-session`. Run `capture MODEL SCENE SESSION` or `rollout MODEL SCENE INITIAL_JSON STEPS`. Launch capture with the existing probe protocol: child stdout is per-frame JSON line followed by exactly 921600 RGB bytes. It opens SESSION/leader.sock for a JSON array of 5 degree angles and 1 percentage. It observes `reset-request` and `stop` markers and reports `reset_id`. Pause/resume use presence of `pause`; `step-request` advances one physics step while paused. Keep the existing disposable probe's frame fields and rollout stdin command format. Add scene `data/so101/cube-to-plate.json`.

Studio runs `python -u tools/nexus_workflow/worker.py CONFIG_JSON` with a distinct run folder. CONFIG_JSON fields are version (1), kind (capture/train/rollout), runtime, model, scene, leader_port, calibration_dir, dataset, checkpoint, steps and output_root. Paths are absolute for existing inputs and outputs; runtime can be ignored for training. Write resources beside config unless output_root is specified. Training accepts one v3 dataset or a directory of v3 datasets. Respect `meta/dorobot_excluded.json`. Use local/offline operation and no Hub uploads. Optional `synthetic` config bool defaults false; with true use non-hardware repeatable leader input for checks. Dataset aggregation and runnable checkpoints are published only after acceptance.

Worker atomically replaces `status.json`. Its fields are version (1), phase, message, ack (last command id), frames, steps, total_steps, loss (optional), dataset, checkpoint, video (optional absolute paths), task_success (optional bool), source_mode (optional), positions and targets (optional arrays of 6 radians). Unused optional keys may be omitted. Phases are starting, reset_required, resetting, ready, paused, recording, review, saving, saved, training, running, stopping, stopped, completed and failed. Capture continues after saved for another take. Review awaits save/discard. Previews atomically replace `preview.rgb` as big-endian u64 sequence, u16 width, u16 height, then packed RGB24. Studio writes `command.json` atomically with version (1), nonzero id and action (reset, pause, resume, step, record, finish, save, discard, stop). One request outstanding until ack, except stop may override any pending request. Worker ignores previously acknowledged ids. Unknown/invalid-in-state actions acknowledge and give reason.

Rust public API in `core::api::nexus` exports `NexusSettings` (String fields python, leader_port, calibration_dir, dataset, checkpoint, output_root, train_steps, rollout_steps); `NexusState` (settings, snapshot, preview, job_dir, busy, command_pending); snapshot as status fields, preview as sequence,width,height,rgb. `NexusIntent::{Configure(NexusSettings), StartCapture, Reset, Pause, Resume, Step, Record, Finish, Save, Discard, Stop, Train, Run, UseOpenDataset, OpenSavedDataset}`. Wrap in `Intent::Nexus`; Backend returns `fn nexus(&self) -> Option<&NexusState>` with default None. Preserve latest valid dataset/checkpoint through new phase snapshots. Resource overrides DOROBOT_LEROBOT_PYTHON and DOROBOT_SO101_RUNTIME. Optionally DOROBOT_SO101_LEADER and DOROBOT_SO101_CALIBRATION seed settings.

## Review focus

- Capture commands depend on reset and recording phase; no cancelled partial episode gets labeled saved.
- Paths and manifests verify before launch. Missing or incompatible checkpoint gets a real actionable error.
- Supervisor and workers shut down together; no abandoned simulator or trainer holding resources.
- Syncing preview never makes the UI or episode recording block on one another.
- Existing data exclusions and command mapping stay effective across via shared state.

## Tasks

### Task 1 — Native SO-101 Nexus runner
- [x] Add a red test for the six-joint model/profile while preserving Nano parsing.
- [x] Integrate the tested fixed-base tabletop and WebGPU selection, validated scene, cube reset, pause/step and synchronized capture/rollout.
- [x] Verify release builds and a synthetic short run with dimension and target handshake assertions.

### Task 2 — LeRobot session worker
- [x] Add a red test for reset gating and staged capture review prior to v3 submission.
- [x] Bridge capture, acknowledged controls, preview/status, LeRobot v3 write; train with ACT and capture real steps/loss/models; run closed-loop rollout with compatibility validation and saved video.
- [x] Verify data selections/exclusions, retained evidence, and local worker process exit.

### Task 3 — Shared Rust workflow supervisor
- [x] Add a red test for command availability, packet size/identity and non-successful exit.
- [x] Add the shared backend workflow with asynchronous status/preview loading, state preservation and pending command arbitration. Link Data Workspace path into training and latest saved dataset to replay.
- [x] Verify the test backend and legacy application suite.

### Task 4 — Native workflow panels
- [x] Add a red test for registration of one policy plugin with training and inference tabs.
- [x] Extend Control setup and Collection record with a shared camera and robot pane. Add native policy tabs, progress/loss/checkpoint controls, and data handoffs. Preserve legacy navigation.
- [x] Run the full Studio suite. Exercise the actual application with synthetic input, v3 recording, ACT, and closed-loop Nexus. Inspect the displayed states and confirm runtime cleanup.
