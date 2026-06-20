# niri-groom

A fullscreen [layer-shell](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)
overlay for the [niri](https://github.com/YaLTeR/niri) Wayland compositor. I survey
all workspaces and windows as a proportional map — like niri's overview, but with the
workspace name and each window's title shown clearly — and let myself kill a whole
workspace or a single window from the keyboard with no confirmation.

## What it does

- Reads the live state via `niri msg --json workspaces` and `niri msg --json windows`.
- Draws each output, its workspaces (stacked, labelled by name + window count), and
  the windows inside each workspace laid out by their real scrolling-layout position
  (`layout.pos_in_scrolling_layout` → column, row).
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
| `l` / `Right`  | Select next window in the workspace       |
| `h` / `Left`   | Select previous window                   |
| `Tab` / `Shift+Tab` | Jump straight to the next / previous screen (output) |
| `Enter`        | Focus the selected workspace and dismiss the overlay |
| `w`            | Kill the selected workspace (all windows) — no confirm |
| `x`            | Kill the selected window — no confirm     |
| `r`            | Force a refresh                          |
| `q` / `Esc`    | Quit                                     |

The overlay opens on whichever workspace is currently focused. Killing a
workspace closes all its windows and then runs `unset-workspace-name` on it, so
niri reclaims the now-empty (formerly named) workspace instead of leaving it
behind. Unnamed workspaces are reclaimed by niri automatically.

niri's `move-workspace-up`/`-down` only act on the *focused* workspace, and
`focus-workspace` only resolves within the focused output. So moving the selected
workspace is a sequence: focus its monitor → focus it by index → move. Because
that disturbs focus, I record the previously focused workspace first and refocus
it afterwards (by id, re-reading state since the reorder shifts indices), leaving
focus where the user left it.

The overlay uses `KeyboardMode::Exclusive`, so while it is open it grabs the whole
keyboard. That's intentional (it's a transient modal tool), but it means I should
**never run it unattended without an auto-kill timeout** — see Testing below.

## Tech stack

- **Rust** — matches niri itself; single binary, no runtime deps beyond the GTK libs.
- **GTK4** (`gtk4` crate 0.11) for the windowing + the cairo `DrawingArea` I paint on.
- **gtk4-layer-shell** (`gtk4-layer-shell` crate 0.8) to anchor the window as a
  fullscreen overlay surface and grab the keyboard.
- **serde / serde_json** to parse the niri IPC output.

Text is drawn with cairo's toy font API (`select_font_face` / `show_text`) and
truncated with an ellipsis to fit — deliberately no pango dependency, the labels are
short.

## Layout of the code

- `src/niri.rs` — the IPC layer. `Workspace` / `Window` deserialization, helpers
  (`label()`, `column()`, `row()`), and the `fetch_*` / `close_window` calls. This is
  the only module that shells out to `niri`.
- `src/main.rs` — everything GTK. `build_model()` turns the IPC snapshot into the
  `Model` (outputs → workspaces → windows, plus a flat `nav` order for selection),
  `refresh()` rebuilds it while preserving the selection by id, the key handler, and
  the cairo drawing functions (`draw` → `draw_workspace` → `draw_window`).

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

## Deploying with home-manager

The flake exposes `packages.default` (built with `buildRustPackage` + wrapped by
`wrapGAppsHook4`, so the GTK runtime env is set up). I install it through my
home-manager flake and bind it to a niri key, so day-to-day it's just the
`niri-groom` command — no `nix run`.

In the home-manager flake `inputs`:

```nix
niri-groom.url = "path:/home/sam/proj/perso/niri-groom";
# (or a git remote once pushed, e.g. "git+ssh://…/niri-groom")
```

In a home-manager module (make `inputs` available via `extraSpecialArgs`):

```nix
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.niri-groom.packages.${pkgs.system}.default ];
}
```

Then bind it in the niri config (KDL):

```kdl
binds {
    Mod+Shift+K { spawn "niri-groom"; }
}
```

After editing the binary's source I rebuild the home-manager generation
(`home-manager switch --flake …`, bumping the flake input with
`nix flake lock --update-input niri-groom` if it's pinned) to pick up changes.

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
