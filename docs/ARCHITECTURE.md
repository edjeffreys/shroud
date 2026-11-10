# shroud architecture

Start here. This is the map: what the layers are, how a keystroke and a burst of output move through the code, and what the words mean. The reasoning behind the decisions lives in `DESIGN.md`; this file says where things are.

Paths are relative to `crates/`: `core/…` is `shroud-core`, `vt/…` is `shroud-vt`, `plugins/…` is `shroud-plugins`, `shroud/…` is the binary, and `smoke/…` is the end-to-end check, which drives the binary from outside.

## The big picture

```
 ┌─────────┐  raw bytes  ┌────────┐ unix socket ┌──────────────────────────────┐   PTY   ┌───────┐
 │ Ghostty │ ──────────▶ │ client │ ──────────▶ │ server                       │ ──────▶ │ shell │
 │         │ ◀────────── │ (dumb) │ ◀────────── │  sessions → tabs → panes     │ ◀────── │ nvim  │
 └─────────┘ frame bytes └────────┘   output    │  each pane: PTY + vt terminal│         └───────┘
                                                └──────────────────────────────┘
```

There is one server per user. It owns every PTY and every terminal, and it renders. A client only moves bytes between Ghostty and the server.

When config lists plugins, the server also starts one plugin host, `shroud __plugins`. It runs every Lua plugin and talks to the server over `control.sock` like any script, so a plugin stuck in a loop or crashing in C takes down only the host, which the server restarts.

Starting a session from a layout file runs a short-lived `shroud __layout` child to parse it, killed after 1 s, because the `kdl` crate can take seconds on a malformed file.

## Layers

Each layer imports only layers below it. The crate graph enforces the coarse split: `shroud` → `shroud-vt` → `shroud-core`, with `shroud-plugins` beside `shroud-vt`; it uses core's plugin sources, never vt. `crates/core/tests/layers.rs` checks the finer layers on every `cargo test`.

```
 5  shroud: main.rs config_file.rs layout_child.rs plugin_cli.rs bench.rs  CLI dispatch, where config and data live, the layout parser child, the plugin manager, the benchmark
 4  shroud: server/{mod,event_loop,render,reload,client_conn,control_conn,api,events,overlays,plugin_bar,plugin_commands,plugin_host,layout_api,input,visual,jump,mouse_select,outer,links,lifecycle,switch,launch,shortcuts} client/ runtime: sockets, poll, signals, config reload, scripting, the plugin host's lifecycle, shortcut jobs
 3  shroud: server/{session,tab,tab/floats,pane}             model: owns PTYs and terminals
 2  shroud: pty.rs        shroud-vt (crates/vt)   shroud-plugins (crates/plugins)   adapters: PTY, libghostty-vt C API, LuaJIT (host process only)
 1  core: config/ layout/ render/ input/ protocol/ pace/ shell/ visual/ statusbar overlay watchdog theme   pure logic, tested without I/O
 0  core: render/{color,cell,grid} input/event layout/rect   plain data
```

Four rules worth memorising:

- **Only `crates/vt` and `crates/plugins` touch C.** If you need something new from libghostty-vt, wrap it in vt. LuaJIT is linked through `plugins`, whose Lua runs only in the host process, never the server. `shroud-core` forbids `unsafe` outright.
- **`render/` never sees a pane, and `layout/` never sees a PTY.** Panes copy their screen into a `render::Grid`. Layout trees hold `PaneId`s.
- **Colour values live only in `theme.rs`** (and `render/sgr.rs`, which encodes them). shroud's chrome uses semantic tokens (`theme.border_active`). Pane cells keep the colour references their program wrote and are never recoloured.
- **Core code has no syscalls and no `unsafe`.** Where it produces bytes, it appends to a `&mut Vec<u8>` the caller owns.

## Trace: one keystroke

You press `Ctrl+W` in nvim, in the right-hand pane.

