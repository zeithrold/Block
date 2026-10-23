# Toast and Drawer component increment

Date: 2026-10-23. This is a Milestone III implementation checkpoint, not user acceptance.

## Scope and upstream choices

Keep the restrained Home composition and existing page responsibilities. Replace
the single painted Toast with a shared window layer, and give the three existing
Home drawers working drag, bounded layout and retained exit transitions. No
catalog, authentication, recovery, download or window-chrome policy changes are
part of this increment.

The locked, published dependencies remain unchanged: `gpui-base` 0.8.0,
`gpui-component` 0.8.0 and `gpui-pre` 0.3.9. The implementation uses Base Root
plugins, `ToastManager`, `ToastStack`, Base Toast, Base Sheet, Presence and Spring.
The library owns focus trapping, Escape, backdrop close ordering, stack geometry
and motion primitives. Block supplies palette, content, gesture policy and
lifetime coordination; it does not fork or patch the dependencies.

The prototype's installed implementation is the reference: Sonner 2.0.8 through
the Shadcn wrapper for Toast, and Base UI Drawer 1.9 through its Shadcn wrapper
for Drawer. The stock GPUI Notification/Sheet APIs cover the basic use cases,
but their public presentation does not provide all of the needed swipe,
coalescing and retained-exit behavior. This increment adapts the unstyled
foundation rather than creating a separate modal/focus system.

## Toast behavior

- Success, information and error messages share one bottom-center stack.
  Ordinary messages last four seconds; errors last eight seconds. Identical
  kind/message pairs extend the existing deadline without replaying entrance.
- At most five active messages remain, with three collapsed layers. Hovering or
  focusing the stack expands it and pauses expiry; an active swipe also pauses
  expiry. Removed messages finish their exit before their storage is released.
- The close button uses Base Button's keyboard and accessibility behavior.
  Horizontal or downward swipes follow the pointer. Releasing below the
  dismissal threshold animates it back; an accepted swipe continues outward and
  fades. Pointer tracking continues outside the original card within the window.
- Width is capped at 356 logical pixels and fits short/narrow windows. The
  product reserves its existing Play/Save/Retry action rail so a transient
  message cannot cover those buttons. This is an intentional adaptation of the
  prototype's global bottom offset to Native's fixed action footers.
- Current palette and reduced-motion policy apply to the stack and card.
  Changing reduced motion settles existing transitions without replaying the
  entry. The lifecycle timer stops when the stack becomes empty.
- Form errors remain on their forms. This component work does not implement the
  three-level recovery policy or introduce new automatic repairs.

Product messages are queued until the window's presentation pass, preserving
several messages produced in one engine poll. The previous last-message shell
state remains for existing status consumers; it no longer paints a second Toast.

## Toast timing alignment

The installed Sonner 2.0.8 `dist/styles.css` and `dist/index.mjs` are the source
for this follow-up. The Shadcn wrapper and application CSS do not override these
motion rules.

| Path | Timing and travel |
| --- | --- |
| Entrance | 400 ms CSS `ease` (`cubic-bezier(.25, .1, .25, 1)`), from one measured card height below its resting position, with the same opacity curve. |
| Stack arrival, expansion and collapse | 400 ms CSS `ease` for the final measured bottom edge, width, layer visibility and content fade. Interrupted transitions continue from their sampled values; reversals use the upstream CSS transition shortening behavior. |
| Rejected swipe | Track the pointer directly, then return over 400 ms CSS `ease`. |
| Accepted swipe | Continue from the release offset by another card width or height, fading over 200 ms CSS `ease-out` (`cubic-bezier(0, 0, .58, 1)`). |
| Ordinary removal | Front/expanded cards request 400 ms movement and opacity. Collapsed back cards request 500 ms movement to 40% of their height and a 200 ms fade. Sonner removes the node after 200 ms in all these cases; Native retains that actual removal timing. |
| Reduced motion | Settle all presentation channels immediately under either the application preference or the OS preference. Disabling the preference does not replay entrance. |

