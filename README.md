# shroud

A terminal multiplexer for modern terminals, built on [libghostty-vt](https://github.com/ghostty-org/ghostty), Ghostty's terminal-emulation library. Made for working locally across many projects in [Ghostty](https://ghostty.org).

## Why shroud

tmux and Zellij are both excellent and shaped how shroud works: tmux's scripting model and copy mode, Zellij's floating and command panes. shroud explores a different set of trade-offs: it's built for one modern terminal on your own machine rather than for every terminal and remote servers, so it can lean on what that terminal does well. It aims to be fast enough that you forget it's there.

- **Performant.** Damage-tracked frame diffing, synchronized output, and backpressure instead of queued frames.
- **Shell-aware.** Shell integration survives the multiplexer: shroud tracks OSC 133 prompt and command marks and OSC 7 working directories in every pane, so you can jump between prompts, copy a command's output, open splits where you are, and see when a command in another pane fails.
- **Matches Ghostty.** Panes are parsed by the same library Ghostty uses, so widths, graphemes, the kitty keyboard protocol, links, images and synchronized output behave the same inside shroud as outside it.
- **Themeable.** Every colour shroud draws is a semantic token. The default theme uses only your terminal's own palette, so shroud follows your Ghostty theme, light or dark.
- **Extensible, like nvim.** A small core with a scripting API, an event stream and Lua plugins. Key hints and session saving are built-in plugins written against the same API.
- **Stable.** A crashed server leaves your terminal intact, a config mistake costs only that setting, and every decoder of untrusted input is fuzzed.

[DESIGN.md](DESIGN.md) explains the decisions behind it.

## Status

shroud is young (0.1). It's developed on macOS and also builds and passes its tests on Linux. Expect the config, scripting API and plugin API to change before 1.0.

Features:

- sessions, tabs, splits, floating panes and zoom, with nvim's `CTRL-W` keys after a leader;
- named sessions you create, switch between, rename and close from the CLI, leader keys or a fuzzy switcher;
- detach and reattach with the screen restored; the server keeps your shells running;
- prompt jumping, copying a command's output, and status-bar badges for commands finishing in other panes;
- an nvim-style visual mode over scrollback, with search, text objects and mouse selection;
- a config file for keys, theme and shell, reloaded as you save it, and layout files a new session starts from;
- shortcuts: keys that open a program in a pane or run one in the background;
- a scripting API: open panes, type into them, run commands and wait for their exit codes, and stream events;
- Lua plugins, installed from git and pinned in a lockfile, run in their own process and watched so a stuck one can't freeze shroud;
- your terminal's real colours passed to pane programs, OSC 8 links, OSC 52, notifications and images in panes;
- `shroud bench`, a frame-time overlay, and performance floors checked in CI.

## Installing

With [Homebrew](https://brew.sh), on macOS or Linux:

```bash
brew install edjeffreys/tap/shroud
```

`brew install --HEAD edjeffreys/tap/shroud` builds the newest `main` from source instead.

Without Homebrew, each [release](https://github.com/edjeffreys/shroud/releases) has a prebuilt binary for macOS (Apple silicon and Intel) and Linux (x86_64 and arm64, glibc 2.35 or newer), with a SHA-256 checksum for each:

```bash
target=aarch64-apple-darwin   # or x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
version=0.1.0
curl -LO "https://github.com/edjeffreys/shroud/releases/download/v${version}/shroud-${version}-${target}.tar.gz"
tar -xzf "shroud-${version}-${target}.tar.gz"
install "shroud-${version}-${target}/shroud" ~/.local/bin/   # or anywhere on your PATH
```

To try the newest `main`, the [`dev` prerelease](https://github.com/edjeffreys/shroud/releases/tag/dev) is rebuilt whenever CI passes on it, at fixed links:

```bash
curl -LO "https://github.com/edjeffreys/shroud/releases/download/dev/shroud-dev-${target}.tar.gz"
```

`shroud --version` says which commit a dev build is, such as `0.2.0-dev.12+g3f9c2ab`. Or build it yourself.

## Building

You need Rust (stable) and Zig 0.17.x. Zig only builds libghostty-vt.

```bash
# macOS
brew install rustup zig && rustup default stable
# elsewhere: Rust from https://rustup.rs, Zig 0.17 from https://ziglang.org/download/
```

```bash
cargo build --release
```

The first build downloads the libghostty-vt 1.0.0 source tarball, checks its SHA-256, and builds it with Zig, which takes about 40 seconds on an M-series Mac. Later builds reuse it from `target/`.

To build offline or against a system package, set `SHROUD_LIBVT_DIR` to a prefix containing `lib/libghostty-vt.a`.

shroud is developed and tested in Ghostty. Other terminals that speak the kitty keyboard protocol should work, but some features, such as loading shell integration into panes, rely on Ghostty.

## Usage

```bash
shroud    # or target/release/shroud if you built it
```

Running `shroud` attaches to the session you used most recently, starting the server and a session called `default` if needed. The server keeps every session's panes alive after you detach.

```bash
shroud session new [<name>]        # start a session in this directory and attach
shroud session attach [<name>]     # attach to a session
shroud session ls                  # list sessions
shroud session rename <old> <new>
shroud session kill <name>         # close a session and its shells
shroud server restart              # start a new server, e.g. after a rebuild
shroud server stop                 # stop the server and close every shell
```

The server outlives its windows on purpose, so quitting Ghostty leaves your shells running. `shroud server restart` stops it and every attached window starts the new binary and reattaches. Shells start afresh, since the old server owned them; with the `sessions` plugin's `auto_restore` on, the layouts come back as they were up to two seconds before.

Inside shroud, `Ctrl+Space C` creates a session (it asks for a name and starts in the current pane's directory), `Ctrl+Space $` renames the current one, and `Ctrl+Space (` / `)` move to the previous or next session.

`Ctrl+Space ;` opens the switcher: every session, most recently used first. Type to filter, and if the name you type doesn't exist, Enter creates it. Move with the arrows or `Ctrl+N` / `Ctrl+P`; Enter switches, `Ctrl+R` renames, `Ctrl+X` closes (after asking), Escape leaves.

When a session ends, its clients move to the most recently used session left, and exit if there isn't one.

The leader is **Ctrl+Space**. After it, keys follow nvim's `Ctrl+W` window commands:

| Keys after the leader | Action |
|---|---|
| `v` / `s` (or `n`) | split: new pane right / below |
| `h` `j` `k` `l` (or arrows) | move focus |
| `w` / `W` | next / previous pane; with a count, e.g. `03w`, go to pane 3 |
| `p` | the previously focused pane |
| `t` / `b` | first / last pane |
| `c` / `q` | close pane |
| `o` | close every other pane |
| `+` / `-` | taller / shorter (counts work: `010+`) |
| `>` / `<` | wider / narrower |
| `=` | make all panes equal |
| `_` / `\|` | maximise height / width; with a count, set it (`020_`) |
| `H` `J` `K` `L` | move pane to the far left / bottom / top / right |
| `x` | swap with the next pane |
| `r` | rotate panes in the row or column |
| `[` / `]` | jump mode: select the last finished command, then keep pressing `[` `]` without the leader to move between commands; `y` copies the selected one's output, `Y` the command line with it, `v` selects its output in visual mode; any other key returns (shroud only) |
| `y` | copy the last command's output to the clipboard (shroud only) |
| `Y` | copy the last command line and its output, as `$ cmd` then the output; just the output if your shell doesn't mark where input starts (shroud only) |
| `d` | detach (shroud only) |
| `N` | new tab with a fresh shell (shroud only) |
| `T` | move the focused pane into its own tab (nvim's `CTRL-W T`) |
| `1`–`9` | go to that tab (shroud only) |
| `gt` / `gT` | next / previous tab |
| `g Tab` | the last tab you were on (nvim's `g<Tab>`) |
| `;` | open the session switcher (shroud only) |
| `C` | new session (shroud only) |
| `$` | rename this session |
| `(` / `)` | previous / next session |
| `'` | the last session you were in (shroud only) |
| `gp` | show or hide the frame-time overlay (shroud only) |
| `f` | show or hide this tab's floating panes; opens one if there are none (shroud only). From a float, `h`/`j`/`k`/`l` move to the next float that way, or past the last one back to the tiles, hiding the floats |
| `F` | open another floating pane (shroud only) |
| `e` | float the focused pane, or tile a floating one (shroud only) |
| `z` | zoom: the focused pane fills the tab until you press it again or move focus (shroud only) |
| `,` | name this tab; an empty name clears it (shroud only) |
| `V` | visual mode: scroll back, search and copy with nvim keys (shroud only) |
| `g v` / `g V` / `g Ctrl+v` | visual mode with a character, line or block selection already started (shroud only) |
| `Ctrl+Space` | send a literal Ctrl+Space |

As in nvim, `Ctrl+<letter>` after the leader works the same as the plain letter, so `Ctrl+Space Ctrl+h` moves focus left.

Counts start with `0`, because `1`–`9` go straight to tabs: `Ctrl+Space 03w` focuses pane 3. Unbind a digit in config and it starts a count again.

Visual mode starts at the cursor and follows nvim:

- `h` `j` `k` `l`, `w` `b` `e` `ge`, `W` `B` `E` `gE`, `0` `^` `$`, `gg` `G`, `H` `M` `L`, `Ctrl+d` `Ctrl+u` `Ctrl+f` `Ctrl+b`, with counts;
- `f` `F` `t` `T` find a character on the line, and `;` / `,` repeat it;
- `[` / `]` jump between prompts;
- `/` and `?` search (smartcase), moving as you type, with matches on screen underlined; Escape goes back to where you started, and `n` / `N` repeat it;
- `v`, `V` and `Ctrl+v` select by character, line or block, and `o` jumps to the other end;
- with a selection, text objects replace it: `iw` `aw` `iW` `aW`, `i"` `a"` (and `'`, `` ` ``), `i(` `a(` (and `[`, `{`, `<`; `b` and `B` too);
- `y` copies the selection and leaves. Without one, `y` takes a motion or text object as nvim's does: `yw`, `ye`, `y$`, `yiw`, `yi"`, `yt,`, `yj`; `yy` and `Y` copy lines. Enter copies the selection, or the cursor's line;
- Escape cancels a pending `y`, then clears the selection, then leaves; `q` leaves at once. The leader also leaves, so window keys still work.

`visual` enters with nothing selected. `visual-char`, `visual-line` and `visual-block` enter with a selection already started, as `v`, `V` and `Ctrl+v` do in nvim. Their default keys are `g v`, `g V` and `g Ctrl+v`, since nvim's `CTRL-W v` is a split; with your splits bound elsewhere, `"v" = "visual-char"` and friends give you nvim's keys. The status bar shows `VISUAL`, `V-LINE` or `V-BLOCK`.

Prompt jumps follow nvim's `[[` and `]]` in terminal buffers.

The status bar along the bottom shows the leader or jump mode and the focused pane's directory and git branch on the left, and the time on the right. When a command fails or finishes in a pane you weren't looking at, that pane's number appears by the clock with ✗ or ✓ (`2✗`); `Ctrl+Space 02w` takes you there and clears it.

In jump mode the selected command's line is highlighted, and the bar says which it is, such as `3/12  cargo test  ✗1`: its place among the commands in the pane's history, the command, and how it exited when the shell said.

Jumping, copying and badges need your shell to send shell-integration marks. In Ghostty, shroud loads Ghostty's own integration into each pane for zsh and fish, with no dotfile changes. For bash, source `$GHOSTTY_RESOURCES_DIR/shell-integration/bash/ghostty.bash` from your `.bashrc`.

Panes set `SHROUD=1`, so dotfiles can tell they're inside shroud, and `shroud` typed in a pane prints its usage rather than nesting. They also set `TERM_PROGRAM=ghostty`, because programs in a pane are talking to Ghostty's parser whatever terminal you run shroud in. A `.zshrc` that starts shroud in Ghostty windows should check `$SHROUD` first, or every pane will try to start it again:

```zsh
[[ -z $SHROUD && $TERM_PROGRAM == ghostty ]] && exec shroud
```

Mouse:

- click a pane to focus it;
- the wheel scrolls back through a pane's history unless the program in it uses the mouse itself;
- in a pane whose program doesn't use the mouse, drag to select and release to copy; double-click copies a word and triple-click a line.

A program that uses the mouse (nvim with `mouse=a`, htop) gets every click and drag itself. Ghostty selects natively if you hold Shift, but that selection spans pane borders.

## Config

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

[keys.direct]                      # keys that act without the leader; panes never see them
"ctrl+h" = "focus-left"
"ctrl+j" = "focus-down"
"ctrl+k" = "focus-up"
"ctrl+l" = "focus-right"
```

Theme tokens: `border.active`, `border.inactive`, `title.active`, `title.inactive` (pane titles, with `pane_frames`), `status.fg`, `status.bg`, `status.mode`, `status.session`, `status.tab`, `status.tab_active`, `status.message`, `switcher.border`, `switcher.match`, `switcher.selected`, `badge.error`, `badge.done`.

Commands: `vsplit`, `split`, `focus-left`/`-down`/`-up`/`-right`, `focus-next`, `focus-prev`, `focus-last-used`, `focus-first`, `focus-last`, `close`, `only`, `grow-height`, `shrink-height`, `grow-width`, `shrink-width`, `equalize`, `maximize-height`, `maximize-width`, `move-left`/`-down`/`-up`/`-right`, `exchange`, `rotate`, `prompt-prev`, `prompt-next`, `copy-output`, `copy-command`, `new-tab`, `move-to-new-tab`, `next-tab`, `prev-tab`, `switcher`, `new-session`, `rename-session`, `next-session`, `prev-session`, `detach`, `send-leader`, `goto-tab`, `last-tab`, `last-session`, `zoom`, `float-or-tile`, `rename-tab`, `visual`, `visual-char`, `visual-line`, `visual-block`, and `frame-time` (`g p`), which toggles the previous render's cost in the status bar. A binding can fix a command's count, as `"alt+3" = "goto-tab 3"`; the defaults bind `1`–`9` that way. Counts typed after the leader still work with rebound keys. If an unbound `ctrl+<letter>` or arrow follows the leader, shroud treats it as the plain letter, as nvim does.

### Shortcuts

A binding can run a program instead of a command. `run` opens a pane in the focused pane's directory and types the command into its shell; `exec` runs a program with no pane and puts the result in the status bar:

```toml
[keys.leader]
"g e" = { run = "nvim" }                            # a float; it closes when nvim exits
"g k" = { run = "k9s", open = "tab" }               # "float" (the default), "right", "down" or "tab"
"g t" = { run = "cargo test", open = "down", keep = true }
"g P" = { exec = ["git", "pull"] }                  # git pull ✓ Already up to date.
"g b" = { exec = "make && make install", timeout = 60 }
```

A `run` pane is a command pane. When the command succeeds, the pane closes. When it fails, or always with `keep = true`, the pane holds and the status bar says `cargo test exited 101 · Enter reruns · q closes`: Enter runs it again, `q` closes the pane, and any other key gives you the shell. Seeing the command end needs shell integration (see above); without it the pane stays open as a shell.

`exec` takes a program and its arguments, run directly, or a string, run with `/bin/sh -c`. It runs in the focused pane's directory with shroud's environment, not your shell's, so set `PATH` where shroud starts, or use `run` for anything that needs your shell. The bar shows `…` while it runs, then ✓ or ✗ with the exit code and the last line it printed. After `timeout` seconds (600 by default) it gets SIGTERM, with everything it started, then SIGKILL.

Both take `cwd`, relative to the focused pane's directory; `~/` works. The hints box and `bindings` show a shortcut by what it runs.

A mistake costs only the setting it's in. The status bar shows the first problem with its line number, and a red `config` stays on the right until the file is fixed. A syntax error, such as a half-saved file, keeps the config that was already running. `shroud config check` lists every problem.

### Layouts

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

## Scripting

Scripts and agents drive shroud through `shroud pane` and `shroud events`. They target the pane given with `-p`, else the pane the script is running in (`$SHROUD_PANE`), else the focused pane.

```bash
shroud pane run --wait 'cargo test'      # opens a pane, runs it, prints its output, exits with its code
shroud pane split --down --cwd ~/src     # prints the new pane's id; focus stays where it was
shroud pane split --float                # a floating pane over the tab
shroud pane run --float --close lazygit  # a popup: the pane closes when lazygit exits
shroud pane run --keep 'cargo test'      # a command pane: when it ends, Enter reruns and q closes
shroud pane send -p 7 'make' -k enter    # text and keys, in order
shroud pane output -p 7                  # the last command's output
shroud pane capture -p 7 --history       # the pane's text: the screen, or with --history everything kept
shroud command zoom                      # any command a key can be bound to, as if it were pressed
shroud pane close -p 7
shroud pane ls                           # sessions, tabs and panes as JSON
shroud events                            # JSON lines: command_finished, pane_opened, pane_closed
```

New panes don't take focus unless you pass `--focus`, so a script never moves your cursor. `--wait` and `command_finished` need shell integration in the pane (see above). Without it, `--wait` fails after 10 seconds rather than hang.

Underneath is newline-delimited JSON on `control.sock` in shroud's runtime directory (`$XDG_RUNTIME_DIR/shroud` where your system sets it, as most Linux desktops do; otherwise `$TMPDIR/shroud-$UID`), so any language can talk to it directly:

```
→ {"id":1,"cmd":"split","dir":"down","command":"cargo test","wait":true}
← {"id":1,"ok":true,"pane":8,"exit":0,"output":"…"}
→ {"cmd":"send","pane":8,"input":[{"text":"q"},{"key":"ctrl+c"}]}
← {"ok":true}
→ {"cmd":"subscribe"}
← {"ok":true}
← {"event":"command_finished","pane":8,"exit":1}
```

The commands are:
- **panes:** `list` (each pane's `process` is its foreground program, when that isn't the shell), `split` (`pane`, `dir`, `cwd`, `command`, `focus`, `wait`, `close_on_exit` or `keep`, `float` with `x`, `y`, `w`, `h` in percent), `send`, `output`, `capture` (`history` for everything kept), `close`, `focus`;
- **commands:** `run` (`command` such as `"zoom"`, `"goto-tab 3"`, `"sessions.save"` or a shortcut table like `{"exec": ["git", "pull"]}`, as if `client` pressed its key; without `client`, a client on `session`), so anything a key can do, a script or plugin can too;
- **layouts and sessions:** `layout` (a tab in the layout-file model, with sizes in percent), `apply_layout` (a new tab from one), `set_layout` (rearrange an existing tab: a pane node with `pane` moves that pane, one without starts a new one, and every tile must be placed), `pane_title` (names a pane on its border when `pane_frames` is on, in place of its program's title; no `title` goes back to that), `tag` (a small string a plugin keeps on a pane, like tmux's `@user` options: shown in `list` and `layout`, taken by `apply_layout` and `set_layout` for new panes, and saved by `sessions`; no `value` removes it), `session_new`, `session_rename`, `session_kill`, `tab_new`;
- **for plugins:** `message` (text in the status bar for a few seconds), `status_take` / `status_set` / `status_release` (draw the status bar yourself: you get a `status` event with each client's state whenever it changes, and answer with spans for the left and right), `overlay_open` (optionally for one `client`), `overlay_update` and `overlay_close` (a box of text whose spans take a theme token or colour for `style` and `bg`, plus `bold` and `italic`; a modal one sends its keys to you, and Escape always closes it), `register_command` (a `plugin.command` name with a suggested key, used only if it's free), `bind` (a key to any command or shortcut table, over config, until the connection closes; no `command` unbinds) and `bindings`;
- `subscribe`, for events: `command_finished`, `pane_opened`, `pane_closed`, `focus_changed`, `layout_changed`, `tab_changed`, `session_opened`, `session_closed`, `session_renamed`, `client_attached`, `client_detached`, `key_pending` (the leader and keys so far, after a 300 ms pause), `key_done`, `mode_changed` (`normal`, `leader`, `jump` or `visual`), `bell`, `title_changed`, `cwd_changed`, `tag_changed` and `output_started` (a pane printing after 2 s without output, not counting a key's echo: activity in a background pane). `command`, `overlay_key` and `overlay_closed` go only to the connection they concern.

Overlays and commands belong to the connection that made them and go away with it. A mistake gets `{"ok":false,"error":"…"}` and the connection stays open.

## Plugins

Plugins are Lua, run by LuaJIT in a separate process (`shroud __plugins`) that the server starts and watches. A plugin that runs a handler for more than 2 seconds gets the process restarted without it, and `plugin` appears in the status bar; it stays off until you change the config. Plugins run as you, with Lua's `io`, `os` and `ffi`, as nvim plugins do.

Declare them in `~/.config/shroud/init.lua`, as in nvim:

```lua
shroud.plugins {
  { "github:someone/shroud-harpoon", version = "v1.2.0",   -- a tag, branch or commit
    opts = { marks = 4, label = function(n) return "#" .. n end } },
  { "builtin:sessions", config = function(plugin, opts) plugin.setup(opts) end },
  { "builtin:hints", enabled = false },                     -- turn off a built-in
  "path:~/src/shroud-hello",                               -- one you're writing
}

-- init.lua is your own plugin too, with the whole API:
shroud.on("session_opened", function(ev) shroud.message{ text = "hello " .. ev.session } end)
```

`opts` goes to the plugin's `setup` and can hold functions; `config`, if given, is called instead of `setup`. init.lua runs in the plugin host, never the server, and restarts the host when you save it. Plugins can also be listed in `config.toml`, which is enough when the options are plain values:

```toml
[[plugins]]
src = "github:someone/shroud-harpoon"
version = "v1.2.0"
[plugins.opts]
marks = 4
```

A spec in init.lua replaces config.toml's entry for the same source.

Then:

```bash
shroud plugin sync      # install what's missing, at the commits in plugins.lock
shroud plugin update    # move to the newest commit of each version and relock (or name some)
shroud plugin ls        # what's installed, where, and anything the watchdog turned off
shroud plugin clean     # delete plugins config no longer lists
```

`sync` writes `plugins.lock` beside `config.toml`; commit it with your config and every machine runs the same plugin code. Plugins install to `$XDG_DATA_HOME/shroud/plugins` (`~/.local/share/shroud/plugins`). git runs from these commands, never from the server, and they restart the plugin host when they're done.

A plugin is a directory with two files:

```toml
# plugin.toml
name = "hello"
version = "0.1.0"
api = "0.1"
[commands]
greet = { key = "g y" }            # Ctrl+Space g y, used only if nothing else has it
```

```lua
-- init.lua
local word
shroud.command("greet", function(ev)
  local pane = shroud.split{ dir = "down", float = {} }
  shroud.send{ pane = pane, input = { { text = "echo " .. word }, { key = "enter" } } }
end)
return { setup = function(opts) word = opts.word end }
```

Two plugins come built in, written with the same API as any other:
- **hints**, on by default: pause after `Ctrl+Space` (or partway through a sequence) and a box lists what each next key does.
- **sessions**, off by default: saves every session's tabs, splits and directories as they change and when you detach; `Ctrl+Space g s` saves now, and `Ctrl+Space g r` reopens saved sessions that aren't running, such as after a reboot. A program that was running in a pane, such as `nvim notes.md`, is typed back at its prompt without Enter, so you choose whether to rerun it (`retype = false` turns that off). With `opts = { auto_restore = true }` it does that by itself once when the server starts, taking over the first client's new session if it has the same name as a saved one. It restores shells in the right directories, not the programs that were running.

```lua
shroud.plugins {
  { "builtin:sessions", opts = { auto_restore = true } },  -- turn it on
  { "builtin:hints", enabled = false },       -- or turn hints off; any plugin can be switched off this way
}
```

(or `src = "builtin:sessions"` and `enabled = false` in a `[[plugins]]` entry in config.toml).

Every scripting command is a function of the same name (`shroud.split`, `shroud.capture`, `shroud.layout`, …, and `shroud.run("zoom")`) that returns the useful part of the reply, or `nil, err`. A call waits for its reply without blocking shroud, because each handler runs in its own coroutine. A plugin can also draw the whole status bar: `shroud.status_take()`, then answer each `status` event with `shroud.status_set{ client = ev.client, left = {…}, right = {…} }`. There are also `shroud.on(event, fn)`, `shroud.after(ms, fn)`, `shroud.message{text = …}` (in the status bar), `shroud.log(msg)` (and `print`, both to the server log), `shroud.data_dir()`, `shroud.server` (the server's `pid` and `started`, in Unix milliseconds, for doing something once per server) and `shroud.json.encode` / `decode`. Modules go in the plugin's `lua/` directory, for `require`.

Keys can be bound from Lua too, as nvim's `vim.keymap.set` does. A bind sits over config.toml and plugins' default keys, and lasts while the plugin is loaded:

```lua
shroud.keymap("g y", "zoom")                                  -- after the leader
shroud.keymap("ctrl+h", "focus-left", { direct = true })      -- without it
shroud.keymap("g u", function(ev) shroud.message{ text = "hi" } end, { name = "hi" })
shroud.keymap("g e", { run = "nvim" })                        -- a shortcut, as in config.toml
shroud.keymap("x", nil)                                       -- unbind
```

A function becomes a command of the plugin (`user.hi` from init.lua), so hints and `bindings` can name it. `shroud.run{ exec = { "git", "pull" } }` runs a shortcut without a key.

To run a program, use `shroud.system` (nvim's `vim.system`), not `io.popen` or `os.execute`, which block every plugin and trip the two-second watchdog:

```lua
local r = shroud.system({ "git", "status", "--short" }, { cwd = dir }):wait()   -- r.code, r.stdout, r.stderr
shroud.system({ "make" }, function(r) shroud.message{ text = "make: " .. r.code } end)
```

`:wait()` yields the handler like any other call; the options are `cwd`, `env`, `stdin` and `timeout` (milliseconds, then SIGTERM), and `job:kill(signal)` stops one early.

For completion in your editor, `shroud plugin types > shroud.lua` writes definitions for lua-language-server; add that file's directory to `workspace.library`.

When something goes wrong, the server log is `server.log` in the same runtime directory. If the server crashes, the client restores your terminal and says so; Ghostty is never affected.

## Throughput

`time cat` of a 200 MB file mixing text, colour and wide characters, on an M-series Mac with Ghostty 1.3.1, 2026-11-07:

| | Time |
|---|---|
| Ghostty, no multiplexer | 1.57 s |
| Ghostty, inside shroud (release build) | 1.87 s |

The cost is shroud's own parse of the stream, about 107 MB/s. Ghostty only ever sees a few dozen diffed frames, not the 200 MB. To reproduce, run `scripts/bench.sh` once in a plain Ghostty window and once inside shroud.

### Compared with tmux and Zellij

These numbers come from one synthetic workload on one machine, to keep shroud honest about its own performance rather than to rank the tools. tmux and Zellij do more than shroud does (remote use, many terminals, their own plugin systems) and make different trade-offs, and they may well do better on workloads not measured here.

`cargo run --release -p shroud-smoke --bin shroud-compare -- target/release/shroud` (add `--mb N` or `--only shroud,tmux`) drives each multiplexer headless through a PTY at 200×50: the same bash, an isolated socket and config, and the outer PTY drained as fast as it can be read, so it times the multiplexer rather than a terminal's renderer. M-series Mac, shroud release build, tmux 3.8, zellij 0.45.1, 2026-11-08:

| | `cat` 50 MB | CPU for it | key echo p50 / p99 | while another pane floods | idle CPU | memory after the `cat` | to first prompt |
|---|---|---|---|---|---|---|---|
| no multiplexer | 0.36 s | — | 0.32 / 0.55 ms | — | — | — | 0.01 s |
| shroud | 0.46 s | 0.38 s | 0.59 / 1.01 ms | 0.19 / 0.24 ms | 0% | 17 MB | 0.03 s |
| tmux | 2.61 s | 2.54 s | 0.43 / 0.79 ms | 200.6 / 200.7 ms | 0% | 8 MB | 0.03 s |
| zellij | 0.84 s | 2.16 s | 12.5 / 13.3 ms | 4.7 / 7.1 ms | 0% | 71 MB | 0.10 s |

Key echo is a bash loop that writes each key back as eight copies of itself, typed 15 ms apart; "while another pane floods" is the same with `yes` running in a second pane. At that relaxed pace tmux's echo is 0.1–0.2 ms quicker from run to run; typed back to back, shroud echoes in 0.05 ms. With a pane flooding, the other pane's echo took about 200 ms under tmux in this test. shroud's memory includes 10 MB of history (`scrollback`), where tmux keeps 2,000 lines by default.

Memory is resident size of every process the multiplexer started, shells excluded; shroud's includes the plugin host.

`shroud bench` times the server's hot path in-process, with no PTY or terminal involved, so its numbers repeat. On the same Mac, release build:

```
shroud bench (release, 64 MB, 200×50)
  parse    485.6 MB/s
  frame  p50 0.184 ms  p99 0.255 ms  max 0.268 ms  (4.7 KB written per frame)
  echo   p50 0.013 ms  p99 0.018 ms  max 0.188 ms  (53 B written per frame)
```

`frame` redraws a full screen of new output; `echo` adds one character to a full screen, as a keystroke does. `shroud bench --check` fails if parsing drops below 40 MB/s, a frame's p99 goes above 4 ms or an echo frame's above 1 ms; CI runs it. The smoke test also holds an idle server to under 1% CPU.

## Development

```bash
cargo test
```

`cargo test` runs the unit tests, the golden-frame snapshots and the layer lint, with no PTY or socket involved.

```bash
cargo run -p shroud-smoke -- target/debug/shroud
```

The smoke test drives the real binary through PTYs, each section with its own isolated TMPDIR and server, all at once. It reads the frames back through libghostty-vt, so its checks look at the screen: it splits, types, reloads config, starts a layout, drives the scripting API and plugins, detaches, reattaches, restarts and exits. Name sections after the binary's path to run only those.

The decoders that read untrusted bytes have fuzz targets. Fuzzing needs nightly Rust and cargo-fuzz, but only for these targets; shroud itself builds on stable.

```bash
rustup toolchain install nightly && cargo install cargo-fuzz
```

```bash
cargo +nightly fuzz run -s none input_decode
```

The other targets are `protocol_message`, `shell_tracker`, `config_parse` and `api_request`.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first. Agents should start at [CLAUDE.md](CLAUDE.md).

## Licence

shroud is released under the [MIT licence](LICENSE). libghostty-vt and LuaJIT, which it links statically, are MIT-licensed too.
