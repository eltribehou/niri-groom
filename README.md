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
| `h` / `l`       | Select previous / next **window**            |
| `Tab` / `S-Tab` | Jump to the next / previous **screen**       |
| `Enter`         | **Focus** the selected workspace and close the overlay |
| `w`             | Kill the selected workspace (all its windows) |
| `x`             | Kill the selected window                     |
| `r`             | Refresh                                      |
| `q` / `Esc`     | Quit                                         |

There is **no confirmation** — `w` and `x` kill immediately. Killing a workspace
also drops its name so niri reclaims the empty workspace.

## Run

Requires [Nix](https://nixos.org/) with flakes enabled. All build tooling
(Rust + GTK4 + gtk4-layer-shell) is provided by the flake; nothing is installed
globally.

```sh
nix run            # build and launch
# or
nix build          # → ./result/bin/niri-groom
```

For development, `nix develop` drops you into a shell with `cargo`, or use
[direnv](https://direnv.net/) (`direnv allow`) to load it automatically.

To install it permanently (e.g. via home-manager) and bind it to a niri key, see
the deployment notes in [CLAUDE.md](./CLAUDE.md#deploying-with-home-manager).
