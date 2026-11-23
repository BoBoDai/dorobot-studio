# SO-101 live follower mirror

## Intent and milestone

Mirror the physical SO-ARM101 follower in Dorobot Studio while a person operates the leader through Dorobot2. Prove the path from real encoder reads to the desktop viewer across two machines on the same LAN. The first milestone covers joint state, automatic discovery, and a read-only preview in Studio. Dorobot2 owns the hardware and the existing teleop session. The preview uses the measurements from that session, including the follower's response time and actual range of motion.

The milestone covers joint telemetry and its source status. Physical control from Studio, camera and dataset transfer, simulated contacts, and gamepad integration are separate milestones. The current SO-101 teleop smoke test with the relative target limit unset is the hardware baseline.

Develop both Dorobot2 and Studio on the existing Linux bench PC as separate processes. Verify the Linux Studio build and render path early, then exercise the SO-101 mirror on the same machine. The distributed acceptance run keeps Dorobot2 and the arms on Linux and moves Studio to a macOS desktop on the same LAN. Both stages use the same publisher, receiver, and model profile.

## Approaches

| Approach | Tradeoff |
| --- | --- |
| **Studio's existing URDF viewer** | Reuses `RobotModelLoader` and the embedded `RobotView`. A read-only panel and a generic telemetry source are enough to show measured poses. Selected for the first milestone. |
| **Extend the Nano/F710 physics runtime** | Provides physics and simulated sensors. The current Nano preview expects 17 joints and three cameras, and the physics setup selects Metal. Supporting SO-101 here would expand the scope of the first LAN test. |
| **Use the standalone Makepad viewer** | Supports opening an individual URDF for model checks. Its checkout expects a missing sibling Makepad tree. Studio already vendors the viewer and is the acceptance target. |

## Robot-side publication

Add an opt-in Zenoh publisher node to Dorobot2's generated teleop graph. The node subscribes to the configured follower's existing `all_joint` output. The arm node attaches a source instance ID, capture sequence, and capture time immediately after a successful bus read. The publisher uses the configured joint order to label the values, with explicit units from the SO-101 source profile. Validation fails clearly for mismatched joint counts or unsupported source profiles. A new setting selects the follower by the configured arm name.

Use a one-sample Dora input queue with oldest-sample eviction and a replaceable latest-sample slot. The publisher worker owns the Zenoh session, serialization, and transport. Use a declared publisher with drop-on-congestion and express delivery for these small pose packets. Send each new sample once at the existing arm rate, about 30 Hz on the bench rig. A stalled or disconnected network receiver must be unable to delay the serial loop. On node stop, close the session and release its presence token. Startup or transport failure is surfaced by the telemetry node and must leave the existing hardware graph usable.

Use the `eclipse-zenoh` version pinned by Dorobot2's lock for bench testing, currently 1.10.2. Make telemetry dependencies available as an opt-in install for the existing edge environment. Use a compatible Rust Zenoh version in Studio and confirm Python-to-Rust interoperability with the actual selected versions.

## Discovery and wire contract

Start both endpoints in Zenoh peer mode with multicast scouting enabled. Both applications can discover their peer locators on the same multicast-capable LAN, including when either endpoint starts late. Use liveliness tokens for application discovery, independently of Zenoh's transport discovery. Studio subscribes with history so it can list already-running sources. Support an explicit Zenoh peer endpoint or router in configuration for networks that filter multicast. Use the Zenoh API on both sides so their different Dora versions stay local to each host. For deterministic Linux development tests, configure a TCP loopback endpoint and disable Zenoh shared memory to exercise serialization and network transport. For the macOS LAN run, use peer scouting without a manually configured endpoint.

Each session declares an `alive` token, serves a small `descriptor` queryable, and publishes `joint_state` under `dorobot/telemetry/v1/robots/<robot_id>/sessions/<session_id>/`. The robot ID is stable and configurable. The session ID changes on publisher or arm source restart. Declare the descriptor and publisher before announcing the token. The descriptor carries `version`, `robot_id`, `session_id`, `model_id`, and `nominal_hz`. The arm source increments the sequence once per measured sample before any queue. A new session may begin at any sequence value. Later gaps indicate skipped samples. Capture timestamps use the arm host's monotonic clock domain. A sample may also carry `capture_utc_ns` for latency measurement with synchronized hosts.

A v1 state payload is UTF-8 JSON. Six joints at 30 Hz fit comfortably in this format and both stacks already have JSON support. An example payload shape follows.

