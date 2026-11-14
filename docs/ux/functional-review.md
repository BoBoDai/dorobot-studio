# Studio functional UX judge loop

**Result: 91/100, with the functional gates below passed for the tested desktop
software flows.** This is a model-authored review, not an independent human grade
or physical robot certification. Implementation: `6a5cc6b` (on top of `fbdc049`).

The earlier 91.5 visual score is superseded. It missed the removal of flexible
Replay panels and cannot establish application acceptance. This review retains
the original [rubric and weights](../../tools/ux/ios-rubric.json), adds mandatory
functional parity gates, and includes configured-state journeys.

## Judge → fix → retest

Intermediate scores below are retrospective judgments of the retained iteration
evidence using the same rubric. A failed functional gate rejects a round
regardless of its visual score.

| Round | Score | Judgment and repair |
| --- | ---: | --- |
| Baseline `1ab64cb` | Rejected | The fixed Replay layout removed the draggable grid and tracking-error panel. The earlier 91.5 is withdrawn as app acceptance. |
| [Round 1](functional-evidence/iterations/round-1.png) | 76.5 | Four stable panels returned. Content now moves with its title. Extra windows, configured workflows and persistence still needed evidence. |
| [Configured-flow review](functional-evidence/iterations/round-3-record.png) | 84 | Found an unbound Evaluate Stop action, Space activating both a focused button and playback, active-looking disabled recording buttons, clipped calibration/record text and an unreachable final evaluation trace. Rejected; fixed and retested. |
| Final | **91** | Real pointer, keyboard and native-picker journeys pass. Independent Replay windows, panel recovery and restart persistence are exercised. Remaining deductions are listed below. |

The final function inventory also exposed a speed setting without a visible control. Both Replay windows now expose 0.25×/0.5×/1×/2×, share the selected speed, and restore it after restart. The actual 2× rate, good-tag navigation, and upload-form cancellation were then tested; affected Replay captures were refreshed.

The loop also caught test-harness defects: controls with duplicate names across
windows needed scoped observations; repeated tests needed to normalize existing
tags and pane visibility; native dialog tests needed visible windows and the
remote interface's user-idle interval. Those failed runs were not counted as
passes. The final native compile additionally required qualifying `::log::warn!`;
that name-only correction does not change the captured UI or its behavior.

## Fixed functional gates

| Gate | Observed result |
| --- | --- |
| Flexible Replay panels | Real drags move 3D/plot content with titles. Both divider axes resize. Maximize/Escape, close/reopen, all-panels-closed recovery, three arrangements and reset pass. Tracking error is restored. |
| Additional native window | Shares episode/transport/frame position; changing its layout leaves the main layout intact. Close/reopen preserves its own layout and the main task. |
| Persistence | A fresh process restores custom panel order/visibility/sizes, navigation width, episode, tag and paused playhead. It does not start playback or capture. |
| Library and cancellation | Dataset selection/navigation and Hub form cancellation preserve the task. Real macOS folder and rig-profile picker cancellation pass in a visible test app. |
| Replay and curation | Selection, playback from the extra window, pause, pointer seek, frame stepping, four shared playback speeds with an actual 2× rate check, good/bad tags, exclusion/Undo, and all six existing signal lanes are exercised on a disposable file dataset. |
| Hardware and Record | Setup guidance sits alongside the real controls. No-rig Record/Save are disabled. Explicit simulated flows cover Record → Stop → Save, Discard and Re-record. Compact calibration scroll reaches its actions; incomplete calibration keeps Continue disabled. |
| Evaluate | Stop is connected to its backend intent and becomes disabled after stopping. An absent policy runner is explicit. The final divergence trace is reachable by scrolling. Generated evaluation data is confined to labeled demo mode. |
| Fit and input | All five screens captured at 1440×900 and 1024×768, light and dark, with file-backed and simulated configured states. Space respects focused controls. Curation and calibration retain scroll access. |

## Scorecard

