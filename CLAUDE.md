# niri-groom

A fullscreen [layer-shell](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)
overlay for the [niri](https://github.com/YaLTeR/niri) Wayland compositor. I survey
all workspaces and windows as a proportional map — like niri's overview, but with the
workspace name and each window's title shown clearly — and let myself kill a whole
workspace or a single window from the keyboard with no confirmation.

## What it does

- Reads the live state via `niri msg --json workspaces` and `niri msg --json windows`.
- Places each output by its real horizontal position: `niri msg --json outputs`
  gives every output's `logical` rectangle (x/y/width/height); I scale `x`/width by
  the horizontal span so a screen on the left/right shows up there at its relative
  width. The axes are decoupled vertically: tops align to a common edge and every
  output is drawn full-height (a configured y-offset like `HDMI-A-1 position y=360`
  is intentionally *not* reproduced — it'd just waste vertical space). Falls back to
  a synthetic row if positions are missing.
- Draws each output's workspaces (stacked, labelled by name), and
  the windows inside each workspace laid out by their real scrolling-layout position
  (`layout.pos_in_scrolling_layout` → column, row).
- Hides unnamed empty workspaces. niri keeps a permanent trailing empty workspace
  per monitor (plus transient empties after moves); these are scratch space that
  can't be meaningfully killed, so showing them only confuses. Named empty
  workspaces are kept (you can still rename or kill them).
- Refreshes on niri's event stream (`niri msg --json event-stream`): a worker
  thread reads one JSON line per change and pings a `bounded(1)` channel that the
  GTK loop awaits, so the map re-fetches near-instantly on change instead of
  polling. The channel coalesces bursts (extra pings dropped while one is
  queued). A slow 2s timer remains as a fallback (e.g. for output-geometry
  changes niri may not emit an event for). The event-stream child is spawned
  with `PR_SET_PDEATHSIG` so it can't outlive the app.
- Kills windows with `niri msg action close-window --id <id>`. "Killing a workspace"
  means closing every window it holds (niri keeps named/empty workspaces around by
  design, so an empty workspace is a no-op).

### Keybindings

| Key            | Action                                   |
| -------------- | ---------------------------------------- |
| `1`–`9`        | Jump to the workspace with that niri index on the current output |
| `<` / `>`      | Jump to the first / last workspace of the current output |
| `j` / `Down`   | Select next workspace; crosses to the next screen at the boundary |
| `k` / `Up`     | Select previous workspace; crosses to the previous screen at the boundary |
| `Shift+J`      | Move the selected workspace down within its monitor |
| `Shift+K`      | Move the selected workspace up within its monitor |
| `Shift+H`      | Move the selected workspace to the screen on the left |
| `Shift+L`      | Move the selected workspace to the screen on the right |
| `l` / `Right`  | Select next window in the workspace       |
| `h` / `Left`   | Select previous window                   |
| `Ctrl+L`       | Move the selected window's column right within the workspace |
| `Ctrl+H`       | Move the selected window's column left within the workspace |
| `Tab` / `Shift+Tab` | Jump straight to the next / previous screen (output) |
| `s`            | Solo the selected monitor (toggle): show only it, full-width; `Tab` then swaps which one |
| `f`            | Auto-show (toggle): niri's focus follows the selection as I navigate |
| `Enter`        | Focus the selected window (or workspace if empty); the overlay stays open and keyboard-focused |
| `Shift+Enter`  | Same, but dismiss the overlay if the target is on the overlay's own monitor |
| `r`            | Rename the selected workspace (inline text field) |
| a `mark-kind` key | Toggle that kind of mark on the selected workspace (runs the kind's command; opens rename first if the workspace is unnamed). `m` and `p` in my config |
| `t`            | Open the theme picker (live preview; Enter saves, Esc cancels) |
| `?`            | Toggle the key legend panel (hidden by default; a small `? keys` hint shows) |
| `w`            | Kill the selected workspace (all windows) — no confirm |
| `x`            | Kill the selected window — no confirm     |
| `q` / `Esc`    | Quit                                     |

While the rename field is open the whole keyboard feeds the edit buffer.
`Enter` commits (`set-workspace-name`, or `unset-workspace-name` if left empty),
`Esc` / `C-g` cancels. The field is a small line editor (`Edit`) with
readline-style (Emacs) bindings: `C-a`/`C-e` start/end, `C-b`/`C-f` char,
`M-b`/`M-f` word, `C-d`/`Backspace` (`C-h`) delete, `C-k` kill-to-end,
`C-u` kill-to-start, `C-w`/`M-Backspace` kill-word-back, `M-d` kill-word-fwd,
plus arrows/Home/End/Delete. There's no separate manual-refresh key — the 800ms
timer keeps the map current.

`rename_workspace_by_id` renames **without moving focus**, by targeting the
workspace through `set-workspace-name`'s `--workspace` reference rather than
focusing it. A named workspace is referenced by its current name; an unnamed one
by its index — which niri resolves on the *focused* output, so I focus its
monitor first (and restore the previous one) only when it's on a different
output. niri's `set-workspace-name` is a **case-insensitive no-op** — setting
`foo` over `Foo` does nothing, so a case-only edit would silently fail. For a
named workspace I force the change through a throwaway intermediate name (a
zero-width-space prefix), referencing the workspace by name at each step. This
matters for the `m`-on-unnamed flow: naming the workspace must not switch focus
to it (which focusing-to-rename used to do).

`s` toggles **solo mode** (`State::solo: Option<usize>`): only the selected
output is shown, laid out full-width (`compute_layout` takes a `solo` arg and
`layout_output` places that one output across the whole content area). While
solo, `j`/`k` navigation is confined to that output and `Tab` swaps which output
is solo'd. Press `s` again to show all screens.

## Auto-show

`f` toggles **auto-show** (`State::auto_show`, the state machine in
`src/autoshow.rs`): while it's on, niri's focus follows the selection, so
navigating the map previews each window on its own screen. It's what `Enter` does,
run on every navigation step — mainly for the two-output setup where the overlay
sits on one output and maps another (`--open-on-monitor eDP-1 --solo HDMI-A-1`).

Turning it on **arms** it and focuses nothing: it seeds itself with niri's current
focus (`State::focused_target`), so the invariant is *niri's focus equals the
selection* and navigating away and back costs no niri calls. The seed is re-taken
on every enable, and turning it off drops anything waiting to be sent. The mode is
session-only — off at launch, with no config key and no flag.

Only navigation focuses anything: `j`/`k`, `h`/`l`, `1`–`9`, `<`/`>`, `Tab` (all
routed through `navigated()`), plus a pointer click that changed the selection.
Because `hit_select()` runs on press with `drag` already armed, that click case
lives in `drag_end` and fires only when no drag happened. Selection changes that
come out of a refresh never focus anything — the snap to niri's focus when the
overlay regains the keyboard stays silent — with one exception: a kill (`x`/`w`).
The handler records the killed target (`AutoShow::follow_kill`) and every refresh
goes through `sync_from_niri`, which waits until that target has left the map
(closing is asynchronous, so the dying item lingers for a moment) and then
previews whatever the selection fell back to. A manual navigation in the meantime
supersedes it.

The target is whatever the map highlights, so after a `j`/`k` step it's the
workspace's first window (`move_ws` resets `sel_win`); an empty workspace falls
back to `focus-workspace`. `preview_selection()` focuses it and hands the keyboard
grab straight back (`regrab_keyboard`) with no synchronous refresh — the focus
change makes niri emit an event, and the event stream redraws from that.

A ~120 ms debounce (`AUTO_SHOW_DEBOUNCE`) coalesces bursts: one pending target,
latest wins, so holding `j` through ten workspaces costs one focus call. A pending
target equal to the last sent one is dropped. A window that dies during the
debounce is recorded as sent anyway — no retry at a dead id, no error banner over
a map being browsed fast. Focus stays on the last previewed target when the mode is
switched off or the overlay quits; nothing is restored.

A focus the mode didn't make is recorded too (`AutoShow::record_sent`, called from
`activate_selection`), so `Enter` and the second-click path keep the guard matching
niri. Without it, a target focused by hand looks unfocused to the guard and the next
navigation back to it would be dropped as redundant, leaving the accent border and
niri's focus on different windows.

The move actions (`Shift+J`/`K`, `Shift+H`/`L`, `Ctrl+H`/`L`, pointer drops) end
with `keep_grab_under_auto_show()` while the mode is on: their `restore_focus` puts
niri back on the previewed target rather than the overlay, and a workspace that
crosses outputs carries niri's output focus with it, either of which would strand
the keyboard grab.

An accent `auto-show` pill sits in the bottom-right beside the `? keys` hint, drawn
even when the overlay is unfocused — the mode moves focus as soon as a key is
pressed, so a background map has to show it's armed.

## Mouse drag-and-drop

Everything the keyboard moves can also be done by dragging (a `GestureDrag` on
the `DrawingArea`):

- **Grab a workspace header** → move the whole workspace: reorder it within its
  monitor, or drag it onto another monitor.
- **Grab a column** (a window/column body) → move the column: reorder it within
  its workspace, or drop it on another workspace / monitor.

A plain **click** (a press that doesn't pass the drag threshold) selects what's
under the cursor exactly like `hjkl`: clicking a window selects that window (and
its workspace), clicking a workspace header or empty card area selects the
workspace. Selection happens on press in `drag_begin` via `hit_select()`, so it
applies whether or not the press turns into a drag. A **second click on the
already-selected** item (i.e. click to select, then click again — or just a
double-click) **focuses** it, exactly like `Enter`: `drag_begin` records
`Drag::was_selected` (the target was the visible selection at press time) and a
release without a drag calls `activate_selection()`, the helper shared with the
`Enter` key handler.

`compute_layout()` produces the positioned boxes shared by rendering and pointer
hit-testing. While a drag is in progress the model refresh is **frozen** (so the
grabbed geometry stays put), neighbours **reflow** to open a gap at the drop slot
(eased per-item positions in `anim_ws` / `anim_col`, advanced by a frame-clock
tick callback), and the grabbed item floats under the cursor. On drop the move is
applied with niri actions — `move-workspace-to-index` /
`move-workspace-to-monitor` for workspaces; `move-column-to-index` /
`move-column-to-workspace` (`--focus false`) / `move-column-to-monitor` for
columns — then the freeze lifts and the event stream syncs the result. Drop
indices are mapped through neighbours' real niri indices so hidden trailing empty
workspaces don't offset them.

## Command-line options

- `--solo <monitor>` — start in solo mode showing only that output's content
  (full-width). This is about *what content* is shown, not where the overlay
  appears. Ignored if no output matches the name.
- `--open-on-monitor <monitor>` — place the overlay surface on that output (via
  `gtk4-layer-shell` `set_monitor`). niri **cannot** position a layer-shell
  surface from config — `open-on-output` is a *window*-rule and doesn't apply to
  layer surfaces, and layer-rules have no output property — so the client must
  request it. Independent of `--solo`: e.g. `--open-on-monitor eDP-1 --solo
  HDMI-A-1` puts the overlay on eDP showing HDMI's map.
- `--app-id <id>` — set the **layer-shell namespace** (default `niri-groom`).
  niri identifies a layer surface by its namespace, so this is what niri
  *layer-rules* match (for opacity/shadow/etc. — not placement). A valid, unique
  `GApplication` id is *derived* from it (`derive_app_id`) so single-instance
  still works and distinct namespaces are distinct instances — e.g. a persistent
  map (`--app-id niri-groom-map`) coexists with the `Mod+G` grooming instance.
- `--toggle` — make the binding a toggle: a second launch of the same instance
  closes the overlay instead of re-presenting it. (GApplication forwards the
  second `activate` to the running instance; with `--toggle` the guard in
  `build_ui` calls `app.quit()`. Note the *first* launch must carry `--toggle`,
  since it's the primary's opts that decide — using the same bind both presses
  guarantees that.)
- `--focus` — move niri's focus onto the overlay's output at launch (via
  `niri msg action focus-monitor`), so the exclusive-keyboard surface grabs the
  keyboard and is navigable even when opened on a non-focused output. Mainly
  useful with `--open-on-monitor`, since otherwise the overlay opens on the
  already-focused output.

Args are parsed by `parse_args()` before the app is built, and the app is run
with `run_with_args` passing only argv[0] so `GApplication` doesn't try to parse
our flags. A typical toggle bind:
`niri-groom --toggle --solo HDMI-A-1 --open-on-monitor eDP-1 --app-id niri-groom-map`.

## Theming and config

Colors come from a `Theme` (`src/theme.rs`): a handful of base colors (bg /
surface / window / text / subtext / accent / urgent); the extra shades the
drawing needs (separators, subtle borders, selection tints) are *derived* from
those — separators and borders are `text` at a low alpha, so the same code reads
correctly on both dark and light palettes. Nine themes ship: catppuccin
mocha/macchiato/latte, gruvbox material/light, tokyo-night, nord, dracula,
rose-pine. Every color in the drawing pulls from the active theme; there are no
hardcoded palette values left (only the dim-backdrop black behind modals).

`t` opens an in-overlay picker (modal, keyboard-captured like rename) listing the
themes with little color swatches. Moving the highlight (`j`/`k` or arrows)
applies the theme **live** to the whole overlay; `Enter` saves it, `Esc` reverts.

The config (`src/config.rs`) is `$XDG_CONFIG_HOME/niri-groom/niri-groom.kdl`
(falling back to `~/.config/...`), created with the default on first run. It's
read at startup and rewritten on save via the `kdl` crate, which round-trips the
document so comments and any other keys survive. The schema is `theme
"<name>"` (default catppuccin-mocha), an optional `workspace-badges
command="..."` and any number of `mark-kind` nodes (see below).

## Workspace badges

I can flag workspaces with a small colored pill — I use it to surface the
workspace *bookmarks* my niri config maintains, but the app deliberately knows
nothing about bookmarks. The mechanism is generic (`src/badges.rs`): a config
key `workspace-badges command="<cmd>"` names a command I run (via `sh -c`, so
`~`/pipes work) on startup and on every refresh. It prints one tab-separated
line per workspace to mark:

```
<workspace-name>\t<label>[\t#rrggbb]
```

I match by workspace **name** (case-insensitively, like niri) and draw a pill
with `<label>` in the top-right of the card. The pill color is the optional
third field, falling back to the theme's `marker` color (each palette's
yellow/gold, distinct from `accent`/`urgent`). The pill is the *only* mark —
there's no colored border (an earlier version outlined the card too, but the
pill alone reads more cleanly). An empty label therefore shows nothing. Badges
are decorative: a missing/failing command or unparseable output just yields no
badges, never an error.

Keeping the *definition* of a "bookmark" outside the app is the whole point — my
bookmarks live as `<key> { focus-workspace "<name>"; }` binds in
`~/.config/niri/bookmarks.kdl`, and a one-line `niri-groom-badges.sh` greps that
into the tab-separated format. niri doesn't expose configured binds over IPC, so
that knowledge can only come from such a command.

## Workspace marks

A **mark** is a workspace's flagged state — the write half of the badge
mechanism. The app keeps no mark state of its own: the config declares the
*kinds* of mark, each naming a key and a command I run (via `sh -c`):

```kdl
mark-kind "work"     key="m" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
mark-kind "personal" key="p" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
```

Pressing a kind's key runs its command with the selected workspace name in
`$NIRI_GROOM_WORKSPACE` and the kind's name in `$NIRI_GROOM_MARK_KIND`. The
command owns the store (a file, an extra line in `bookmarks.kdl`, whatever) and
flips the mark there; the `workspace-badges` command above reads it back, so a
marked workspace simply shows as a pill — there's no separate rendering. Binding
the *same* script to a niri key toggles from outside the app, so niri and the
overlay share one source of truth and can't diverge. With no kind declared, no
key toggles anything.

The name is a label: it reaches the command and it names the key in the `?`
legend (`key_legend` therefore returns owned strings, one row per kind). Nothing
else reads it, which is the point — the app never learns that a kind means
"work" or "personal", and never learns that two kinds exclude each other. My
work and personal marks *are* mutually exclusive, and that rule lives in the one
script both kinds point at, which owns both stores and clears the other when it
sets one. See `docs/adr/0001-mark-kinds-are-configuration.md`.

Several kinds may share a command, and a kind whose key collides with a built-in
map-mode binding (`RESERVED_KEYS`) is dropped, so a typo can't shadow
navigation. `m` is absent from that list on purpose: it's the key
`workspace-mark-toggle command="<cmd>"` uses, the shorthand for a single kind
named `mark`, read only when no `mark-kind` node is present.

A mark needs a stable identity, and the only stable handle is the workspace
name (marks are keyed by name, like badges). So a mark key on an *unnamed*
workspace opens the rename field first (`mark_after_rename`, which carries
*which* kind is pending), and applies the mark once a non-empty name is
committed; cancelling the rename clears the pending mark.

Because a mark change is just a file write, niri emits no event for it, so a
toggle from a niri bind would otherwise only show up on the slow 2s fallback
poll. To make it instant I watch a **refresh-trigger file**
(`$XDG_RUNTIME_DIR/niri-groom-refresh`, `refresh_trigger_path`) with a
`gio::FileMonitor`: touching it refreshes every running overlay at once. The
mark command touches it after writing. It's a generic poke — anything that
mutates the badge source can touch it — and it's why a background map updates
immediately when I mark from the keyboard. (The in-overlay mark keys also
refresh synchronously, so they don't depend on the poke.)

`Enter` focuses the *selected window* (`focus-window`), falling back to the
workspace when it's empty, and never quits: it always hands the keyboard grab
straight back to the overlay's own monitor afterwards (`focus_monitor`), which
is needed because focusing a workspace on another output moves niri's output
focus there first (`focus_workspace_by_id` does its own `focus-monitor` before
`focus-workspace`). Re-focusing the overlay's monitor undoes that theft — or
is a no-op if the target was already there — so the map keeps browsing while
the picked window or workspace becomes niri's real focus underneath.

`Shift+Enter` runs the same focus call but quits the overlay when the target
is on the overlay's own monitor (found via `overlay_output` — the connector of
the `Monitor` under the window's surface): there the overlay covers the thing
you're switching to, so it must close; on another monitor it stays put as a
map and you keep working on the other screen (losing focus hides its
selection).

The overlay opens on whichever workspace is currently focused. Killing a
workspace closes all its windows and then runs `unset-workspace-name` on it, so
it becomes an unnamed empty workspace — which is then hidden (see above) and
reclaimed by niri. There's no niri action to remove a workspace directly; niri
auto-reclaims empty unnamed workspaces (except the trailing one per monitor)
when they lose focus, which is why hiding them is the clean answer rather than
trying to force deletion.

niri's `move-workspace-up`/`-down` only act on the *focused* workspace, and
`focus-workspace` only resolves within the focused output. So moving the selected
workspace is a sequence: focus its monitor → focus it by index → move. Because
that disturbs focus, I record the previously focused workspace first and refocus
it afterwards (by id, re-reading state since the reorder shifts indices), leaving
focus where the user left it.

The overlay uses `KeyboardMode::Exclusive`: while it is **focused** it grabs the
whole keyboard. niri releases that grab when the surface loses focus (e.g. you
focus another monitor), so it does *not* trap the keyboard when left running in
the background — which makes the two usage modes possible: a quick grooming
session (launch, act, quit) or a persistent map left on a second monitor. (For
automated testing there's usually no second surface to focus away to, so it
stays focused/grabbing — **never run it unattended in a test without an auto-kill
timeout**; see Testing below.)

When the overlay is **not** focused, its own selection cursor (the accent
border on the selected workspace/window) and the `? keys` hint are hidden, so a
background map shows only niri's "you are here" focus highlight rather than a
stale selection competing for attention. This is driven by GTK's
`Window::is_active` (`State::active`); in the focused grooming flow it's always
active, so nothing changes there.

It's single-instance: `GApplication` (the default unique behaviour, keyed on the
app id) forwards a second launch's `activate` to the running instance and the
second process exits. The `activate` handler (`build_ui`) guards on
`app.windows()` — if a window already exists it just `present()`s it and returns,
so pressing the keybind twice can't stack two exclusive keyboard grabs (which
otherwise deadlocks input until the app is killed).

