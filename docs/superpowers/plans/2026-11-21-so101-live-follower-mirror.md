# SO-101 Live Follower Mirror Implementation Plan

> **For agentic workers**
>
> Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task by task. Use the checkboxes to track verification.

**Goal**

Show the measured SO-101 follower pose in Studio during Dorobot2 teleop, first on the Linux bench PC and then across the LAN to macOS.

**Architecture**

An opt-in Python Dora node sends the follower's measured positions to a Zenoh publisher worker. A robot-independent Rust receiver feeds the newest valid sample to a dedicated Studio panel. The panel maps the named joints into the existing URDF viewer through a local model profile.

**Tech stack**

Python 3.12, Dora 1.1.0, LeRobot 0.7.0, Eclipse Zenoh 1.10.2, UTF-8 JSON, Rust, and Studio's embedded Makepad `RobotView`.

**Spec**

[SO-101 live follower mirror](../specs/2026-11-21-so101-live-follower-mirror-design.md).

## Global Constraints

- Dorobot2 retains exclusive hardware access and the verified local teleop, with the relative target limit unset. Studio receives measured follower positions.
- Use the versioned topic and payload in the spec. Limit each payload to 32 KiB and each pose to 128 named, finite joint values. Honor `rad`, `deg`, `m`, and `percent_open`.
- Keep the latest frame under congestion. Carry capture sequence and monotonic time from the arm node. Enable peer scouting by default and support explicit endpoints.
- Develop with separate processes on Linux and TCP loopback. Validate automatic discovery on a second machine running macOS. Measure the provisional 100 ms p95 capture-to-viewer target on a wired LAN.
- Keep model interpretation inside local Studio profiles. The first asset is `so101_new_calib.urdf` at LeRobot asset revision `1b184a0e9ea822b8f8bfd1382f3ae0b28e1bfad3`.
- Run repo-relative commands from the indicated repository. Use two isolated worktrees at execution time. Base Dorobot2's source work on the verified Dora/LeRobot pair according to `dorobot2/docs/BRANCHING.md`. Base Studio's work on the approved design commits. Keep local hardware configuration and calibration out of source control.

## Review Focus

- A configured motor order that differs from the arm node's actual order must fail before telemetry can mislabel angles. Task 1.
- A blocked or unavailable network publisher must leave the graph publishing and coalesce to the newest pose. Task 2.
- A previous session's delayed sample must never replace a selected replacement session. Task 3.
- A shot of stale pose data or a profile mismatch must never move a newly loaded model. Tasks 4 and 5.
- A desktop that opens late or restarts must discover the running robot and resume at a fresh pose. Task 6.

## File Map

| Repository | Files and responsibility |
| --- | --- |
| Dorobot2 | `dorobot/config/loader.py`, `dorobot/graph.py`, and `dorobot/nodes/arm_node.py` select and tag the measured source. |
| Dorobot2 | `dorobot/telemetry.py` defines the v1 payload builder and Zenoh worker. `dorobot/nodes/zenoh_telemetry_node.py` connects it to Dora. |
| Studio | `crates/dorobot-studio-core/src/api/telemetry/{mod,wire,receiver}.rs` define the common wire types, source catalog, and network worker. |
| Studio | `crates/dorobot-studio-core/src/data/telemetry_model.rs` maps decoded joints to a local model. `data/so101/` holds the verified asset and attribution. |
| Studio | `crates/dorobot-studio-core/src/api/{mod,files,mock}.rs`, `dorobot-flex/src/bin/dorobot-flex.rs`, and `plugins/control/src/{lib,live_mirror}.rs` expose the source and bind it to the viewer. |

## Task 0 — Verify the Linux desktop baseline

- [ ] Confirm both worktrees identify the expected pins. From Dorobot2, run `.venv/bin/dora --version` and `.venv/bin/dorobot teleop -c configs/so101.yaml --dry-run`. Expect Dora 1.1.0 and a generated graph. Use the worktree's own environment or diagnostics with the equivalent activated environment.
- [ ] From Studio, run `cargo build --locked -p dorobot-flex --no-default-features`. Fix fetch or Linux build prerequisites at their source and record the build command and any prerequisites in the runbook. Keep dependency pins in place unless a verified blocker calls for a scoped change.
- [ ] Run `cargo run --locked -p dorobot-flex --no-default-features -- --demo` from Studio. Open a built-in robot preview, orbit its model, and confirm the native renderer works on this Linux display. This baseline uses the root Cargo workspace.

## Task 1 — Capture and route measured follower samples

**Dorobot2 files**

Modify `dorobot/config/loader.py`, `dorobot/graph.py`, `dorobot/nodes/arm_node.py`, and `tests/test_graph.py`. Create `tests/test_telemetry_capture.py`.