| Dimension | Weight | Round 1 | Configured review | Final | Final deduction |
| --- | ---: | ---: | ---: | ---: | --- |
| Structure | 20 | 18 | 18 | 18 | Four-view defaults can spend space on absent cameras; the user can close/maximize views. |
| Controls | 15 | 10 | 11.5 | 14 | Panel arrangement controls could communicate the current mode more explicitly. |
| Typography | 15 | 12 | 13 | 13.5 | Dense signal/calibration metadata remains smaller than primary task text. |
| Color | 10 | 8 | 8 | 8.5 | Legacy plot-axis text fails the 4.5:1 target; see the measured token pairs below. |
| Clarity | 15 | 10 | 12.5 | 14 | Some technical dataset terminology remains; missing hardware/media and demo data are explicit. |
| Fit | 10 | 7 | 8 | 9 | Narrow layouts require scrolling or panel maximize; very small custom splits reduce preview usefulness. |
| Continuity | 10 | 7 | 8.5 | 9.5 | Panel and task state recover; native window position is not persisted by this change. |
| Consistency | 5 | 4.5 | 4.5 | 4.5 | Legacy plot chrome differs from the newer buttons and panels. |
| **Total** | **100** | **76.5** | **84** | **91** | |

## Evidence and validation

- [52 passing UI assertions](functional-evidence/checks.json): 24 file/workflow,
  6 layout recovery, 7 simulated configured workflows, 5 restart, 2 native picker,
  1 compact calibration reachability, and 7 speed/curation checks.
- [Measured contrast pairs](functional-evidence/contrast.json): primary/body/warning pairs pass 4.5:1; legacy plot-axis text does not and is deducted.
- [Original-pixel manifest with SHA-256 hashes](functional-evidence/manifest.json),
  including the 40 final matrix captures and the interaction/iteration captures.
- [Shared speed control](functional-evidence/speed/shared-speed-control.png).
- [Independent native Replay window](functional-evidence/workflow/native-replay-window.png),
  [custom layout](functional-evidence/workflow/custom-layout.png),
  [restored session](functional-evidence/restart/restored-layout.png),
  [all panels closed](functional-evidence/layout/all-panels-closed.png),
  [last signal](functional-evidence/layout/compact-last-signal.png).
- [Simulated take review](functional-evidence/simulated-workflow/simulated-take-review.png),
  [compact calibration actions](functional-evidence/compact/compact-calibration-actions.png),
  [last evaluation trace](functional-evidence/compact/compact-evaluation-last-trace.png).
- Rust library tests: **20 passed**. Capture worker tests: **5 passed**, including
  actual encoded video/frame statistics using mocked robot devices.
- Makepad JIT check: **0 errors in checked application crates**. Native
  `cargo check -p dorobot-flex --no-default-features --bin dorobot-flex` passes.

This correction was tested locally on Apple M5 Max, macOS 26.6.2. The older
report's Mac-mini results apply to its older implementation; these new changes
have not been revalidated on that fleet. No credentials, private recordings,
source CAD files or runtime preference files are included in the evidence.

## Reproduce

Use a disposable dataset copy and an isolated preferences directory. Keep output
outside Git. Launch with `DOROBOT_UX_REPORT` and `DOROBOT_STUDIO_STATE_DIR`; use
`MAKEPAD_HIDE_WINDOWS=1` for the matrix and general journeys. Native picker tests
need a visible dedicated app and existing macOS Accessibility access.

```sh
python3 tools/ux/functional_journeys.py --pid "$test_pid" \
  --remote "$makepad_remote" --report "$audit_dir/probe.json" \
  --out "$audit_dir/journeys"
# On the same isolated instance, exercise additional layout recovery:
python3 tools/ux/functional_journeys.py --mode layout --pid "$test_pid" \
  --remote "$makepad_remote" --report "$audit_dir/probe.json" \
  --out "$audit_dir/layout-recovery"
```

The `speed` mode exercises the shared speed control, measured playback rate, good tagging and upload cancellation.

The `demo` mode expects a fresh `--demo` app and uses the same arguments. The
`restart` mode compares a fresh process using the same preferences directory
against `expected-restart.json` written by the replay journey; run it before
later layout tests change that expected state. `native_dialog_journey.py` accepts
`--pid`, `--remote`, `--report` and a JSON file path for `--out`. The existing
`capture_matrix.py --app studio` captures all five pages, sizes and appearances.

Physical arm motion/calibration/torque shutdown, authenticated Hub transfers,
a connected policy runner, FFmpeg playback and iPhone deployment are outside
this acceptance. The OpenCV encoder test does not certify a physical capture.
Relax still emits pre-existing background diagnostics in dormant numeric
packages; passing Nano workflows does not prove all JIT dependency paths.
