# Config

shroud reads `~/.config/shroud/config.toml` (or `$XDG_CONFIG_HOME/shroud/config.toml`). Every setting is optional, and the file reloads within a second of saving it.

```toml
shell = "/opt/homebrew/bin/fish"   # what new panes run; default $SHELL
leader = "ctrl+space"
scrollback = 10                    # megabytes of history per pane; 0 keeps none
pane_frames = false                # true: a frame around the tiles, with each pane's title on the border above it

[theme]                            # "default", a palette index 0-255, or "#rrggbb"
"border.active" = "#7aa2f7"
status.bg = 0

[keys.leader]                      # keys after the leader, merged over the table above
"g t" = "next-tab"                 # space-separated keys make a sequence
"x" = "none"                       # unbind a default
"|" = { command = "vsplit", replace = true }  # the only vsplit key: v is unbound

[keys.direct]                      # keys that act without the leader; panes never see them
"ctrl+h" = "focus-left"
"ctrl+j" = "focus-down"
"ctrl+k" = "focus-up"
"ctrl+l" = "focus-right"
```

Binding a key replaces whatever that key did, but the command keeps its default keys too, as a mapping does in nvim. To move a command rather than add a key for it, write `{ command = "vsplit", replace = true }`: its default keys are unbound, and so is a plugin command's suggested key. Keys you bound yourself stay, whatever order they are written in, and a key you free can be bound again. `replace` works in `[keys.direct]` too.

Theme tokens: `border.active`, `border.inactive`, `border.broadcast` (the other panes' borders while a tab broadcasts), `title.active`, `title.inactive` (pane titles, with `pane_frames`), `status.fg`, `status.bg`, `status.mode`, `status.session`, `status.tab`, `status.tab_active`, `status.message`, `switcher.border`, `switcher.match`, `switcher.selected`, `badge.error`, `badge.done`.

Commands: `vsplit`, `split`, `focus-left`/`-down`/`-up`/`-right`, `focus-next`, `focus-prev`, `focus-last-used`, `focus-first`, `focus-last`, `close`, `only`, `grow-height`, `shrink-height`, `grow-width`, `shrink-width`, `equalize`, `maximize-height`, `maximize-width`, `move-left`/`-down`/`-up`/`-right`, `exchange`, `rotate`, `prompt-prev`, `prompt-next`, `copy-output`, `copy-command`, `new-tab`, `move-to-new-tab`, `next-tab`, `prev-tab`, `switcher`, `new-session`, `rename-session`, `next-session`, `prev-session`, `detach`, `send-leader`, `goto-tab`, `last-tab`, `last-session`, `zoom`, `broadcast`, `float-or-tile`, `rename-tab`, `visual`, `visual-char`, `visual-line`, `visual-block`, and `frame-time` (`g p`), which toggles the previous render's cost in the status bar. A binding can fix a command's count, as `"alt+3" = "goto-tab 3"`; the defaults bind `1`–`9` that way. Counts typed after the leader still work with rebound keys. If an unbound `ctrl+<letter>` or arrow follows the leader, shroud treats it as the plain letter, as nvim does.

## Shortcuts

A binding can run a program instead of a command. `run` opens a pane in the focused pane's directory and types the command into its shell; `exec` runs a program with no pane and puts the result in the status bar:

```toml
[keys.leader]
"g e" = { run = "nvim" }                            # a float; it closes when nvim exits
"g k" = { run = "k9s", open = "tab" }               # "float" (the default), "right", "down" or "tab"
"g t" = { run = "cargo test", open = "down", keep = true }
"g P" = { exec = ["git", "pull"] }                  # git pull ✓ Already up to date.
"g b" = { exec = "make && make install", timeout = 60 }
```

A `run` pane is a command pane. When the command succeeds, the pane closes. When it fails, or always with `keep = true`, the pane holds and the status bar says `cargo test exited 101 · Enter reruns · q closes`: Enter runs it again, `q` closes the pane, and any other key gives you the shell. Seeing the command end needs [shell integration](usage.md#shell-integration); without it the pane stays open as a shell.

`exec` takes a program and its arguments, run directly, or a string, run with `/bin/sh -c`. It runs in the focused pane's directory with shroud's environment, not your shell's, so set `PATH` where shroud starts, or use `run` for anything that needs your shell. The bar shows `…` while it runs, then ✓ or ✗ with the exit code and the last line it printed. After `timeout` seconds (600 by default) it gets SIGTERM, with everything it started, then SIGKILL.

Both take `cwd`, relative to the focused pane's directory; `~/` works. The hints box and `bindings` show a shortcut by what it runs.

A mistake costs only the setting it's in. The status bar shows the first problem with its line number, and a red `config` stays on the right until the file is fixed. A syntax error, such as a half-saved file, keeps the config that was already running. `shroud config check` lists every problem.

## Layouts

A new session starts from `.shroud.kdl` in its starting directory if there is one, otherwise from `~/.config/shroud/layout.kdl`, otherwise with one pane. Parent directories aren't searched.

```kdl
tab {
    vsplit {
        pane
        split size="30%" {
            pane cwd="src"
            pane command="cargo test"
        }
    }
}
tab
```

`vsplit` puts its children side by side and `split` stacks them, as in nvim. `size` is a percentage, and children without one share what's left. `cwd` is relative to the session's directory (`~/` works too). `command` is typed into the pane's shell, so the shell stays open after the command ends. If the layout has a problem, the session starts with one pane and the status bar says why.