## Tech stack

- **Rust** — matches niri itself; single binary, no runtime deps beyond the GTK libs.
- **GTK4** (`gtk4` crate 0.11) for the windowing + the cairo `DrawingArea` I paint on.
- **gtk4-layer-shell** (`gtk4-layer-shell` crate 0.8) to anchor the window as a
  fullscreen overlay surface and grab the keyboard.
- **serde / serde_json** to parse the niri IPC output.
- **kdl** (crate 6) to read/write the KDL config, preserving comments on save.
- **async-channel** to hand event-stream pings from the reader thread to the GTK
  main loop, and **libc** for the child's `PR_SET_PDEATHSIG`.
- **pangocairo** (crate 0.22) to lay out and paint the text.

Text is drawn through Pango (`layout_for` builds a `pango::Layout`, `text_at`
paints it with `pangocairo`), truncated with an ellipsis to fit. Pango falls back
across font families per glyph, so an emoji in a window title is rendered by an
emoji font even though the body text is plain sans-serif. The `Font` struct
carries just a pixel size and a bold flag (the only variation the labels need);
`text_at` positions text by its baseline, matching where the old cairo
`show_text` drew it.

The font family is the bare generic `sans-serif` (`FONT_FAMILY`) — *not* a list.
Naming extra emoji families there makes fontconfig resolve the body text to some
other (worse-reading) face, so the emoji handling is done by presentation
selector instead.

