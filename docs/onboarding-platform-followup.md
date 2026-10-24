# Onboarding exit-point and platform follow-up

Date: 2026-10-24 (Asia/Taipei). User review and visual acceptance remain pending.

The welcome button now shrinks the visible inverted fill toward the point where
the pointer crosses out of the original circle. The crossing is computed from
the last pointer segment, including off-axis exits. A partially painted mask is
retained when direction changes so retargeting cannot teleport an existing fill.
The hit area follows the circle rather than its square bounds.

The 160 px button, solid icon, bold Welcome Back heading and bottom Skip setup
are retained. The central group moves up about 27 px: its button anchor changes
from 56% to 52% of the content height, placing the center at (180, 401) in a
360 × 720 window.

## Source and scope

Final test snapshot is **Round 2**, based on `f1174710448618d13ce8836813e5224ae2557c56`
plus the authorized uncommitted changes. Canonical POSIX-path Rust fingerprint:
`98fcb4910063d7b5b60a81aa6d4cbbcba74b1f9dc31299c72911d0fb947b85ca`.
The archive SHA-256 is
`a8205b79dc47bc8d24c4cacfbe2c3948aff223e6ceee5200406b2a7a97bee8d3`.
All 1,310 files are enumerated and hashed. `.env`, credentials, Git metadata,
build caches and player data are excluded. Remote source and binary identity
are recorded independently; no commit or push is implied.

Round 1 is retained as investigation evidence. Its native gates passed on both
Windows targets, but a Mac compositor-frame inspection exposed intermittent
empty masks from nearly duplicated circle-arc endpoints and floating-point
cancellation. It is **not** the final visual result. Round 2 uses double-precision
arc intersections, unique join endpoints, local signed areas and short-edge
tolerance, with a regression requiring monotonically shrinking fill area.

Validation covers native formatting, check, strict Clippy, workspace tests and
desktop builds; Welcome light/dark, exit direction and reversal, Drawer source
and selection navigation, Wizard cancellation/step navigation, secondary sign-in
options and storage entry. Synthetic scenarios and filesystem fixtures establish
these results without live Microsoft sign-in, copying real player games or
launching Minecraft.

## Final results

| Platform | Final native gate | Final desktop interaction | Evidence directory |
| --- | --- | --- | --- |
| macOS ARM64 reference | PASS: 631 passed, 0 failed/ignored; full local gate | Welcome exit directions, reentry and circular hit area pass; 302 compositor frames; 18 static theme/scenario captures | `mac/round2/` |
| Windows ARM64 | PASS: 629 passed, 0 failed/ignored | Welcome motion, Drawer selection retention, Wizard cancellation/draft retention and storage entry pass within synthetic scope | `windows-arm64/round2/` |
| Windows AMD64 | PASS: 629 passed, 0 failed/ignored | Welcome motion, source/list selection, Wizard cancel/Continue and sign-in disclosure pass; backend action limits below | `windows-amd64/round2/` |
| Linux ARM64 | PASS: 629 passed, 0 failed/ignored | Welcome directions/reentry, selection retention, Wizard navigation and isolated storage save/reopen pass | `linux-arm64/` |
| Linux AMD64 | PASS on complete retry: 629 passed, 0 failed/ignored | Welcome motion/physical geometry, selection retention, Wizard flows, isolated Skip/restart, storage save and synthetic batch copy pass | `linux-amd64/round2/` |

Directories are relative to ignored `target/onboarding-validation/platform-followup/`.
The Windows AMD64 and Linux AMD64 work is strictly serial because they share a
physical host. ARM64 VMs run independently. A temporary Mac activity assertion
kept the reference desktop awake during the checks and was stopped afterward;
the normal desktop preview is retained. Linux ARM64 used a task-owned temporary
swap extension. Linux AMD64 required build-cache recovery as described below.
Cleanup and limitations are recorded with the platform results.
The Linux ARM64 temporary swapfile has been safely disabled and removed; its
pre-existing swap partition and user desktop session remain intact.
The first Linux AMD64 gate stopped during `gpui-component` archive creation
because the VM filesystem filled. This is retained as a failed environment
attempt. Once Cargo, rustc and the linker had exited, three audited caches in
the older default-host `target/debug/` tree were authorized for removal:
`deps`, `build` and `.fingerprint`. The existing executables, current explicit
target tree, source, Cargo registry and all evidence are preserved. The final
gate passed a complete retry after this build-cache recovery; the final native
executable identifies itself as `linux/x86_64`, SHA-256
`df15cc561662687395f484b3a252ddbdf5773440af5b3ec126f3e566b93a8ec0`.
The first cleanup approval was rejected after an earlier parent-assistant
preserve-cache constraint was misattributed to the human user. The root verified
that attribution and submitted the same explicit action for approval with a new
read-only directory audit. Approval then passed. Safe removal freed the audited
caches, increasing available space from 58,109,952 to 19,123,351,552 bytes; the
preserved binaries' inode, size and modification time are unchanged. Both audit
and execution receipts are retained as `linux-amd64/round2/root-cache-*.json`.

