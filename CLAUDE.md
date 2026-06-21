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
- Draws each output's workspaces (stacked, labelled by name + window count), and
  the windows inside each workspace laid out by their real scrolling-layout position
  (`layout.pos_in_scrolling_layout` → column, row).
- Hides unnamed empty workspaces. niri keeps a permanent trailing empty workspace
  per monitor (plus transient empties after moves); these are scratch space that
  can't be meaningfully killed, so showing them only confuses. Named empty
  workspaces are kept (you can still rename or kill them).
- Refreshes on an 800ms timer so the map keeps up with the compositor.
- Kills windows with `niri msg action close-window --id <id>`. "Killing a workspace"
  means closing every window it holds (niri keeps named/empty workspaces around by
  design, so an empty workspace is a no-op).

### Keybindings

| Key            | Action                                   |
| -------------- | ---------------------------------------- |
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
| `Enter`        | Focus the selected workspace and dismiss the overlay |
| `r`            | Rename the selected workspace (inline text field) |
| `t`            | Open the theme picker (live preview; Enter saves, Esc cancels) |
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

niri's `set-workspace-name` is a **case-insensitive no-op** — setting `foo` over
`Foo` does nothing, so a case-only edit would silently fail. `rename_focused_workspace`
works around this by first setting a throwaway intermediate name (a zero-width-space
prefix) and then the real one, which forces the change through. I avoid the simpler
unset-then-set because unsetting an empty workspace's name can let niri reclaim it
mid-rename.

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
document so comments and any other keys survive. Schema today is just
`theme "<name>"`. The default is catppuccin-mocha.

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

The overlay uses `KeyboardMode::Exclusive`, so while it is open it grabs the whole
keyboard. That's intentional (it's a transient modal tool), but it means I should
**never run it unattended without an auto-kill timeout** — see Testing below.

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

Text is drawn with cairo's toy font API (`select_font_face` / `show_text`) and
truncated with an ellipsis to fit — deliberately no pango dependency, the labels are
short.

## Layout of the code

- `src/niri.rs` — the IPC layer. `Workspace` / `Window` / `Output` deserialization,
  helpers (`label()`, `column()`, `row()`), and the `fetch_*` / `close_window` /
  rename / move calls. This is the only module that shells out to `niri`.
- `src/main.rs` — everything GTK. `build_model()` turns the IPC snapshot into the
  `Model` (outputs → workspaces → windows, plus a flat `nav` order for selection),
  `refresh()` rebuilds it while preserving the selection by id, the key handler, and
  the cairo drawing functions (`draw` → `draw_workspace` → `draw_window`, plus
  `draw_rename` / `draw_picker`).
- `src/theme.rs` — the `Theme` struct, derived-color helpers, and the bundled
  theme presets.
- `src/config.rs` — locating, creating, reading and writing the KDL config.

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