Emoji are forced to **monochrome** so they don't clash with the themed card
backgrounds. The lever is Unicode presentation, not the font: Pango picks a color
emoji font for any character that defaults to emoji presentation, ignoring the
requested family. So `force_text_presentation` appends the text-presentation
selector `U+FE0E` after each such character (identified via the embedded
`Emoji_Presentation` range table) and drops any explicit `U+FE0F`; fontconfig's
own per-glyph fallback then supplies the monochrome outline, painted in the
current text color like the rest of the label. Multi-character emoji (skin-tone /
ZWJ sequences) don't survive this and fall back to their component glyphs — an
accepted trade for a uniformly monochrome map.

## Layout of the code

- `src/niri.rs` — the IPC layer. `Workspace` / `Window` / `Output` deserialization,
  helpers (`label()`, `column()`, `row()`), and the `fetch_*` / `close_window` /
  rename / move calls. This is the only module that shells out to `niri`.
- `src/main.rs` — everything GTK. `build_model()` turns the IPC snapshot into the
  `Model` (outputs → workspaces → windows, plus a flat `nav` order for selection),
  `refresh()` rebuilds it while preserving the selection by id, the key handler, and
  the cairo drawing functions (`draw` → `draw_workspace` → `draw_window`, plus
  `draw_rename` / `draw_picker`).