1. **Ghostty** has kitty keyboard flags enabled by `shroud/client/tty.rs`, so it sends `ESC [ 119 ; 5 u`.
2. **`shroud/client/mod.rs`** reads stdin and sends `input(bytes)`, encoded by `core/protocol/message.rs`.
3. **`shroud/server/event_loop.rs`** sees the client socket is readable. **`shroud/server/client_conn.rs`** decodes the message and feeds the bytes to its `core/input/decode.rs` decoder, which yields `KeyEvent { key: 'w', mods: CTRL }`.
4. **`core/input/keybinds.rs`** turns the key into a `core/input/chord.rs` `Chord` and checks it against the `Bindings` from config. It isn't the leader, no leader is pending and it isn't a direct key, so the result is `Passthrough`.
5. **`shroud/server/input.rs`** sends the event to the active tab's focused pane, found through `shroud/server/tab.rs`.
6. **`vt/key_encode.rs`** syncs the encoder from that pane's terminal. nvim asked for kitty flags, so the encoder produces `ESC [ 119 ; 5 u`; a plain shell would get `0x17`.
7. **`shroud/server/pane.rs`** queues the bytes, and **`shroud/pty.rs`** writes them to the PTY master when it's writable.

With the leader (`Ctrl+Space`, then `v`), step 4 returns `Pending` and then `Action(Split(Horizontal))`. `shroud/server/tab.rs` applies it with `core/layout/tree.rs`, spawns a pane, resizes the affected panes and marks the frame dirty.

## Trace: a burst of output

htop redraws, so its PTY has 9 KiB to read.

1. **`shroud/server/event_loop.rs`**: the PTY master is readable. **`shroud/server/pane.rs`** reads up to 64 KiB, then calls **`vt/terminal.rs`** `write`, which runs `ghostty_terminal_vt_write`.
2. During that write, libghostty-vt fires **effects**. A device-attributes query becomes a reply queued for the PTY. A title change is queued for clients. An OSC 133 or OSC 7 sequence from the shell updates the pane's `core/shell/tracker.rs`; a finished command in an unfocused pane gives that pane a badge. Callbacks only enqueue, because libghostty-vt isn't re-entrant.
3. The pane is marked dirty. **`core/pace.rs`** decides whether to render now or arm the timer: a client with unsent bytes is skipped, and frames are capped at 120 fps, dropping to 30 fps under a flood. A pane's first output after a key skips the cap.
4. At render time, **`vt/snapshot.rs`** updates the pane's render state and copies the **dirty rows only** into the pane's `render::Grid`. If htop opened a synchronized-output hold, the grid stays frozen until it ends.
5. **`core/render/compose.rs`** combines the layout rects (from `core/layout/tree.rs`) with the pane grids, borders (`core/render/borders.rs`) and the cursor of the focused pane into a `render::Frame`. **`core/statusbar.rs`** then draws the bottom row from that client's mode and the inputs in `shroud/server/status.rs`.
6. **`core/render/diff.rs`** compares that frame with the client's previous one and emits the minimal bytes: cursor moves, SGR transitions from `core/render/sgr.rs`, and text, all wrapped in `CSI ? 2026 h … l`. Rows whose `compose` key matches the previous frame's (same layout, theme and pane-row generations, nothing drawn since) are skipped without comparing cells.
7. **`shroud/server/client_conn.rs`** sends `output(bytes)`. **`shroud/client/mod.rs`** writes them to stdout, and Ghostty parses them a second time. That second parse is the unavoidable cost of any multiplexer that draws through the outer terminal (see `DESIGN.md`).

## Where does my change go?

