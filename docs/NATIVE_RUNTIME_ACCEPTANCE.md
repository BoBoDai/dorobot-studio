# Native Rust RC.2 acceptance

Verified on macOS Apple Silicon on 2026-11-15. This checks the native runtime
and the standalone application; it is not a new visual UX score or a physical
robot certification.

## Results

| Check | Result |
|---|---|
| Studio Rust unit tests | 56 passed |
| Native runtime unit tests, without the optional GPU feature | 13 passed |
| Separate-process Dora mock-base integration | 2 passed, including suspended-input watchdog and neutral rearm |
| Packaged native GPU simulation | 96 frames, all 17 joints commanded and measured moving, 3 synchronized cameras |
| Independent sensor audit | 96 raw checksums, 288 camera poses, all 96 frames decoded from each video |
| Relocated application workflow | 16 checks passed with empty HOME, Python execution denied, external network denied, source/cache access denied and bundle writes denied |
| Bundle verification | No Python code/runtime, no private build paths, valid ad-hoc signatures, confined native dependencies, macOS 14 deployment target |
| Makepad JIT check with video | Completed with 0 errors; existing dependency/import and unused-code diagnostics remain |

The GPU test drives the mobile base, both arms and head through the actual Dora
graph, reads measured state from Nexus, and verifies recording continuity at
200 ms simulation intervals. It checks that the base stays on the floor and
that no root-pose writes are used to fake navigation. Native URDF import now
requires STL collision meshes and rejects missing collision geometry.

The moved-app test covers starting simulation, three live cameras, navigation
while recording, waiting for all sensor frames, retaining an unsaved take,
saving, replaying all 17 measured joints, discarding a later take, clean
simulation shutdown, invalid hardware/profile errors, calibration gating,
mock-base operation and stopping the base when leaving its controls. Live and
replay screenshots were inspected. The preview transport also has a regression
test for a complete three-camera packet under socket backpressure on macOS.

## Reproduce

Build prerequisites and pinned engine revisions are documented in
[`runtime/README.md`](../runtime/README.md). Packaging and isolated application
acceptance are documented in [`tools/release/README.md`](../tools/release/README.md).

```sh
cargo test -p dorobot-flex --lib --locked
cargo test --manifest-path runtime/Cargo.toml --locked --no-default-features --lib
cargo test --manifest-path runtime/Cargo.toml --locked --test dora_base \
  -- --ignored --test-threads=1
cargo test --manifest-path runtime/Cargo.toml --locked --test dora_sim \
  -- --ignored --nocapture --test-threads=1
```

The integration tests require native Dora 1.0.1. The GPU test requires Metal;
`DOROBOT_TEST_RUNTIME` can select the packaged runtime executable. The sensor
audit in `tools/aisle-validation/verify_session_record.py` runs independently on
the builder, outside the application sandbox. Build/test tooling may use Python;
the distributed application does not include or execute it.

## Limits

- Camera sampling is 5 Hz in simulation time. These checks do not establish
  30 FPS real-time streaming or task success for autonomous cube pickup.
- Camera intrinsics and mounting transforms are nominal simulation values.
- Physical Nano base, SO-100/101 calibration/capture and authenticated Hub
  upload need validation with the respective devices/account. Nano physical
  support remains limited to the supplied Feetech wheel driver.
- Video codecs use native FFmpeg; Metal/AVFoundation are system frameworks.
  Application logic and runtime nodes are Rust, not every linked dependency.
- The RC is ad-hoc signed and is not Apple-notarized.

Local recordings, screenshots, process logs, device paths, preferences and
credentials are excluded from source and release assets. Release validation
summaries contain only sanitized results.
