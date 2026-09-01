# niri-groom

A fullscreen overlay for the [niri](https://github.com/YaLTeR/niri) Wayland
compositor that shows my workspaces and windows as a proportional map — like the
overview, but with workspace names and window titles spelled out — and lets me kill a
whole workspace or a single window from the keyboard, instantly.

![keys: j/k workspace · h/l window · w kill workspace · x kill window · q quit](#)

## Keys

| Key             | Action                                       |
| --------------- | -------------------------------------------- |
| `j` / `k`       | Select next / previous **workspace** (crosses screens at the boundary) |
| `J` / `K`       | **Move** the selected workspace down / up within its monitor |
| `H` / `L`       | **Move** the selected workspace to the screen left / right |
| `h` / `l`       | Select previous / next **window**            |
| `C-h` / `C-l`   | **Move** the selected window's column left / right |
| `Tab` / `S-Tab` | Jump to the next / previous **screen**       |
| `s`             | **Solo** the selected monitor (toggle) — show only it, full-width |
| `f`             | **Auto-show** (toggle) — niri's focus follows the selection as you navigate |
| `Enter`         | **Focus** the selected window (closes the overlay only if it's on the overlay's monitor) |
| `r`             | **Rename** the selected workspace (inline field, readline/Emacs keys) |
| a mark key      | Toggle a kind of **mark** on the selected workspace (see below) |
| `t`             | Open the **theme** picker (live preview; Enter saves, Esc cancels) |
| `?`             | Toggle the **key legend** (hidden by default) |
| `w`             | Kill the selected workspace (all its windows) |
| `x`             | Kill the selected window                     |
| `q` / `Esc`     | Quit                                         |

There is **no confirmation** — `w` and `x` kill immediately. Killing a workspace
also drops its name so niri reclaims the empty workspace.

You can also **drag with the mouse**: grab a workspace's header to reorder it or
move it to another monitor, or grab a column to move it within its workspace or
onto another workspace/monitor. Neighbours slide to open a gap as you drag.

## Options

- `--solo <monitor>` — start showing only that monitor's content (full-width);
  ignored if no monitor matches.
- `--open-on-monitor <monitor>` — open the overlay on that monitor. (niri can't
  place a layer-shell surface from config, so the app requests it.) Independent of
  `--solo`.
- `--app-id <id>` — set the layer-shell namespace (default `niri-groom`); it keys
  single-instance, so a different id runs as a separate instance.
- `--toggle` — a second launch of the same instance closes the overlay, so one
  keybind opens and closes it, e.g.
  `niri-groom --toggle --solo HDMI-A-1 --open-on-monitor eDP-1 --app-id niri-groom-map`.
- `--focus` — move niri's focus onto the overlay's output at launch, so it grabs
  the keyboard and is navigable even when opened on another monitor (pairs with
  `--open-on-monitor`).

## Themes

Nine built-in themes — catppuccin (mocha/macchiato/latte), gruvbox
(material/light), tokyo-night, nord, dracula, rose-pine. Press `t` to pick one
with a live preview. The choice is saved to
`$XDG_CONFIG_HOME/niri-groom/niri-groom.kdl` (created on first run; default
catppuccin-mocha):

```kdl
theme "catppuccin-mocha"
```

## Workspace marks

A **mark** flags a workspace with a small pill, and niri-groom deliberately does
not decide what a mark means. Declare the kinds you want, each with a key and a
command that owns its store:

```kdl
// Read the marks back and draw the pills: one tab-separated line per flagged
// workspace, `<name>\t<label>[\t#rrggbb]`.
workspace-badges command="~/.config/niri/scripts/niri-groom-badges.sh"

// Toggle a mark. The key runs the command with the workspace name in
// $NIRI_GROOM_WORKSPACE and the kind name in $NIRI_GROOM_MARK_KIND.
mark-kind "work"     key="m" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
mark-kind "personal" key="p" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
```

Kinds may share one command, which is how a single script can own several stores
and keep them mutually exclusive if you want that. A kind's name shows beside its
key in the `?` legend; a key that collides with a built-in binding is ignored. On
an unnamed workspace a mark key opens the rename field first, since marks are
keyed by workspace name.

Bind the same script to a niri key and the overlay and the compositor share one
source of truth. Touching `$XDG_RUNTIME_DIR/niri-groom-refresh` makes every
running overlay re-read the marks at once.

## Build from source

niri-groom is a standard Rust (Cargo) project. At runtime it needs the
[niri](https://github.com/YaLTeR/niri) compositor; to build it you need a Rust
toolchain plus the GTK4 and gtk4-layer-shell development libraries and
`pkg-config`.

Install the system dependencies:

```sh
# Fedora
sudo dnf install gtk4-devel gtk4-layer-shell-devel pkgconf-pkg-config

# Arch
sudo pacman -S gtk4 gtk4-layer-shell pkgconf

# Debian / Ubuntu
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev pkg-config
```

Then build and install with Cargo:

```sh
cargo build --release      # → ./target/release/niri-groom
cargo install --path .     # → ~/.cargo/bin/niri-groom
```

### With Nix 

If you use [Nix](https://nixos.org/) with flakes, the flake pins the entire
toolchain and GTK stack. 

```sh
nix run                    # build and launch
nix build                  # → ./result/bin/niri-groom
nix develop                # dev shell with cargo, clippy, rustfmt, GTK…
```

With [direnv](https://direnv.net/), `direnv allow` loads the dev shell
automatically on `cd`.
