# Studio refinement review

**95/100 — PASS for the tested desktop refinement scope.** The same model that
implemented the changes reviewed the original screenshots and real-input
results. This is a model-authored judgment, not an independent human score or
an objective measurement. Implementation: `3d72c53`, based on merged `feabe37`.

This review uses the separate [refinement rubric](../../tools/ux/polish-rubric.json),
whose weights and 95-point threshold were fixed before implementation. It does
not replace the earlier [91-point functional review](functional-review.md),
and does not regrade Nexus Studio.

## Judge → repair → retest

| Round | Score | Finding and resulting repair |
| --- | ---: | --- |
| Baseline | 82 | [Replay](polish-evidence/iterations/baseline-replay.png) had mismatched panel edges, weak plot labels and repeated empty-camera text. [Library](polish-evidence/iterations/baseline-library.png) spent too much space on thumbnails and wrapped actions. |
| 1 | 89 | [Recording](polish-evidence/iterations/round-1-record.png) exposed insufficient card insets and clipped session content. Increased corner clearance, removed the duplicate header filler and drew the playhead over its track. |
| 2 | 92 | [Compact recording](polish-evidence/iterations/round-2-record.png) still clipped camera guidance and session cues. Made session height intrinsic, shortened unavailable-camera text and explained missing take thumbnails. Widened calibration range readouts. |
| 3 | 94 | Real layout changes left the arrangement caption and selected buttons stale. Toolbar state now follows the active layout, including changes restored after appearance updates. |
| Final | **95** | All gates below pass. The final pass also corrected dark-mode Stop text and verified compact scrolling, toolbar feedback and the 44px scrub target. |

The [round record](polish-evidence/rounds.json) includes dimension scores and
observations. These are evaluative judgments from the same model, not separate
independent judges.

## Scorecard

| Dimension | Maximum | Final | Remaining deduction |
| --- | ---: | ---: | --- |
| Geometry | 20 | 19 | Automatic 3D framing can crop a tall simulated arm pose; manual orbit/zoom remains available. Very small custom splits reduce preview usefulness. |
| Typography | 15 | 14.5 | Dense technical metadata remains compact; recent-session dates have limited width in the narrow layout. |
| Color | 15 | 14.5 | Plot series still depend heavily on hue distinctions. The measured text pairs do not constitute a complete accessibility audit. |
| Components | 15 | 14.5 | Legacy plot rendering is less refined than the shared panel and toolbar treatment. |
| States | 15 | 14.5 | Empty states explain what is missing, but camera setup still requires navigation to Hardware. |
| Density | 10 | 9 | Default layouts retain absent-camera panes; long libraries and calibration content require scrolling. |
| Interaction | 10 | 9 | Arrangement tools require opening Panels; native window position is not persisted. |
| **Total** | **100** | **95** | |

## Acceptance evidence

- **40 original matrix images inspected:** Library, Record, Replay, Hardware
  and Evaluate × 1440×900 and 1024×768 × light/dark × a disposable Nano file
  dataset and explicitly labeled simulated fixtures. The `recorded` directory
  contains synthetic file-backed preview data, not physical robot recordings.
- **75 passing UI assertions** in [checks.json](polish-evidence/checks.json):
  24 workflow, 7 shared-speed/curation, 6 layout recovery, 5 restart,
  7 simulated recording/evaluation, 13 file refinement and 13 simulated
  refinement/compact checks. Tests use real remote pointer/keyboard input and
  read-only app observations.
- Flexible panels retain drag/reorder, both resize axes, maximize/Escape,
  close/reopen and recovery after closing all views. The
  [extra native Replay window](polish-evidence/workflow/native-replay-window.png)
  shares transport and frame position while keeping its own layout.
  [Restart restores the session](polish-evidence/restart/restored-layout.png),
  including speed, curation and paused position.
- [Selected panel controls](polish-evidence/refinement/selected-panel-controls.png)
  reflect the active layout. Original hover, pressed and focus captures in
  both appearances are retained in `polish-evidence/refinement`.
- Compact scrolling reaches [both calibration actions](polish-evidence/compact/compact-calibration-actions.png),
  [the last evaluation trace](polish-evidence/compact/compact-evaluation-last-trace.png)
  and [the final library cards and recent sessions](polish-evidence/compact/compact-library-bottom.png).
  The two latter results were visually checked; they are not counted as extra
  automated assertions. Controls below a scroll viewport are accepted only
  when scrolling reveals their full target.
- [11 authored text/surface pairs](polish-evidence/contrast.json) meet 4.5:1,
  including the previously weak plot-axis text. This measures the named color
  tokens, not every rendered pixel or every interaction state.
- Rust library tests: **20 passed**. Native
  `cargo check -p dorobot-flex --no-default-features --bin dorobot-flex` passes.
  The pinned Makepad JIT check reports **0 errors in checked application crates**.
  Tested application logs contain no UI shader error or panic.
- [67 original PNGs and their SHA-256 hashes](polish-evidence/manifest.json)
  include the matrix, interaction states and rejected iteration examples.
  No screenshot pixels were resized, retouched or otherwise edited for acceptance.

## Provenance and limits

Matrix and workflow captures use the final control/layout implementation.
Evaluate captures and its Stop journey were refreshed after the final Stop-label
color correction; refinement and compact captures were refreshed afterward.
The later Evaluate file captures retain the custom navigation width and episode
from the persistence journey. These differences are real retained application state.

Failed hot-reload runs were discarded and acceptance ran in fresh processes.
The test driver was corrected to scroll the inspector before clicking curation
controls below the larger timeline. A suspected missing header in image previews
was cross-checked against original PNGs and OCR; the original status text was
present. The speculative rendering change was reverted and is not credited as
a product repair.

Validation ran locally on Apple M5 Max with macOS 26.6.2 and the repository's
pinned Makepad JIT. Physical robot motion/calibration, authenticated Hub transfers,
a connected policy runner, FFmpeg playback, iPhone deployment and the Mac-mini
fleet were not revalidated. Native picker and capture-worker results in the
earlier functional report were not rerun or counted here. Pre-existing Relax
diagnostics in dormant numeric dependencies remain outside this acceptance.

Published evidence contains fixtures and synthetic data only. Raw runtime
reports, local preferences, machine paths, credentials and private CAD/recordings
are excluded. The changed source and evidence text passed a redacted secret scan.

## Reproduce

Use a disposable dataset and isolated `DOROBOT_STUDIO_STATE_DIR`; write
`DOROBOT_UX_REPORT` outside Git. Run one input driver at a time per app process.
`capture_matrix.py --app studio` records the five pages in both sizes and themes.
Run the refinement driver before workflow tests alter the fixture:

```sh
python3 tools/ux/polish_journeys.py --pid "$test_pid" \
  --remote "$makepad_remote" --report "$audit_dir/probe.json" \
  --out "$audit_dir/refinement"
# Use --demo with a separate fresh --demo app for configured-state checks.
```

The existing `functional_journeys.py` modes cover workflow, speed, simulated
recording/evaluation, restart and layout recovery. Run restart against the
expected state saved by the speed journey before subsequent layout changes.
