# Native Studio runtime

Studio launches `dorobot-nano-runtime`. The executable supervises a native Dora
1.0.1 graph; it does not invoke Python, uv, OpenCV or the AISLE Python package.

Simulation graph:

```text
gamepad → control → guard → Nexus simulation → recorder
                                   └───────→ preview → Studio
```

The room, nominal camera mounts and control behavior are ported from Studio's
AISLE Nano integration. The GPU physics/rendering engine is the same pinned
Dimforge Nexus revision used by RC.1. Source provenance and licenses are under
`vendor/` and `notices/`. The small pinned source subset avoids fetching upstream
media and keeps shader build dependencies consistent. Physics engine source is
unchanged; vendor workspace membership and local dependency patches are adjusted.

All Studio application logic, runtime nodes, serial control, recording and Hub
transfers are Rust. Video codecs still use dynamically linked FFmpeg, and the
app uses the operating system's Metal/AVFoundation frameworks. “Rust runtime”
does not mean every third-party library is implemented in Rust.

## Build

Requires Rust, cargo-gpu 0.10.0, Xcode command-line tools, pkg-config and FFmpeg
8.0.3 headers/libraries built with `tools/release/build_ffmpeg.sh`. The pinned
SPIR-V 0.10 shader compiler uses nightly-2026-07-03 with rustc-dev, rust-src and
llvm-tools. Install its build toolchain before an offline release build:

```sh
cargo install cargo-gpu --version 0.10.0 --locked --no-default-features
cargo gpu install --shader-crate runtime/vendor/vortx/vortx-shaders \
  --auto-install-rust-toolchain
PKG_CONFIG_PATH=/tmp/dorobot-rc-ffmpeg/lib/pkgconfig \
  cargo build --manifest-path runtime/Cargo.toml --release --locked
```

The runtime has its own Cargo workspace so Nexus/GPU dependencies do not change
Makepad's JIT dependency pins. Development Studio finds
`runtime/target/release/dorobot-nano-runtime`; `DOROBOT_NATIVE_RUNTIME` overrides
that location. Put native `dora` beside the executable, on PATH, or select it
with `DOROBOT_DORA`. Installed Studio resolves both inside its relocated bundle.

```sh
runtime/target/release/dorobot-nano-runtime sim \
  --model data/adora_nano/adora_nano.urdf \
  --output /tmp/new-nano-session --socket /tmp/new-nano.sock
```

`base --mock --self-test` exercises the same byte-level Feetech packet and ACK
handling as the real base without opening a serial device. Self-test is rejected
for a real base. Real Nano support remains limited to the supplied wheel driver;
no arm/head feedback or physical Nano camera implementation is fabricated.

## Data and shutdown

Preview sends the existing length-prefixed JSON/BGRA protocol over a local Unix
socket. Its bounded latest-frame slot may drop preview frames. The independent
recorder validates a contiguous sequence and waits for the producer's final
frame count before committing the dataset.

A saved episode contains three MPEG-4 videos, measured 17-joint state and motor
targets, scene transforms, and lossless NPZ RGB/depth/segmentation/pose samples.
Camera sampling remains 5 Hz of simulation time. Raw samples retain their source
physics clock. Manual task success is explicitly unevaluated.

L1 release, disconnected/stale input and reordered commands are guarded. A
physical Feetech worker has a separate watchdog thread, Studio lease and process
heartbeat. Every write requires a valid ACK; stopping attempts zero-speed and
torque-disable on all three motors even when one fails. SO-100/101 capture also
has a separate follower watchdog and stores the limited action actually sent.

Build and acceptance scripts may use the builder's Python; no Python executable,
package or source is included in the application bundle. The relocated bundle
acceptance test explicitly denies Python execution.