**Interfaces**

- `TelemetryConfig(robot_id: str, follower_arm: str, mode: str = "peer", multicast: bool = True, connect_endpoints: tuple[str, ...] = (), listen_endpoints: tuple[str, ...] = ())`. `DoRobotConfig.telemetry` holds it when the YAML has a `telemetry` mapping.
- The optional `zenoh_telemetry` node consumes input `joint_state` from `arm_<follower_arm>/all_joint`. Its environment carries `DOROBOT_TELEMETRY_CONFIG` as a JSON object with the `PublisherSettings` fields from Task 2.
- The selected arm adds `source_instance_id`, `capture_sequence`, `capture_monotonic_ns`, and `capture_utc_ns` to Dora metadata. It stamps each successful read before emitting the matching `all_joint` array.

- [ ] Add `test_telemetry_routes_measured_follower`, `test_telemetry_rejects_unsupported_joint_order`, and `test_arm_metadata_tracks_successful_captures`.

```python
assert node["inputs"]["joint_state"] == {
    "source": "arm_follower/all_joint", "queue_size": 1, "queue_policy": "drop_oldest"
}
assert graph_without_telemetry == existing_graph
with pytest.raises(ValueError, match="joint order"):
    generate_teleop_dataflow(config_with_reordered_motor_list)
assert [first["capture_sequence"], after_failed_read["capture_sequence"]] == [0, 1]
assert first["source_instance_id"] == after_failed_read["source_instance_id"]
```

- [ ] Run `pytest tests/test_graph.py tests/test_telemetry_capture.py -q` and confirm the new assertions fail with the existing graph or metadata behavior.
- [ ] Add the config and graph tap to both teleop graph entry paths. Require a unique follower with driver `so101` and the arm node's six-joint order. Use the shared default order for capture and validation, preserving the existing command path. Keep the measured array format and add metadata only to the opted-in source.
- [ ] Rerun those tests and `pytest tests/test_dora_v110_runtime.py -q`. Require the new tests and the unchanged hardware-independent graph checks to pass. Commit the graph and capture slice after the publisher in Task 2 can launch its generated node.

## Task 2 — Publish with bounded asynchronous handoff

**Dorobot2 files**

Create `dorobot/telemetry.py`, `dorobot/nodes/zenoh_telemetry_node.py`, and `tests/test_zenoh_telemetry.py`. Modify `pyproject.toml` and `uv.lock` for an opt-in `telemetry` extra pinned to `eclipse-zenoh==1.10.2`.

**Interfaces**

- `CaptureFrame(source_instance_id: str, sequence: int, capture_monotonic_ns: int, capture_utc_ns: int | None, positions: tuple[float, ...])`.
- `encode_joint_state(settings: PublisherSettings, session_id: str, capture: CaptureFrame) -> bytes`. `PublisherSettings(robot_id: str, model_id: str, joint_names: tuple[str, ...], joint_units: tuple[str, ...], nominal_hz: float, mode: str, multicast: bool, connect_endpoints: tuple[str, ...], listen_endpoints: tuple[str, ...])` decodes the node configuration from Task 1.
- `PublisherWorker(settings: PublisherSettings)`, with `offer(capture: CaptureFrame) -> None` and `close() -> None`. The node reads metadata and passes a capture to this worker. Treat source open and publish errors as worker status.

- [ ] Add `test_wire_units_and_sequence`, `test_blocked_publish_replaces_pending_sample`, `test_failed_session_keeps_offers_nonblocking`, and `test_source_restart_renews_session`. Use a fake Zenoh session behind the worker in unit tests.

```python
assert decoded["sequence"] == 42
assert [joint["unit"] for joint in decoded["joints"]] == ["deg"] * 5 + ["percent_open"]
assert published_sequences == [42, 90]  # Worker was stalled while 43 through 90 arrived.
assert session_after_source_restart != session_before_source_restart
assert missing_network_did_not_block_offer
```

- [ ] Run `pytest tests/test_zenoh_telemetry.py -q` and confirm the missing module or wire behavior is reported.
- [ ] Implement the v1 encoder, topic validity checks, and single replaceable slot. In the worker, configure Zenoh peer/client endpoints, disable shared memory, declare descriptor and pose before liveliness, and use DROP and express for poses. Reopen failed sessions with bounded retry delay and release them on exit. Catch missing optional dependency at node startup and expose an actionable diagnostic while the graph continues. Exercise publisher process restart or failure against real Dora 1.1.0 before the hardware run.
- [ ] Run `pytest tests/test_zenoh_telemetry.py tests/test_telemetry_capture.py tests/test_graph.py tests/test_dora_v110_runtime.py -q` and `ruff check dorobot/ tests/`. Require both to pass. Commit the Dorobot2 slice with the version-prefixed commit subject from `AGENTS.md`.