| I want to… | Go to |
|---|---|
| add or change a default keybind | `core/input/bindings.rs` (`DEFAULTS`), `shroud/server/input.rs` (what an action does) |
| add a bindable command | an `Action` in `core/input/keybinds.rs`, its config name and count rule in `core/input/commands.rs` |
| add a config setting | `core/config/settings.rs` (parse, with a problem for bad values), then apply it in `shroud/server/mod.rs` `poll_config` |
| change the layout file format | `core/config/layout_file.rs` (KDL → `layout::Shape`, size and depth limits), `shroud/layout_child.rs` (parsing in a child with a deadline), `shroud/server/tab.rs` `from_shape` (spawning) |
| support a new input sequence from Ghostty | `core/input/decode.rs` (+ `core/input/event.rs` if it's a new event kind) |
| change what pane programs learn about the outer terminal's colours | `core/input/outer_colors.rs` (query and reply parsing), `shroud/server/outer.rs` (passing them, the scheme and grapheme widths to panes), `vt/terminal.rs` `set_default_colors` |
| change how OSC 8 links get from a pane to the terminal | `vt/snapshot.rs` (cell → `LinkId`), `core/render/link.rs` (the URI table), `core/render/diff.rs` (writing OSC 8), `shroud/server/links.rs` (dropping unused URIs) |
| change what bytes a pane receives for a key | `vt/key_encode.rs` |
| change how borders look | `core/render/borders.rs` (shape), `core/theme.rs` (colour) |
| change pane frames or titles | `shroud/server/tab.rs` `tiles` (the inset tile area), `shroud/server/launch.rs` (`frames`, carried to every tab), `core/render/compose.rs` `title` (drawing on the border), `shroud/server/api.rs` (`pane_title`) |
| add a colour for new chrome | a new token field in `core/theme.rs` with an ANSI-only default, plus its config key in `token_mut` and `TOKENS` |
| make frames smaller | `core/render/diff.rs`, `core/render/sgr.rs` |
| change frame-rate policy | `core/pace.rs` |
| react to a new terminal effect (OSC, query) | `vt/terminal.rs` (callback) → `shroud/server/pane.rs` / `shroud/server/mod.rs` (routing) |
| add a split or focus behaviour | `core/layout/tree.rs`; rearranging (resize, swap, rotate) is `core/layout/arrange.rs` |
| use prompt marks, exit codes or a pane's cwd | `core/shell/tracker.rs` (state), read via `shroud/server/pane.rs` |
| change what the status bar shows | `core/statusbar.rs` (layout, no I/O), `shroud/server/status.rs` (branch, clock, messages), `core/theme.rs` (colours) |
| change when a pane gets a badge | `shroud/server/pane.rs` (which commands count), `shroud/server/mod.rs` `read_pane` (focus rule) |
| jump between prompts or read a command's output or line | `vt/prompts.rs` (every prompt, any command's line and output), `vt/command_output.rs` (`last_output`, `last_command`), `shroud/server/jump.rs` (the selected command, its label, `y` / `Y` / `v`), `shroud/server/pane.rs` `exit_of` (exit codes by prompt) |
| support shell integration for another shell | `core/shell/inject.rs` |
| add a tab behaviour | `shroud/server/session.rs` |
| stop or restart the server | `shroud/server/lifecycle.rs` `control` (`StopServer`), `shroud/client/mod.rs` `restart` (waiting for the old socket, then running the binary again) |
| add a session operation | `shroud/server/lifecycle.rs`, then expose it from the CLI (`shroud/main.rs` `session`, a `Control` in `core/protocol/message.rs`), the switcher (`core/switcher.rs`) or a key |
| change how the switcher looks or filters | `core/switcher.rs` |
| add a client⇄server message | `core/protocol/message.rs`, then both ends (bump the protocol version if old clients would misread it) |
| add a scripting command or event | `core/protocol/api.rs` (`Request` / `Event`, NDJSON shape), `shroud/server/api.rs` (what it does), `shroud/client/api.rs` (the `shroud pane` CLI) |
| change how a shell is spawned | `shroud/pty.rs` |
| read what a pane is running | `shroud/process.rs` |
| measure or gate performance | `shroud/bench.rs` (`shroud bench`, its floors), `smoke/src/sections/terminal.rs` (idle-CPU budget), `smoke/src/bin/shroud-compare.rs` (against tmux and zellij), `.github/workflows/ci.yml` |
| change how releases and dev builds are built or published | `.github/workflows/release.yml` (one native runner per target; the version, including dev builds' `-dev.N+g<commit>`; the release, the rolling `dev` prerelease, and the tap jobs), `.github/scripts/update-tap.sh` (the `shroud` and `shroud-dev` formulas and the tap's README), `shroud/main.rs` `VERSION` (`SHROUD_VERSION` from CI), `.github/dependabot.yml` |
| add an end-to-end check | `smoke/src/sections/` (one file per section, each with its own server), `smoke/src/harness.rs` (PTY clients read through libghostty-vt, waits), `smoke/src/ctl.rs` (`control.sock`, `shroud events`) |
| change floating panes | `core/layout/float.rs` (placement), `shroud/server/tab/floats.rs` (floats beside the tree, moving a pane between float and tile), `shroud/server/tab.rs` (focus order, zoom), `core/render/compose.rs` (drawn after the tiles) |
| change how images in panes reach the terminal | `vt/images.rs` (placements and pixels), `core/render/images.rs` (clipping and the Kitty protocol bytes), `shroud/server/images.rs` (what each client has been sent) |
| change how much history panes keep, or when it's compressed | `core/config/settings.rs` (`scrollback`), `vt/memory.rs` (limits, memory, compression), `shroud/server/history.rs` (applying the limit, compressing quiet panes) |
| change pane tags | `core/protocol/tags.rs` (names and limits), `shroud/server/api.rs` (`tag`, `list`), `shroud/server/layout_api.rs` (tags in layouts) |
| change rearranging a tab (`set_layout`) | `core/layout/rearrange.rs` (checking it), `shroud/server/tab/rearrange.rs` (moving and starting panes), `shroud/server/layout_api.rs` (the request) |
| change runtime key binds | `shroud/server/keymaps.rs` (`bind`, rebuilding the table), `plugins/raw.rs` (`keymap`), `core/input/bindings.rs` |
| change shortcuts or command panes | `core/input/shortcut.rs` (the table, its errors, summaries, held keys), `core/config/settings.rs` `command` (tables in config), `shroud/server/shortcuts.rs` (opening panes, holding, jobs and their timeouts), `shroud/server/keymaps.rs` `target_command` (tables from the API and Lua) |
| change how plugins run programs | `plugins/system.rs` (jobs and their threads), `plugins/raw.rs` (`system*` calls), `plugins/prelude.lua` (`shroud.system`), `plugins/lib.rs` (the loop's wake socket) |
| change visual mode | `core/visual/keys.rs` (keys, counts, `y` as an operator), `core/visual/motion.rs` (where motions land, search), `core/visual/object.rs` (text objects), `core/visual/mod.rs` (cursor, selection, the view), `vt/scrollback.rs` (reading and scrolling history), `shroud/server/visual.rs` (routing, copying), `core/render/highlight.rs` (drawing the selection) |
| change selecting with the mouse | `shroud/server/mouse_select.rs` (drags, double and triple clicks, the wheel in visual mode), `vt/scrollback.rs` `word_at` / `line_at` (Ghostty's word and line rules) |
| change go-to-tab, last tab or last session | `core/input/keybinds.rs` (digits are keys, counts start with 0), `shroud/server/session.rs` `last_tab`, `shroud/server/lifecycle.rs` `last_session` |
| change the plugin-drawn status bar | `shroud/server/plugin_bar.rs` (owner, `status` state, `status_set`), `core/overlay.rs` `draw_bar`, the `status_owner` branch in `shroud/server/render.rs` |
| change how overlays look or route keys | `core/overlay.rs` (sizing, drawing), `shroud/server/overlays.rs` (ownership, modal keys) |
| add an event | a variant in `core/protocol/api.rs`; state events are found in `shroud/server/events.rs` by comparing snapshots |
| change plugin commands | `core/input/commands.rs` (`Command`, plugin names), `core/input/bindings.rs` (`bind_plugin_default`), `shroud/server/plugin_commands.rs` |
| change the Lua API | `plugins/src/prelude.lua` (the `shroud` table), `plugins/src/raw.rs` (calls into Rust), `plugins/lua/shroud.lua` (definitions; a test checks every function has one) |
| change the built-in plugins | `plugins/builtin/<name>/` (`plugin.toml`, `init.lua`), `plugins/src/builtin.rs` (embedding and their tests), `core/config/plugins.rs` `BUILTIN` (which are on by default) |
| change init.lua or `shroud.plugins{…}` | `plugins/src/user.rs` (running init.lua as `user`, specs, resolving sources, the spec-only mode for the CLI), `shroud/plugin_cli.rs` `lua_specs`, `shroud/server/plugin_host.rs` (watching init.lua) |
| change how plugins load or are watched | `plugins/src/host.rs` (loading, coroutines, event routing), `plugins/src/manifest.rs` (`plugin.toml`), `core/watchdog.rs` (when to kill or give up), `shroud/server/plugin_host.rs` (spawning, `path:` sources, alerts) |
| change `plugin sync`, `update`, `ls` or `clean` | `shroud/plugin_cli.rs` (git runs here, never in the server), `core/config/plugins.rs` (install directory names, what a pin means), `core/config/lockfile.rs` |
| change layout or session requests | `shroud/server/layout_api.rs`; `Tree::to_shape` in `core/layout/shape.rs` |
| fuzz a new decoder or parser | a target in `fuzz/fuzz_targets/` plus a `[[bin]]` in `fuzz/Cargo.toml` (outside `crates/`) |

## Glossary

| Term | Meaning |
|---|---|
| **server** | The per-user background process that owns everything. Started by the first `shroud`. |
| **client** | A `shroud` process attached to a terminal window. It holds no state. |
| **session** | A named group of tabs. Names are chosen by the user and unique. |
| **tab** | One layout tree plus the panes it shows. A session shows one tab at a time. |
| **switcher** | The `Ctrl+Space ;` overlay for moving a client to another session or starting one in a directory. |
| **pane** | A PTY, the program on it, and one libghostty-vt terminal that tracks its screen. |
| **layout tree** | A binary tree of horizontal and vertical splits with ratios. The leaves are `PaneId`s. |
| **rect** | A screen-space rectangle in cells, `{x, y, w, h}`. |
| **cell** | One screen position: a grapheme, its width and its `Style`. |
| **grid** | A w×h array of cells. Each pane keeps one that mirrors its terminal, and every frame contains one. |
| **frame** | Everything one client should see: a grid, the cursor and the title. |
| **compose** | Building a frame from pane grids, rects and borders. |
| **diff** | Turning a previous frame plus the next frame into escape bytes. |
| **dirty** | Changed since the last frame. libghostty-vt tracks this per row, and shroud tracks it per pane. |
| **render hold** | A stretch where a pane program asked, with mode 2026, not to be drawn mid-update. |
| **effect** | A libghostty-vt callback for something the terminal can't do itself: a query reply, title, clipboard, bell. |
| **leader** | The prefix key (`Ctrl+Space` unless config says otherwise) that makes the next keys a shroud action instead of input. The default keys after it follow nvim's `CTRL-W` commands. |
| **chord** | One key plus modifiers, as written in config (`ctrl+h`, `H`, `f5`). Shift folds into the character. |
| **bindings** | The leader table (key sequences after the leader) and the direct table (single keys that act without it), each mapping to a command name. Config merges over the defaults. |
| **direct key** | A binding that acts without the leader. The pane never sees it. None by default. |
| **command** | The config name of an action, such as `vsplit` or `focus-left`. It also decides what a count means. |
| **problem** | A config mistake with its line. It costs only that setting; a syntax error keeps the running config. |
| **layout file** | `.shroud.kdl` in a session's starting directory, or the global `layout.kdl`: the tabs and splits a new session starts with. |
| **shape** | A layout as written, with splits of any number of parts. `Tree::from_shape` turns it into the binary layout tree. |
| **action** | A shroud command such as `Split`, `Focus(Left)`, `Grow` or `Detach`. |
| **passthrough** | An input event that isn't a keybind and is forwarded to the focused pane. |
| **backpressure** | Holding back new frames from a client whose socket still has unsent bytes. |
| **pacer** | The pure policy that decides when to render next. |
| **shell integration** | Sequences a shell emits about itself: OSC 133 prompt and command marks with exit codes, and OSC 7 for the cwd. shroud records them per pane. |
| **status bar** | The bottom row of every frame: the mode, the focused pane's cwd and git branch (or a short message) on the left; badges and a clock on the right. |
| **badge** | A pane number and mark at the right of the status bar, such as `2✗`: a command failed (✗) or finished (✓) in that pane while you weren't looking. Focusing the pane clears it. |
| **jump mode** | After `Ctrl+Space [` or `]`: a client's selected command, starting at the last finished one. Further `[` and `]` move it; `y`, `Y` and `v` act on it; another key leaves. |
| **injection** | Starting a pane's shell with an environment that makes it load the outer terminal's shell-integration scripts. |
| **prompt mark** | A row where a prompt or a command started or ended, recorded by `shell::Tracker`. |
| **handshake** | `hello` → `welcome` on connect. It checks that client and server speak the same protocol version. |
| **control socket** | `control.sock`, the NDJSON scripting API and event stream. It is separate from the client socket. |
| **target pane** | The pane a scripting request acts on: `-p`, else `$SHROUD_PANE` (set in every pane), else the focused pane of the most recently used session. |
| **shortcut** | A binding that runs a program rather than a command: `{ run = … }` opens a command pane, `{ exec = … }` runs a job. |
| **command pane** | A pane a shortcut or `split{keep}` typed a command into. It closes when the command succeeds and otherwise holds: Enter reruns it, `q` closes it, any other key makes it a plain shell. |
| **job** | An `exec` shortcut running with no pane. The server reaps it on SIGCHLD and puts its result in the status bar. |
| **wait** | A `split` with a `command` whose reply is held until that command finishes, carrying its exit code and output. The output is read at the next prompt, which bounds it. A pane that sends no OSC 133 mark within 10 s gets an error instead. |
| **outer colours** | The real foreground, background and 256-colour palette of the terminal a client runs in. The server asks for them (OSC 10/11/4) at attach and after a light/dark switch, and makes them every pane's defaults so programs that ask get the truth. Chrome and pane content are not recoloured with them. |
| **link id** | A cell's OSC 8 hyperlink, as an index into the server-wide `Links` table, so cells stay `Copy`. Ids are never reused. |
| **grapheme clustering** | Mode 2027: whether a multi-codepoint emoji is one cell or several. Panes mirror the outer terminal's setting, which shroud asks for at attach, because a disagreement shifts the rest of the row. |
| **float** | A pane drawn over a tab's tiles with its own frame, placed in percentages of the tab. Floats sit beside the layout tree, not in it, and show or hide as a group. |
| **overlay** | A box of text a plugin draws over the panes, with no PTY behind it. Owned by the connection that opened it. A modal overlay sends its keys to the owner. |
| **plugin bar** | A status bar drawn by the connection that sent `status_take`. It hears each client's bar state as a `status` event and answers with `status_set`; frames draw the last bar sent, and the built-in bar otherwise. |
| **plugin command** | A `plugin.command` name a connection registers at runtime. Config binds it like a built-in; its suggested key is used only when free. |
| **zoom** | One pane drawn over the whole tab. The others keep their sizes underneath, so nothing reflows; moving focus ends it. |
| **visual mode** | A client's nvim-style cursor over the focused pane's history, for scrolling, searching and copying. Positions are in history rows, counted from the oldest line kept, so new output doesn't move them. A mouse drag in a pane that doesn't use the mouse makes the same selection. |
| **last tab / last session** | Where you were before: the tab left most recently, or the other session entered most recently. Times rather than indexes, so closing tabs can't point them at the wrong one. |
| **fixed count** | A count written into a binding (`"goto-tab 3"`); it beats a typed one. |
| **layout parser** | `shroud __layout`: reads a layout file on stdin and writes the parsed tabs as JSON. The server and `shroud config check` kill it after 1 s. |
| **plugin host** | `shroud __plugins`: the one child process of the server that runs every Lua plugin, with LuaJIT's JIT on. It proves itself with a token from its environment. |
| **plugin** | A directory with `plugin.toml` (name, version, the API it needs, its commands and their keys) and `init.lua`, which returns `{ setup = function(opts) end }`. Config lists plugins as `[[plugins]]`. |
| **init.lua** | `~/.config/shroud/init.lua`: the user's own plugin, named `user`, run in the plugin host. It can declare other plugins with `shroud.plugins{…}`, whose specs replace config.toml's entries for the same source. |
| **built-in plugin** | A plugin shipped inside the binary: `hints` (on by default) and `sessions` (off). It loads exactly as an outside plugin does. Config lists `builtin:<name>` to configure one, with `enabled = false` to turn it off. |
| **hints** | The built-in plugin that, 300 ms into a leader sequence, shows what each next key does, and follows each key after that. |
| **lockfile** | `plugins.lock` beside `config.toml`: the exact commit of each git plugin. `sync` installs at it; only `update`, or changing a plugin's `version`, moves it. |
| **install directory** | `$XDG_DATA_HOME/shroud/plugins/<repo>`, where `sync` clones a `github:` or `git:` plugin. Two sources with the same repository name are refused rather than sharing one. |
| **watchdog** | The rules for when the host dies: a handler busy for 2 s gets the host killed and that plugin turned off until the config changes; three crashes a minute with no plugin to blame turn plugins off. |
| **running mark** | `plugin_running`, sent by the host before and after every slice of Lua so the server knows whom to blame. Waiting for a reply doesn't count as running. |
| **key pending** | A leader sequence that has paused for 300 ms, reported so a key-hint plugin can show what comes next. |
| **chrome** | Everything shroud draws itself: borders, the status bar, overlays and badges. Only chrome is themed. |
| **token** | A semantic colour name such as `border.active`, which the theme resolves to a `Color`. |
| **Color** | A colour reference: the terminal default, a palette index or RGB. It is emitted as-is so that Ghostty's theme applies. |
| **outer terminal** | Ghostty: the real terminal that the client runs in. |