- `src/autoshow.rs` — the auto-show mode's bookkeeping: the target niri's focus
  was last pointed at, the one waiting behind the debounce, and the rules for
  which of them to send. Plain state, so its rules are unit-tested directly.
- `src/theme.rs` — the `Theme` struct, derived-color helpers, and the bundled
  theme presets.
- `src/config.rs` — locating, creating, reading and writing the KDL config.
- `src/badges.rs` — running the optional `workspace-badges` command and parsing
  its tab-separated output into the per-workspace badge map.

## Dev environment

I keep all build tooling **localized to this repo** via a Nix flake — nothing is
installed globally.

```sh
# One-off (or let direnv do it automatically — see below):
nix develop            # drops into a shell with rustc, cargo, gtk4, gtk4-layer-shell

# Inside the shell:
cargo build
cargo run
```

With [direnv](https://direnv.net/) the `.envrc` (`use flake`) loads the shell
automatically on `cd`. Run `direnv allow` once.

> **Flakes only see git-tracked files.** After adding a new file, `git add` it (no
> commit needed) or `nix develop` / `nix build` will error with "not tracked by Git".

`nix build` produces a standalone binary at `./result/bin/niri-groom`, and
`nix run` builds and runs it.

## Testing / running safely

Because the overlay grabs the keyboard exclusively, verify it with an auto-kill so a
bug can't trap input:

```sh
nix develop --command bash -c 'cargo build && timeout --signal=KILL 2.5 ./target/debug/niri-groom'
```

A clean exit code of `137` means the timeout killed it as expected (not a crash). The
`Gdk-WARNING ... Vulkan: ... VK_ERROR_INCOMPATIBLE_DRIVER` line on Asahi hardware is
harmless — GTK probes for Vulkan and falls back to the GL/cairo renderer.

## Conventions

- Match niri's IPC field names in the structs so the JSON maps directly.
- Keep `niri.rs` as the single place that shells out to the `niri` binary.
- Prose and comments are written in the first person.
- Don't add a confirmation step to the kill actions — instant kill is the whole point.
- Committing directly in this repo is fine — I don't need to ask first. Keep commits
  focused, with a concise imperative subject. (New files must still be `git add`ed
  before a `nix` invocation will see them.)
- I run the **cargo release binary** at `./target/release/niri-groom`, not the debug
  build. So after implementing a change, **always finish by running
  `cargo build --release`** (inside `nix develop`) to refresh it — otherwise I won't
  see the change.

## Agent skills

### Issue tracker

Issues and specs live as markdown files under `.scratch/<feature-slug>/` in this
repo, and are committed. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles, each label string equal to its name. See
`docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root, both created lazily
when a term or a decision actually needs recording. See `docs/agents/domain.md`.