```json
{
  "version": 1,
  "robot_id": "so101-bench",
  "session_id": "a660dc29-8090-4553-858c-0219442d9d4d",
  "model_id": "so101-follower-new-calib",
  "sequence": 42,
  "capture_monotonic_ns": 41000000000,
  "joints": [
    {"name": "shoulder_pan", "position": 4.2, "unit": "deg"},
    {"name": "shoulder_lift", "position": -35.0, "unit": "deg"},
    {"name": "elbow_flex", "position": 26.0, "unit": "deg"},
    {"name": "wrist_flex", "position": 8.0, "unit": "deg"},
    {"name": "wrist_roll", "position": 0.0, "unit": "deg"},
    {"name": "gripper", "position": 33.0, "unit": "percent_open"}
  ]
}
```

Support a variable set of joint names with `rad`, `deg`, `m`, and `percent_open` units. Limit v1 to 128 joints and 32 KiB per message. Validate identifiers, topic and payload agreement, finite positions, and unique names. Reject invalid packets, incompatible versions, and old or duplicate sequences within the same session. The receiver tracks diagnostics without keeping a backlog. Treat these payloads as untrusted input and resolve model IDs against installed local profiles.

## Studio preview and model mapping

Add a **Live mirror** tab to the existing Robot control plugin. Use the current Makepad `RobotView` and background `RobotModelLoader`. Add the Zenoh service to the shared Studio core, and expose its state through the existing backend and panel interfaces. The receiver owns one latest-frame slot per discovered session and expires inactive sources. Backend polling takes the latest frame, and the UI thread updates the viewer. The transport and parser know only the common schema. Local model profiles map joint names and units to a viewer pose. The Nano Unix socket, local capture, and dataset playback retain their own sources.

Select the sole discovered supported source automatically. When multiple sources exist, let the operator select one by robot and session. Display the selected source, sample rate, time since the last valid frame, and discovery or model errors. Mark a stream stale after one second without a valid sample and hold the last pose. Clear sequence tracking for a new session. Rejoin the same robot when precisely one replacement session is present. A duplicate active robot ID requires source selection. For an unknown model, display the connection and incoming joint list so the generic receiver remains useful.

Bundle the official SO-101 follower `so101_new_calib.urdf` and referenced meshes under `data/so101/`. Verify the same model loads from the desktop application's resource root. Pin [LeRobot's asset revision](https://huggingface.co/datasets/lerobot/robot-urdfs/tree/1b184a0e9ea822b8f8bfd1382f3ae0b28e1bfad3/so101) and retain the upstream Apache-2.0 attribution. The asset check verified six moving joints and all 13 relative mesh paths. This asset has a different joint order from Dorobot2's output. Build the renderer's pose vector by name in `RobotView` order. The five arm joints use radians in the viewer. Map the gripper's closed and open percentages to its revolute endpoints. Check joint direction, neutral pose, wrist-roll wrap, and gripper endpoints against physical motion before signing off on the SO-101 profile.

## Evidence required for acceptance

1. **Linux desktop smoke test.** Build Studio on the bench PC, connect a synthetic publisher over Zenoh TCP loopback, and load the SO-101 model. Display a deterministic sweep with permuted input order. Use a second synthetic model profile with a different joint count and units to exercise the same receiver.
2. **Linux hardware mirror.** Run Dorobot2 teleop and Studio together on the bench PC with the verified calibration. Move one joint at a time and then teleoperate normally. Verify the viewer follows the measured follower, including elbow and gripper, and that each named joint moves in the correct direction.
3. **Linux-to-macOS LAN test.** Keep Dorobot2 on Linux and run Studio on the Mac. Verify discovery without a manually configured IP in either startup order, the same URDF mapping, and recovery after disconnecting and reconnecting Studio.
4. **Latency and recovery.** Measure capture-to-viewer-update p50 and p95 during a sustained 30 Hz run. For capture-to-viewer measurements across hosts, synchronize their clocks and record the offset bound. Use a single high-frame-rate camera to measure physical-motion-to-visible-motion separately if clock synchronization is unavailable. Report sample age, gaps, and dropped frames. Use a provisional p95 target of 100 ms on a wired LAN, then report the measured result with the network setup.
5. **Isolation.** Slow the Studio consumer, close Studio, restart the publisher process, and reconnect. Verify the preview jumps to fresh poses after recovery. Compare the Dorobot2 joint tick rate and command path with telemetry toggled. Hardware teleop stays local through every network failure.
