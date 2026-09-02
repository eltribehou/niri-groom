# Mark kinds

A **mark** is a workspace's flagged state, keyed by the workspace name and kept
in a store outside the app. A **mark kind** is one independent category of mark.
Which kinds exist is configuration, so the app never learns what any of them
means: "work" and "personal" appear in `niri-groom.kdl` and in the scripts, never
in `src/`.

## Why it exists

I want to tell apart the workspaces I keep for work from the ones I keep for
personal things, and cycle through only one group at a time. The existing single
mark becomes the work kind, unchanged.

## Config

```kdl
mark-kind "work"     key="m" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
mark-kind "personal" key="p" command="~/.config/niri/scripts/niri-groom-mark-toggle.sh"
```

- Declared in order. Each node carries a name, a single-character key, and the
  command that flips the mark.
- The name is a label: it reaches the toggle command in
  `$NIRI_GROOM_MARK_KIND`, and it names the key in the `?` legend. Nothing else
  reads it.
- Two kinds may share one command, which is how one script can own several
  stores at once.
- `workspace-mark-toggle command="..."` stays valid as shorthand for a single
  kind on `m`, named `mark`. The default config template documents `mark-kind`
  only.
- A kind whose key collides with a built-in map-mode binding is dropped, so a
  typo can't shadow navigation. A kind missing its key or command is dropped too.

## Keys

- The declared key toggles that kind on the selected workspace, exactly as `m`
  does today: run the command with the workspace name in
  `$NIRI_GROOM_WORKSPACE` and the kind name in `$NIRI_GROOM_MARK_KIND`, then
  refresh.
- A mark needs a stable name, so on an unnamed workspace the key opens the
  rename field and applies the mark once a non-empty name is committed.
  `State::mark_after_rename` therefore carries *which* kind is pending rather
  than a plain flag; cancelling the rename clears it.
- The `?` legend lists one row per declared kind, so `key_legend()` returns
  owned strings instead of `&'static str`.

## Exclusivity

Work and personal are mutually exclusive, and the app does not enforce it. It
owns no store and, by design, does not know the kinds are related. The toggle
script owns both stores, so it clears the other kind when it sets one. This is
recorded in `docs/adr/0001-mark-kinds-are-configuration.md`.

## Rendering

Nothing changes. The badges command already merges every source into one pill
per workspace, and exclusivity means a workspace never needs two. Every kind
uses the same `⭐`, and the pill's color is what tells them apart — both chosen
in the badges script: gruvbox-material aqua `#7daea3` for personal against green
`#a9b665` for work. Single-codepoint glyphs only — the monochrome forcing breaks
multi-codepoint sequences.

## Cycling

Cycling lives outside the app, as it does today.
`cycle-niri-groom-marked-workspaces.sh` takes the kind as `$1` and defaults to
the work store, so the existing niri bind keeps working unchanged.

## Outside this repo

In `~/.config/home-manager/dotfiles/`:

- `niri-groom/niri-groom.kdl` — the two `mark-kind` nodes.
- `niri/scripts/niri-groom-mark-toggle.sh` — pick the store from
  `$NIRI_GROOM_MARK_KIND`, and clear the other kind.
- `niri/scripts/niri-groom-badges.sh` — read the personal store and emit its
  glyph and color.
- `niri/scripts/cycle-niri-groom-marked-workspaces.sh` — take the kind as `$1`.
- `niri/binds.kdl` — `Alt+P` toggles the personal mark, `Ctrl+Shift+Space`
  cycles personal workspaces, mirroring `Alt+M` and `Ctrl+Space`. `Alt+P` is
  freed by dropping its `dirty-repos.sh --unpushed` bind.

The script and `niri-groom.kdl` trees are deployed with `mkOutOfStoreSymlink`, so
those edits are live at once. `binds.kdl` is preprocessed at build time and needs
`hms`.