The Mac exit replay observes right exit origin (160, 80), left exit (0, 80), and
off-axis exit (132.915, 140) in button-local coordinates. Each direction has five
nonterminal compositor samples; the inverted pixel area decreases without a
blank frame followed by regrowth. Right/left residual fills remain on their
corresponding exit side. Raw paint traces, window frames, pixel analysis and a
window-only MP4 are separate artifacts. The older Mac helper's component-sorted
fingerprint and the canonical POSIX-string-sorted fingerprint are explicitly
distinguished and verified against identical source members and bytes.
The 18 final static captures use the same native binary as the motion replay.
Seventeen pass automated label OCR; the light Welcome image required direct
visual review because OCR missed two correctly rendered labels. The original
OCR failure and the separate visual-review receipt are both retained.

Linux ARM64 captures distinguish compositor-authorized Spectacle full-desktop
raw stills from XGetImage Xwayland app-XID raster videos. The helper places its
402 × 704 XID image on a black-padded 1280 × 800 output canvas; it does not read
the X root window. A separate Welcome review crop contains only Block's visible
surface. The raster video establishes captured button pixels, not compositor
presentation timing. In a 155 × 155 rectangular
button region, both directional exit tails decrease to the resting glyph
baseline. Rapid reversal bottoms at 1,688 selected pixels above the 612-pixel
resting baseline before growing again; no empty-state flash precedes regrowth.

On this 1280 × 800 VM, the observed Welcome XID is 402 × 704 and its visible
surface is about 360 × 664 including borders. This is consistent with the
existing `window_geometry.rs` display-fitting policy: reserve at least 32 px
above and 64 px below, then account for the Linux client-frame insets. Its
small-screen regression explicitly expects a 704 px fitted outer height. This
explanation is an inference from the owning source and recorded viewport, not
a separately measured window-manager work area. The 720 px content preference
is retained for unconstrained displays; no geometry source was changed.

Engine-less scenario fixtures exercise navigation and visual state, not saved
backend actions. In the Windows AMD64 fixture, Skip reports an unavailable data
directory and stays on Welcome; that action is unverified by that fixture.
Independent save/restart and copy filtering pass native filesystem tests. Linux
ARM64 additionally verifies storage Save and reopening in the normal application
with a completely isolated synthetic home/data directory. Real provider login,
import of real player files, installation and game launch are outside this replay.

Linux AMD64's final directional replay uses X11 root-raster capture cropped to
Block's visible content; direct-XID capture had GPU striping and is not the
review source. The controller's earlier geometry offset is retained as a failed
calibration. A baseline glyph centroid puts the physical circle center near
(600.17, 421.5); the actual pointer path uses screen y=442. Its theoretical local
exit points are (2.67, 100.5) and (157.33, 100.5), matching observed (2.54, 100.01)
and (157.46, 100.01) within one pixel. This is a physical-circle comparison,
separate from merely observing a left or right trace label. A standalone clip
checks directional shrink; the earlier manual clip checks rapid reversal.

Linux AMD64 also launches the normal debug application without `--scenario` or
`--theme`, with isolated HOME/XDG/data directories. Clicking Skip reaches Home,
saves `onboarding_complete=true`, and restarting with the same isolated data
opens Home again. The debug CLI permits normal launches; only `--theme` requires
`--scenario`. The initial probe mistakenly retained `--theme` and is not a
product limitation. No release rebuild or source change was needed.

The Linux AMD64 normal-engine run also discovers one adjacent synthetic game and
three synthetic launcher games, then copies all three launcher games through
the real UI. The result reports 3/3 Imported and the source hashes remain
unchanged. These are separate from the four-game scenario fixture. The imported
Game task routes Continue into the still-unfinished Account Wizard. Secondary
sign-in disclosure, cancellation/reentry, Custom setup Review/Back with a typed
draft, and More options entry/cancel pass; a synthetic storage preference is
saved and verified in its isolated `storage.json`. No external sign-in starts.
In another empty isolated data directory, the three-game launcher list changes
from three selected to two; returning to Sources and reopening the list retains
the exact deselection and `Import 2 games`. This second list is not copied.

All task-owned remote app and capture processes have ended. Linux AMD64's VM
remains running because pre-existing user services are active; none of those
services was stopped. Current executables, source trees, registry caches and
local evidence are retained. The root independently recounts the five native
test logs and verifies that every frozen build input/resource still matches;
only validation documentation advanced after the snapshot. See the ignored
`final-root-verification.json` receipt beside the platform directories.
