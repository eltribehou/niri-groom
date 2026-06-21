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
| `Enter`         | **Focus** the selected workspace and close the overlay |
| `r`             | **Rename** the selected workspace (inline field, readline/Emacs keys) |
| `t`             | Open the **theme** picker (live preview; Enter saves, Esc cancels) |
| `w`             | Kill the selected workspace (all its windows) |
| `x`             | Kill the selected window                     |
| `q` / `Esc`     | Quit                                         |

There is **no confirmation** — `w` and `x` kill immediately. Killing a workspace
also drops its name so niri reclaims the empty workspace.

## Themes

Nine built-in themes — catppuccin (mocha/macchiato/latte), gruvbox
(material/light), tokyo-night, nord, dracula, rose-pine. Press `t` to pick one
with a live preview. The choice is saved to
`$XDG_CONFIG_HOME/niri-groom/niri-groom.kdl` (created on first run; default
catppuccin-mocha):

```kdl
theme "catppuccin-mocha"
```

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
