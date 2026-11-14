# Makepad JIT and iOS appearance

The Studios run as desktop Rust applications through Makepad's Relax JIT.
This is an iOS visual style on desktop; it does not package an iPhone app.

## Runtime

- Reference: [OctoSense main, 388e2f38](https://github.com/OctoSense-org/OctoSense/commit/388e2f3882c1d7b3776fee2dc6b4d875abecc557), checked November 13, 2026.
- JIT integration: [OctoSense jit, 14f258d4](https://github.com/OctoSense-org/OctoSense/commit/14f258d409b3a2b394852a8fa23e185717fdc99e).
- Runtime and runner: [OctoSense Makepad, 44d8220f](https://github.com/OctoSense-org/makepad/commit/44d8220f12acfa46157019267e119e0866dd6b05).

`jit-runtime.json`, the Cargo dependencies and the launcher's runner all use
that same revision. The runner must match the framework's procedural macros.
The root Cargo patches also keep the layout crate, URDF renderer and bitflags
macros on one source revision. `Cargo.lock` remains committed for native builds;
this Relax revision uses its own resolver and does not accept Cargo's `--locked` flag.

## Launch

From this checkout:

```sh
./tools/run_jit.sh --setup         # optional: prepare the shared cached runner
./tools/run_jit.sh --check         # type/borrow checking through Relax
./tools/run_jit.sh                 # light appearance
./tools/run_jit.sh --dark
./tools/run_nano_preview.sh        # synthetic Nano clips, then Play
./tools/run_jit.sh --video --dataset /path/to/dataset
```

The gear switches light/dark appearance. Source edits are watched by Relax;
supported edits patch the running process. Changes to an active long-running
function can require a restart, which the runner reports.

The initial runner build needs a current stable Rust toolchain, Git, Python 3
and the platform C toolchain. On macOS use Xcode command-line tools. The default
launch disables optional FFmpeg decoding; `--video` requires the FFmpeg
development libraries described in the README. Linux additionally needs the
usual Makepad graphics and file-dialog development libraries.

Both launchers share `$XDG_CACHE_HOME/dorobot/makepad-jit/<revision>` (or
`~/.cache/dorobot/makepad-jit/<revision>`). `MAKEPAD_JIT_SOURCE` can reuse a
checkout at the exact pinned revision. Advanced users can set
`DOROBOT_JIT_RUNNER` to a matching `relax` executable. These settings stay local.
The launcher compiles the runner with Cargo, then compiles the Studio with
Relax; it does not substitute a native Studio binary.

## Validation and limits

On Apple Silicon/macOS, the JIT check passes with zero application errors.
The Nano generator writes and reads back three clips with 17 joint channels.
A real Metal window renders the 27-link, 26-joint model and plays joint motion.
Native `cargo test --locked -p dorobot-flex --no-default-features` passes 17 tests.

Relax is beta. In this dependency graph it reports a dormant
`num_rational::BigRational` import diagnostic and background compilation
diagnostics in `num-bigint` and `lexical-parse-integer`. The verified Nano
generation, loading and playback paths continue to work. FFmpeg decoding,
physical capture and other platforms have not been validated under JIT in
this change. Native Cargo remains available for those workflows.

Generated clips, local runtime state and recordings are ignored by Git. The
bundled Nano model and its provenance remain under `data/adora_nano`.
