# DoRobot Studio macOS RC

The default release is a standalone Apple Silicon app. It contains the native
Makepad UI, Python 3.13.5, AISLE at the revision in `release-config.json`, Nexus
0.2.1, Dora CLI/API 1.0.1, the Nano URDF and meshes, the room/teleop/recording Dora
nodes, controller and serial dependencies, and FFmpeg for replay. Starting Nano
does not install packages, clone repositories, or require a system Python.

## Install and use

Unzip the macOS artifact and move **DoRobot Studio.app** to Applications. Open
**Robot setup → Simulation → Start simulation**, then **Operate**. Use **Record**
to start/stop a take, **Save**, and **Replay** to inspect the saved episode.
The first simulation launch prepares GPU shaders. A paired PS4/PS5 controller is
optional for viewing/recording; hold L1 while commanding motion.

Recordings default to `~/DoRobot/Recordings`. Temporary sessions and preferences
are in `~/Library/Application Support/DoRobot Studio`. The app bundle remains
read-only. Closing Studio stops its supervised simulation/driver processes.

This RC is ad-hoc signed, not Apple notarized. macOS may require the user to
approve opening the downloaded app in Privacy & Security. The build targets
macOS 14 or later on Apple Silicon; acceptance is performed on macOS 26.6.2.
Intel, Windows and Linux binaries are not included in this RC.

## Scope

- Nano room physics, mobile base, both arms, head, cubes and three camera views.
- Dora graph supervision, gamepad guard, live preview, recording and LeRobot
  dataset replay with measured joints and scene state.
- Feetech real base adapter and byte-level mock bus. Real arm/head calibration
  and real Nano camera capture are **not implemented** by this adapter.
- The optional SO100/SO101 LeRobot capture stack is not bundled. It requires a
  separate environment selected with `DOROBOT_PYTHON`.
- Camera sampling is 5 Hz in simulation time; this RC makes no 30 FPS live claim.
- AISLE's Nano/Nexus runtime source is included, not its optional Genesis,
  training/VLA model weights or remote experiment infrastructure.

## JIT development entry

The source artifact retains the pinned Makepad JIT workflow. A developer needs
Rust, Xcode Command Line Tools, Git and pkg-config; first setup may download
build dependencies. The developer entry reuses the installed app's Python,
AISLE, Dora, models and FFmpeg headers/libraries:

```sh
DOROBOT_APP='/Applications/DoRobot Studio.app' \
  tools/release/run_jit_macos.sh --video
```

Edit Rust/Splash in the source checkout and use the existing JIT reload flow.
`--check --video` checks the JIT build without opening another UI. The normal
source-only `tools/run_jit.sh` entry also remains available.

## Build the RC

Use a clean Studio checkout and AISLE at the exact pinned revision. Build tools
are needed on the builder only. Install Python 3.13.5 with `uv python install
3.13.5` and populate Cargo dependencies with `cargo fetch --locked`.

Download the unmodified `ffmpeg-8.0.3.tar.xz` source archive from
<https://ffmpeg.org/releases/ffmpeg-8.0.3.tar.xz>. Its SHA-256 must be
`6136812ea6d4e68bdba27e33c2a94382711cdf4f8602ffef056ff792bd6f9818`.
Run `tools/release/build_ffmpeg.sh` from the extracted FFmpeg source directory;
it installs the required shared libraries/headers at `/tmp/dorobot-rc-ffmpeg`.

```sh
PKG_CONFIG_PATH=/tmp/dorobot-rc-ffmpeg/lib/pkgconfig \
  python3 tools/release/build_macos.py \
  --aisle /path/to/pinned/AISLE \
  --ffmpeg-source /path/to/ffmpeg-8.0.3.tar.xz
```

The output is `dist/DoRobot Studio.app`. The builder copies the source to a
neutral temporary directory because Makepad embeds manifest directories. It
copies only tracked runtime/model files, installs hash-locked Python wheels,
relocates native libraries and Python configuration, includes dependency notices
and FFmpeg source, and signs the bundle ad hoc. It never copies local runs,
recordings, credentials or personal preferences. `--reuse-python` and
`--skip-build` are iteration aids; a release should start from a clean output.

`release.json` records the exact Studio and AISLE revisions. A final release
must have `studio_dirty: false`. Do not manually edit the embedded manifest.
AISLE's pinned revision has no top-level license declaration; the bundle records
that fact and preserves source notices rather than inventing an upstream license.

## Verify and publish

```sh
python3 -B tools/release/verify_macos.py 'dist/DoRobot Studio.app' \
  --report /tmp/dorobot-bundle-report.json
python3 -B tools/release/smoke_macos.py 'dist/DoRobot Studio.app' \
  --out /tmp/new-dorobot-acceptance \
  --remote /path/to/makepad-remote
```

The verifier checks signing, external native dependencies, escaped symlinks,
personal build paths, Python imports and version pins. The smoke test copies the
app to a different path containing spaces, starts with an empty HOME and system
PATH, denies reads from the source/build/caches, makes the app read-only, and
blocks external network access while permitting local Dora and Unix sockets.
It drives real Makepad input through simulation, three-camera streaming, record,
save, replay, discard, stop, error handling and mock-base journeys. Physical
robot testing is separate.

After verification, commit and push the release source, archive the `.app`,
export the same commit as the JIT source archive, and publish both with checksums
under a GitHub **prerelease** tag. Developer ID signing and Apple notarization
can replace ad-hoc signing when release credentials are available; no signing
credentials belong in the repository or distribution.