Base's published `ToastStack` API offers spring response tokens rather than a
CSS transition policy. Block disables only its presentation springs and retains
its measurement, placement, hover and focus handling. A small prepaint adapter
transitions the final measured bottom edge, so stack height and item offset do
not compose two different trajectories. The same native prepaint offset moves
the painted content, hitboxes and accessibility bounds. Width and content fade
use the public Base transition primitives. No dependency patches are involved.

The gesture axis locks after one logical pixel of travel. Crossing the other
axis later does not change the card's direction or its dismissal test. Grabbing
a returning card preserves its painted offset, so a new drag does not snap it
back to the resting position. Upward travel uses Sonner's resistance formula.

This aligns timing rather than implementing a browser renderer. Base still owns
variable-height stack geometry and uses a width reduction for collapsed layers;
the web implementation scales the entire card and adopts the front card's
height. Native also retains its horizontal/downward displacement thresholds,
four/eight-second lifetimes and action-rail offset. These intentional product
adaptations are separate from the curve regression checks and desktop review.

## Drawer behavior

- Account, game/profile and download drawers use the existing single window.
  Dialogs remain above the drawer; feedback remains above both.
- A full-panel Presence transition retains the real content during close.
  Desired selector/download heights are capped at 80% of the client area;
  selector results shrink and scroll while the management action remains visible.
- Only the handle initiates a drag. List scrolling, searching and row activation
  remain independent. Downward movement follows the pointer; a short release
  rebounds. A pull past 30% of the panel height or a recent downward fling closes
  it. Fling detection accumulates small pointer samples and rejects stale motion
  after a pause or upward reversal.
- Escape, backdrop and the keyboard-accessible Close button share the logical
  dismissal path. Focus stays modal through exit and returns after the exit
  finishes. Revision checks protect a rapidly reopened drawer from stale closes;
  its original return focus survives interruption and a dialog opened during exit.
- Client-side decoration insets follow the library's border geometry, including
  tiled Linux edges. The scrim uses the prototype's light tint; native backdrop
  blur and nested/snap-point drawers are not implemented in this increment.

## Verification and remaining To-Do

The component regressions exercise physical pointer/key events, retained exit,
focus restoration, stacked dialogs, rapid reopening, short-window layout,
expiry/coalescing, hover/focus pause, swipe dismissal and runtime reduced motion.
They complement the existing selector, overlay and retry-action regressions.
Toast timing regressions sample actual native card bounds at intermediate
timestamps against browser CSS reference values, covering full-height entrance,
stack retargeting, interrupted expansion/collapse, swipe return, outward swipe,
ordinary/back-layer removal, diagonal axis locking, re-grabbing during return
and runtime system reduced motion.
The local macOS native gate passed formatting, all-target/all-feature checking,
strict Clippy, 554 tests across 39 suites (zero failures or ignored tests), and
the desktop executable build. Motion fixtures select both normal and system-reduced
policies within each test thread; they do not change the host OS preference.
This is automated native evidence; the disposable
CLI preview could not be bound reliably by the desktop automation, so the new
components' actual desktop gesture/visual verdict remains pending.
Run the complete native gate with
`python3 scripts/verify-platform.py --platform <platform>`; the automatic CI
matrix applies it to all five native targets.

- [ ] Review the actual visual result and interrupted gestures on Windows,
  macOS, Linux X11 and Wayland/Xwayland. Automated GPUI tests and native builds
  do not constitute desktop or visual acceptance.
- [ ] Audit macOS system Reduce Motion and native transition frames as part of
  the existing Milestone III motion review.
- [ ] Review long localized Toast copy and large expanded stacks in short
  windows before expanding message content or adding task actions.
- [ ] Integrate task-specific coalescing, persistent failure actions and export
  with the separately planned recovery policy; matching text alone is not a
  durable task identifier.
- [ ] Evaluate nested drawers, snap points or native backdrop blur only when a
  concrete product flow requires them. Existing download details remain a Dialog.

Accepted remains Pending.
