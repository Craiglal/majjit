# Majjit

A Rust TUI to manipulate the [Jujutsu](https://github.com/jj-vcs/jj) DAG.

Inspired by the great UX of [Magit](https://magit.vc/).

Once you run the program you can press `?` to show the help info. Most of the commands you can see by running `jj help` in the terminal are implemented.

## Screenshots

### Help menu

![](media/screenshot1.png)

### Command output

![](media/screenshot2.png)

### Fuzzy matching

![](media/screenshot3.png)

## Features

- Browse the jj log tree with dynamic folding/unfolding of commits and file diffs.
- Multi-key command sequences with transient-menu style help popups. For example type `gpa` to run `jj git push --all`, or `gpt` to run `jj git push --tracked`, or `ss` to squash the selected revision into its parent.
- Output from jj commands is displayed in the bottom panel.
- Fuzzy matching for various features like selecting changes or bookmarks.
- Mouse support: left click to select, right click to toggle folding, and scroll wheel to scroll.
- Syntax-highlighted file diffs with line numbers, rendered through [delta](https://github.com/dandavison/delta) when it is installed.
- Visual 3-way merge-conflict resolution with [meld](https://meldmerge.org/).
- Draft commit messages with an AI command of your choice (press `da`), then review and edit before applying.

## External tools

Beyond `jj` itself, Majjit can shell out to a couple of external tools to improve
diffs and conflict resolution:

- **[delta](https://github.com/dandavison/delta)** (optional, recommended) — file
  diffs in the log tree are piped through delta (`--color-only --line-numbers`) for
  syntax highlighting and old/new line numbers. Delta is invoked with `--no-gitconfig`
  so the output is deterministic regardless of your personal delta config, using a
  dark (`Dracula`) theme. If `delta` is not on your `PATH`, Majjit falls back to
  jj's own colored git diff.
- **[meld](https://meldmerge.org/)** (required for conflict resolution) — the
  `jj resolve` action opens meld as a visual 3-way merge editor, auto-merging the
  clean hunks and presenting only the real conflicts for editing. Conflicts shown in
  diffs use jj's `snapshot` conflict-marker style.
- **[tuicr](https://tuicr.dev)** (optional, standalone prerequisite for review) —
  install tuicr separately and make it available on `PATH` to use Majjit's Review
  menu. `R R` reviews the selected change with `tuicr -r <change>-..<change>`, or
  reviews the working copy with `tuicr -w` when no change is selected. `R r` saves
  the selected change as the range base; after navigating to a tip, `R r Enter`
  reviews the range with `tuicr -r <base>..<tip>`.

## AI commit messages

Majjit can draft a commit (describe) message for the selected change using an AI
command you configure. Press `da` (Describe → AI generate) to run it; the generated
message opens in an editable input panel so you can review and tweak it before it is
applied with `jj describe`.

Configure the command under the `[majjit]` table in your jj config
(`jj config edit --user`). jj parses the value as TOML, so wrap the command in quotes:

```toml
[majjit]
ai-describe-command = 'your-ai-cli --commit-message'
```

The change's diff is piped to the command on stdin (with an optional bookmark
header), and these environment variables are exported for it to use:

- `MAJJIT_AI_DESCRIBE_CHANGE_ID` — the change id being described
- `MAJJIT_AI_DESCRIBE_BOOKMARKS` — space-separated bookmark names on the change
- `MAJJIT_AI_DESCRIBE_DIFF_BYTES` — byte size of the diff

The command should print the message to stdout. Markdown code fences and
`<think>…</think>` reasoning blocks (emitted by some reasoning models) are stripped
automatically.

## Supported jj commands

- `jj abandon`
- `jj absorb`
- `jj bookmark advance`
- `jj bookmark create`
- `jj bookmark delete`
- `jj bookmark forget`
- `jj bookmark list`
- `jj bookmark move`
- `jj bookmark rename`
- `jj bookmark set`
- `jj bookmark track`
- `jj bookmark untrack`
- `jj commit`
- `jj describe`
- `jj diff`
- `jj duplicate`
- `jj edit`
- `jj evolog`
- `jj file list`
- `jj file show`
- `jj file track`
- `jj file untrack`
- `jj git fetch`
- `jj git push`
- `jj git remote list`
- `jj interdiff`
- `jj metaedit`
- `jj new`
- `jj next`
- `jj parallelize`
- `jj prev`
- `jj rebase`
- `jj redo`
- `jj resolve`
- `jj restore`
- `jj revert`
- `jj show`
- `jj sign`
- `jj simplify-parents`
- `jj split`
- `jj squash`
- `jj status`
- `jj undo`
- `jj unsign`
- `jj workspace add`
- `jj workspace forget`
- `jj workspace list`
- `jj workspace rename`
- `jj workspace update-stale`

Plus a custom command (`C`) to run arbitrary `jj` commands.

## Installation

With cargo: 
```sh
cargo install --git https://github.com/anthrofract/majjit
```

Or run the nix flake:
```sh
nix run github:anthrofract/majjit
```

Or install with the nix flake:

```nix
inputs.majjit.url = "github:anthrofract/majjit";

...

nixpkgs.overlays = [ majjit.overlays.default ];
environment.systemPackages = [ pkgs.majjit ];
```