## Task 3 — Receive and track generic Studio streams

**Studio files**

Create `crates/dorobot-studio-core/src/api/telemetry/{mod,wire,receiver}.rs`. Modify `crates/dorobot-studio-core/src/api/mod.rs`, `crates/dorobot-studio-core/Cargo.toml`, and `Cargo.lock`. Add tests alongside their modules.

**Interfaces**

- `PoseFrame::decode(key: &str, payload: &[u8]) -> Result<PoseFrame, DecodeError>` and equivalent descriptor validation. `SourceId` contains robot and session IDs. `PoseFrame` carries the v1 fields, with `joints: Vec<JointValue>`. Define `JointValue { name: String, position: f64, unit: JointUnit }` and `JointUnit::{Rad, Deg, M, PercentOpen}`.
- `SourceCatalog::accept_pose(&mut self, frame: PoseFrame, received_at: Instant) -> bool`, `SourceCatalog::latest(&self, source: &SourceId) -> Option<&ReceivedPose>`, and `SourceCatalog::is_stale(&self, source: &SourceId, now: Instant) -> bool`. `ReceivedPose` holds the pose and local monotonic arrival time.
- `ZenohReceiver::start(options: ZenohOptions) -> Result<ZenohReceiver, ReceiverError>` starts a background worker. `ZenohReceiver::poll(&mut self, now: Instant) -> &SourceCatalog` exposes the current catalog without network waits.

- [ ] Add `wire_validates_topic_and_bounds`, `catalog_rejects_out_of_order_and_keeps_latest`, and `catalog_isolates_restarted_session`. Assert malformed, oversized, duplicate-joint, non-finite, version-mismatched, and key/payload-mismatched frames are rejected.

```rust
assert!(!catalog.accept_pose(old_sequence.clone(), now));
assert_eq!(catalog.latest(&replacement).unwrap().frame.sequence, 0);
assert!(!catalog.accept_pose(delayed_from_retired_session, now));
assert!(catalog.is_stale(&replacement, now + Duration::from_millis(1001)));
```

- [ ] Run `cargo test -p dorobot-studio-core --no-default-features telemetry` and confirm failure on the missing wire or catalog API.
- [ ] Implement the wire codec and session catalog. Add Rust Zenoh 1.10.2 with TCP support. Offload discovery, descriptor fetch with a deadline, and subscription IO to the worker. Use liveliness history, bounded messages and session state, local arrival times, and expiry. Keep one valid latest pose per discovered active session through polling and count sequence gaps.
- [ ] Run `cargo test --locked -p dorobot-studio-core --no-default-features telemetry`. Require the protocol and fake-clock catalog tests to pass. Commit the receiver and lockfile.

## Task 4 — Load the official model and map named joints

**Studio files**

Create `data/so101/` with the pinned follower URDF, its 13 referenced meshes, license, and provenance. Create `crates/dorobot-studio-core/src/data/telemetry_model.rs` and register it from `data/mod.rs`.

**Interfaces**

- `TelemetryModel::resolve(model_id: &str, studio_root: &Path) -> Option<TelemetryModel>` uses installed profiles and assets.
- `TelemetryModel::viewer_angles(&self, joints: &[JointValue], viewer_order: &[String]) -> Result<Vec<f32>, PoseError>` returns a complete pose in viewer order.

- [ ] Add `so101_asset_loads_all_six_joints`, `permutes_by_name_and_converts_units`, `rejects_incomplete_pose`, and `generic_units_support_prismatic_joint`. Check the model through Studio's URDF loader.

```rust
assert_eq!(robot.num_movable_joints(), 6); // Robot returned by the Studio asset loader.
assert!((angles[pan_index] - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
assert!((angles[gripper_index] - 1.74533).abs() < 1e-5); // 100% open.
assert!(model.viewer_angles(&missing_elbow, &viewer_order).is_err());
```

- [ ] Run `cargo test --locked -p dorobot-studio-core --no-default-features telemetry_model` and confirm the new mapping or asset check fails.
- [ ] Add the pinned asset and apply explicit per-joint conversions by name. Map 0% and 100% gripper to the official endpoints as the initial profile. Use the installed asset root and return an error for an unknown unit, missing configured joint, or incomplete mapping. Adjust signs or offsets only where the one-joint physical checks demonstrate a difference.
- [ ] Rerun the model tests. The synthetic two-joint profile with a prismatic value in metres must use the same mapping path. Verify the asset resolves from a staged desktop resource root. Commit the asset and profile.

