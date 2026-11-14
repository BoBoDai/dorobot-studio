# PingFang and button refinement

The main verification below covers `163ae01`; the later panel utility
adjustment has its own targeted verification at the end.

Studio now uses the installed PingFang SC face for interface text: Regular
(400) and Semibold (600). The theme installs these faces before controls inherit
their text styles and retains them across appearance changes. Inter remains the
fallback on platforms without PingFang; no Apple font files are bundled.

Action buttons now use 6px rounded rectangles. Standard button padding is
14px horizontally and 8px vertically, retaining the existing minimum 44px
height and 48px recording controls. Playback, toolbar, library, calibration
and curation buttons share this treatment. Navigation, status badges and cards
retain their existing geometry.

PingFang exposed a small vertical overflow in the compact camera placeholder.
Removing redundant vertical label margins fixes it without relaxing the
containment assertions.

## Verification

- Native application check passes; the pinned Makepad JIT check reports zero
  errors in checked application crates. Runtime logs contain no shader error
  or panic.
- **57 real-input UI assertions pass** in [checks.json](typography-controls-evidence/checks.json):
  13 file refinement, 13 simulated refinement, 24 workflow and 7 simulated
  recording/evaluation checks. Multiwindow transport, independent layouts,
  drag, resize, maximize, close/reopen and retained layout all pass.
- Captured 40 screen/size/appearance/backend combinations: five pages,
  1440×900 and 1024×768, light/dark, synthetic Nano file data and simulated
  fixtures. Inspected representative captures from both appearances and
  sizes, including the compact recording layout and native Replay window.
- Runtime font resolution loaded PingFang TTC faces 3 and 11.
  [Font metadata and character coverage](typography-controls-evidence/font-verification.json)
  identify these as PingFangSC-Regular and PingFangSC-Semibold. The
  [mixed Chinese/Latin input](typography-controls-evidence/mixed-text-dark.png)
  also renders in the running application; the transfer form was cancelled.
- [Eight original screenshots](typography-controls-evidence/manifest.json)
  are retained with dimensions and SHA-256 hashes. Pixels were not edited.
  Raw runtime reports, machine paths, preferences and private data are excluded.

This is a presentation follow-up to the [95-point refinement review](polish-review.md),
not a new score or independent judge. That report's 75 assertions and 20 Rust
tests describe its earlier implementation; the 57 checks above were rerun for
this change. Physical hardware, authenticated transfers, other platforms and
Nexus Studio were not revalidated.

## Compact panel utilities

The panel maximize and close buttons now measure 28×28 logical pixels, with
zero padding and an explicit 28px minimum height overriding the iOS theme's
44px minimum. Their shared title row is 40px high with 4px spacing. Other action
buttons retain their existing sizes.

Applied through JIT live reload in the existing process, without a native
build or application restart. Sixteen targeted checks passed: retained layout
and paused episode/frame, all eight Replay utility button dimensions,
maximize/Escape, close/reopen, appearance switching and restoration of the
original layout. Original light/dark captures were visually inspected.
The earlier 57-check suite was not rerun for this localized size adjustment.
