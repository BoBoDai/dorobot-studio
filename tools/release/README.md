# DoRobot Studio macOS RC

RC.2 is a standalone Apple Silicon application with a Rust runtime. It includes
Makepad Studio, native Dora 1.0.1, the pinned Nexus GPU engine, Nano URDF/meshes,
controller and serial nodes, recording/replay, and dynamically linked FFmpeg.
The AISLE Nano scene integration is ported to Rust. No Python, uv or Python
bindings are packaged or launched.

## Use

See the [detailed user guide](../../docs/USER_GUIDE.md) for installation,
controller mappings, camera layouts, capture/replay, hardware calibration,
dataset transfers and troubleshooting.

Move **DoRobot Studio.app** to Applications. Open **Robot setup → Simulation →
Start simulation → Operate**. Start and stop a take with **Record**, then **Save**
and **Replay**. The first simulation launch prepares GPU shaders. A paired
PS4/PS5 controller is optional for viewing/recording; hold L1 to command motion.

Recordings default to `~/DoRobot/Recordings`; sessions/preferences live in
`~/Library/Application Support/DoRobot Studio`. The bundle stays read-only.
The RC targets macOS 14+ on Apple Silicon, is ad-hoc signed and is not notarized.

Nano simulation includes the room, cubes, mobile base, both arms, head, three
RGB/depth/segmentation cameras, measured joints and scene motion. Sampling stays
5 Hz in simulation time; this release makes no 30 FPS live claim. Real Nano
support is the supplied Feetech base adapter, with no invented arm/head/camera
capabilities. SO-100/101 capture and Hub transfer use native Rust workers.
Physical robot validation is separate from simulation/mock acceptance.

## JIT development

Rust, Xcode Command Line Tools, Git and pkg-config are developer prerequisites.
The existing JIT entry reuses the installed native runtime and video libraries:

```sh
DOROBOT_APP='/Applications/DoRobot Studio.app' \
  tools/release/run_jit_macos.sh --video
```

Use `--check --video` for the JIT check, or edit Rust/Splash and reload normally.

## Build

Build dependencies are only needed on the builder. Read `runtime/README.md` for
the pinned Rust shader compiler setup. Fetch Cargo dependencies, including the
shader workspaces, before an offline build. Install native dora-cli 1.0.1 using
a neutral Cargo home and Rust path remapping so private build paths are absent.

Obtain the unmodified FFmpeg 8.0.3 source archive:
`https://ffmpeg.org/releases/ffmpeg-8.0.3.tar.xz`, SHA-256
`6136812ea6d4e68bdba27e33c2a94382711cdf4f8602ffef056ff792bd6f9818`.
Run `tools/release/build_ffmpeg.sh` inside its extracted source directory.
It installs shared libraries/headers under `/tmp/dorobot-rc-ffmpeg`.

```sh
PKG_CONFIG_PATH=/tmp/dorobot-rc-ffmpeg/lib/pkgconfig \
  python3 tools/release/build_macos.py \
  --dora /path/to/native/dora \
  --ffmpeg-source /path/to/ffmpeg-8.0.3.tar.xz
```

The builder stages source under a neutral path (Makepad embeds manifest paths),
compiles Studio and the native runtime, copies model assets, relocates native
libraries, includes notices and FFmpeg source, and signs the app ad hoc. It
removes stale Python resources even when reusing an output directory. It excludes
local sessions, recordings, credentials and preferences. `--skip-build` is an
iteration aid; publish a clean commit with `studio_dirty: false` in release.json.

## Acceptance

```sh
python3 tools/release/verify_macos.py 'dist/DoRobot Studio.app' \
  --report /tmp/dorobot-bundle-report.json
python3 tools/release/smoke_macos.py 'dist/DoRobot Studio.app' \
  --out /tmp/new-dorobot-acceptance --remote /path/to/makepad-remote
```

Verification checks signatures, dependencies, deployment targets, versions,
symlink confinement, private build paths and absence of Python code/runtime.
The smoke test moves the app to a path with spaces, uses an empty HOME/system
PATH, denies source/cache reads, bundle writes, Python execution and external
network access, while permitting local Dora transport. It exercises simulation,
three-camera streaming, record/save/replay/discard, shutdown and mock-base flows.
The test harness uses the builder's Python outside the app's sandbox.

Publish the verified app ZIP, matching JIT source archive and checksums as a new
GitHub prerelease. Preserve RC.1. Apple notarization requires release credentials;
no credentials belong in source or distributables.