## Task 5 — Add Studio's Live mirror panel

**Studio files**

Modify `plugins/control/src/lib.rs`, `crates/dorobot-studio-core/src/api/{mod,files,mock}.rs`, `dorobot-flex/src/bin/dorobot-flex.rs`, and `dorobot-flex/tests/plugins.rs`. Create `plugins/control/src/live_mirror.rs`.

**Interfaces**

- Add `Backend::telemetry(&self) -> Option<&TelemetryState>` and `Intent::SelectTelemetrySource(SourceId)`. Add `FileBackend::configure_telemetry(&mut self, options: ZenohOptions)` and call it from the host's production backend factory. Poll the worker through `FileBackend::poll()` and keep the existing mock backend network-free.
- Add `Robot control → Live mirror` panel ID `hardware_live_mirror` with no legacy screen mapping. Use `RobotModelLoader` and `RobotView` on the current Studio graphics path.
- Support Studio startup options `--zenoh-connect <endpoint>` and `--zenoh-mode peer|client` for loopback or a router. An explicit endpoint disables multicast for deterministic loopback or router tests. A start with neither option uses peer scouting.

- [ ] Add `live_mirror_registers_without_replacing_operate` and `mirror_selection_holds_pose_until_matching_model_is_ready`. Verify existing Operate remains registered and a late model result cannot apply a previous source's pose. Add backend coverage for an unsupported model's read-only joint list, and duplicate robot IDs requiring a selection.

```rust
assert!(panel_ids.contains(&"hardware_teleop"));
assert!(panel_ids.contains(&"hardware_live_mirror"));
assert_eq!(rendered_source_after_late_load, selected_source_b);
assert!(duplicate_robot_selection.is_none());
```
- [ ] Run `cargo test --locked -p dorobot-flex --no-default-features --test plugins` plus `cargo test --locked -p dorobot-studio-core --no-default-features telemetry`. Confirm the new registration and selection cases fail before the change.
- [ ] Add the tab, selection/state wiring, and status labels for search, live, stale, disconnected, and model errors. Use the existing 60 Hz backend/UI poll and apply only the newest matching mapped frame. Show robot/session, receive rate, age, and missed samples. Preserve the last pose when stale. Keep the existing replay and Nano sources in their current paths.
- [ ] Rerun those tests and `cargo build --locked -p dorobot-flex --no-default-features`. Open the Linux panel and verify source selection, asset loading, camera orbit, and the absence of a control path in the panel. Commit the UI slice.

## Task 6 — Prove the Python-to-Studio path

**Files**

Create `dorobot2/tools/telemetry/synthetic_zenoh.py` and Studio `docs/SO101_LIVE_MIRROR.md`. Use the real worker from Task 2 for synthetic samples.

- [ ] Add a synthetic source accepting `--listen <endpoint>`. Publish a deterministic six-joint sequence with independently moving elbow and gripper. From Dorobot2, run `.venv/bin/python tools/telemetry/synthetic_zenoh.py --listen tcp/127.0.0.1:7447`. From Studio, run `cargo run --locked -p dorobot-flex --no-default-features -- --zenoh-connect tcp/127.0.0.1:7447` and open Live mirror. Confirm the asset draws and the displayed sequences advance. Test Studio opening late, stale pose after publisher loss, and a fresh session after restart.
- [ ] Add telemetry to a copy of the existing ignored hardware config. Set `telemetry.robot_id: so101-bench`, `telemetry.follower_arm: follower`, `telemetry.multicast: false`, and `telemetry.listen_endpoints: [tcp/127.0.0.1:7447]` for bench loopback. Generate the active graph with `dorobot teleop -c <local-config> --dry-run --output <local-graph>`. Confirm its teleop command edge and current calibration paths. With the user at the cleared bench, run `dora run <local-graph> --stop-after 35s` and observe one-joint motion followed by ordinary teleop. Verify there are no axes reversed, the gripper endpoints match, and read-only telemetry reflects the measured follower. Compare rate and recovery when Studio exits or is slowed. Confirm both arms release torque at shutdown.
- [ ] On macOS, build Studio with the same model resources, open Live mirror, and run the Linux publisher with peer scouting and empty endpoint lists. Test both application startup orders on the same LAN. Record the real pose, viewer lag, disconnect/reconnect, and unchanged robot tick rate. Use synchronized clocks for capture-to-viewer p50/p95 and state the clock uncertainty with the result.
- [ ] Run the targeted suites from Tasks 1–5 and the Studio workspace suite with `cargo test --locked --workspace --no-default-features`. Record outcomes and reproducible commands in the runbook. Mark cross-host acceptance pending if the Mac is unavailable. Commit the verified runbook and synthetic tool in their respective repositories.
