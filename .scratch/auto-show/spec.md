# Auto-show

A mode in which niri's focus follows the overlay's selection. While it's on,
moving the selection focuses the selected window — or the workspace, if it holds
none — and hands the keyboard grab straight back to the overlay's own output. It
is what pressing `Enter` does, run automatically on every navigation step.

The name is `auto-show` everywhere: the key legend, `State::auto_show`,
`CONTEXT.md` and the docs.

## Why it exists

With the overlay open on one screen and its map showing another
(`--open-on-monitor eDP-1 --solo HDMI-A-1`), browsing the map with `j`/`k` gives
me a live preview of each workspace on the other screen, without a keypress per
look.

## The mode

- Bound to `f`, a bare-letter toggle like `s` for solo.
- Turning it on **arms** the mode without focusing anything, and seeds the guard
  with niri's current focus. The invariant is therefore *keep niri's focus equal
  to the selection*, so navigating away and back costs no niri calls. The seed is
  re-taken on every enable.
- Turning it off cancels anything waiting to be sent.
- Session-only: off at launch, with no config key and no command-line flag.
- `Enter` stays the manual one-shot equivalent, and records what it focused so the
  guard keeps matching niri.

## What triggers a focus

Navigation, and nothing else:

- `j`/`k`, `h`/`l`, `1`–`9`, `<`/`>`, `Tab`/`Shift+Tab` (including the solo swap).
- A pointer click that changed the selection. `hit_select()` runs on press, while
  `drag` is already armed, so the trigger sits in `drag_end` and fires only when
  no drag happened. A real drag focuses nothing; a second click on the already
  selected item keeps its existing `activate_selection` path.

Selection changes that come from a refresh never focus anything: not the clamp
after `x`/`w` kills a window, not the snap to niri's focus when the overlay
regains the keyboard. The rule stays one sentence — *navigation focuses, nothing
else does* — and niri's own focus-after-close behaviour is what I want anyway.

## What gets focused

Whatever the map highlights: the selected window, which after a `j`/`k` step is
the workspace's first one (`move_ws` resets `sel_win` to 0). A workspace holding
no windows falls back to `focus-workspace`. So the focused thing and the
accent-bordered thing can never disagree. It fires whichever output the target
sits on — on the overlay's own screen the change happens behind the overlay,
which keeps niri's focus consistent with the map for when I dismiss it.

## Rate and edge cases

- A ~120 ms debounce, one pending target, latest wins. Holding `j` through ten
  workspaces costs one focus call instead of ten.
- A pending target equal to the last sent one is dropped, sending nothing.
- A target that died before the timer fires: the error is swallowed and the
  target is recorded as sent, so there's no retry loop at a dead window and no
  error banner over a map I'm browsing fast.

## Cost

`preview_selection()` focuses the target and hands the grab back, with no
synchronous refresh — the focus change makes niri emit an event, and the event
stream refreshes from that. `activate_selection` is left alone for `Enter`, where
the immediate refresh matters.

Move actions (`Shift+J`/`K`, `Shift+H`/`L`, `Ctrl+H`/`L` and pointer drops) end
by focusing the overlay's own output while the mode is on, so a move that carries
a workspace across screens can't strand the keyboard grab.

## Where focus ends up

On the last previewed target, both when the mode is toggled off and when the
overlay quits. Nothing is restored: the mode *is* switching windows, just eagerly.

## On screen

- An accent `auto-show` pill in the bottom-right, beside the `? keys` hint. It
  shows even when the overlay is unfocused — the mode will move focus the moment
  a key is pressed, so a background map has to admit it's armed.
- `f auto-show` under General in the key legend.

## Code

`src/autoshow.rs` holds the pure state machine — the last sent target, the
pending one, and the "what should I send now?" predicate — with unit tests.
`main.rs` keeps the debounce timer and the two `niri` calls.
