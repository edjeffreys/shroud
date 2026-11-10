# shroud design

This document explains how shroud is built and why: the decisions, the alternatives they beat, and the constraints behind them. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is the map of where things are in the code; this is the reasoning behind it.

## 1. Goals

shroud is a terminal multiplexer for modern terminals, built on libghostty-vt and optimised for working locally across many projects in Ghostty. It aims to be fast enough that you forget it's there.

### Pillars

- **Performant.** Damage-tracked frame diffing, synchronized output and backpressure, with performance floors checked in CI.
- **Local-first.** Session and layout management come before remote persistence.
- **Themeable from the start.** Every colour shroud draws comes from a named token, and the default theme uses only the terminal's own palette, so shroud follows the user's Ghostty theme (§4, Theming).
- **Extensible, like nvim.** The core is small and unopinionated and exposes primitives (panes, floats, overlays, keys, events). Features that are a matter of taste, such as key hints or saving sessions, ship as built-in plugins written against the same API a third party uses (D10).

### What sets it apart

- **Shell integration that survives the multiplexer.** shroud tracks OSC 133 prompt and command marks and the OSC 7 working directory for every pane. That gives prompt jumping, copying a command's output, new panes in the current directory, and a badge when a command fails or finishes in a pane you aren't looking at.
- **Session management built in.** Create, switch, rename and close named sessions from the CLI, leader keys or a fuzzy switcher. Sessions are named by the user; nothing is inferred from the directory or from tools such as zoxide, which proved too opinionated.
- **Stability as a feature.** If the server crashes, the client restores the terminal, says what happened and exits; Ghostty is never affected. Config mistakes are reported on screen and never crash anything. Every decoder that reads untrusted bytes is fuzzed.
- **Scriptable.** A CLI and a socket API plus an event stream, so scripts, agents and plugins drive shroud through the same commands.
- **Rendering that matches Ghostty**, because both use libghostty-vt: widths, graphemes, the kitty keyboard protocol, OSC 8 links, OSC 52, images and synchronized output.

### The constraint every multiplexer shares

A multiplexer that draws through the outer terminal parses output twice: once itself, then again in the terminal. libghostty-vt makes the first parse fast and accurate but can't remove the second. The performance work therefore goes into sending the outer terminal as little as possible: diffed frames, synchronized output and a capped frame rate.

## 2. Foundations

### Rust, binding libghostty-vt's C API (D1)

libghostty-vt is written in Zig, but what it promises to keep stable is its **C API**, ABI-stable for all of 1.x. Its Zig module may change without warning. Since the C API would be bound either way, the language was free, and shroud uses Rust:

- **Language stability.** Zig changes between releases, and libghostty-vt pins the exact Zig version.
- **Libraries:** `kdl`, `serde_json`, `mlua`, `cargo-fuzz`.
- **Compile-time memory safety**, which backs up stability as a feature.
- **Familiarity** for contributors, human or agent.

The cost is two toolchains: Zig builds the library, Rust builds everything else. All the `unsafe` FFI lives in `shroud-vt`.

**Community bindings ([libghostty-rs](https://github.com/Uzaaft/libghostty-rs)): not yet.** They offer a safe API tested with Miri and sanitizers, and shared maintenance. But at the time of writing they pin a Ghostty commit from before `vt-1.0.0`, their crates.io release lacks callbacks shroud needs, and their 0.x API changes often, so shroud's stability would depend on their release pace rather than on Ghostty's stable ABI. Revisit when a crates.io release builds on Ghostty 1.x; shroud was ported to it once as an experiment, so switching is mostly mechanical.

| | Version | Notes |
|---|---|---|
| Rust | stable, edition 2024 | MSRV in `Cargo.toml` |
| libghostty-vt | 1.0.0 (tag `vt-1.0.0`) | built from the release tarball by `crates/vt/build.rs` |
| Zig | 0.17.x | only to build libghostty-vt; pinned by its `build.zig.zon` |

### How libghostty-vt gets built

`crates/vt/build.rs` downloads the release tarball, **checks its SHA-256** against the value pinned in `build.rs` (taken from the release's signed `SHA256SUMS`), runs `zig build -Doptimize=ReleaseFast`, and links the result statically along with the C++ runtime. The build is cached in `target/` across profiles.

The release tarball matters: it carries the `LIB_VERSION` file that switches the build into library mode, which bundles the SIMD dependencies (simdutf, highway) into one archive. A GitHub archive of the tag would build the Ghostty app instead.

`SHROUD_LIBVT_DIR=<prefix>` skips the download and the Zig build, for offline builds and packagers.

**The bindings are generated once and committed** (`crates/vt/src/ffi.rs`, by `scripts/gen-bindings.sh`). The ABI is stable, so they only need regenerating to use API a newer 1.x adds, and normal builds need neither bindgen nor libclang.

### What libghostty-vt provides, and what shroud adds

| Need | libghostty-vt | shroud |
|---|---|---|
| Parse pane output | `ghostty_terminal_vt_write` | feed it PTY bytes |
| Answer DA, DSR, XTVERSION, size and mode queries | the `WRITE_PTY` effect | write replies back to the PTY |
| Titles, cwd, OSC 52, notifications, bell | effect callbacks | forward them to clients as outer-terminal sequences |
| OSC 133 marks, OSC 7 cwd | the `SEMANTIC_PROMPT` and `PWD_CHANGED` effects | feed the pane's `shell::Tracker` |
| Synchronized output inside a pane | the `RENDER_HOLD` effect | freeze the pane's mirror while it lasts, with a 1 s timeout |
| Which rows changed | render-state dirty flags | copy only dirty rows |
| Widths and graphemes | cells carry graphemes and wide flags | trust them; use `ghostty_unicode_*` for chrome |
| Encode keys, mouse, focus, paste for a pane | encoders synced from the terminal | build events, sync, encode |
| Decode input from the outer terminal | nothing | `core/input/decode.rs` |
| Scrollback, compression, text formatting | yes | drive them from the mouse, visual mode and the API |
| Kitty graphics inside a pane | stores images, lists placements | re-send them to the outer terminal (§7) |

## 3. Architecture

### Process model (D2)

There is **one server per user**. It holds every PTY, every terminal and every layout. **Clients are dumb**: a client puts the outer terminal into raw mode, forwards input bytes and resizes, and writes whatever frame bytes the server sends.

The server composes and diffs a frame per attached client. So reattaching is "this client has no previous frame", which forces a full redraw from libghostty-vt's state with nothing replayed; the client stays around 200 lines and can't drift out of sync; and decoding input on the server means the client never needs to know about keybinds.

```
Ghostty ⇄ client (raw tty) ⇄ unix socket ⇄ server ⇄ PTY ⇄ shell / nvim / htop
```

Two kinds of helper process keep risky work out of the server:

- **The plugin host**, `shroud __plugins`, runs every Lua plugin (§6).
- **The layout parser**, `shroud __layout`, parses a layout file and is killed after 1 s (§5).

### One thread (D3)

The server is **single-threaded**: one `poll(2)` loop over the listening sockets, client sockets, PTY masters and a signal pipe, plus one timer. No locks, and libghostty-vt's two-phase render update isn't needed. If profiling ever shows a flooding pane starving others, PTY reads and `vt_write` can move to a reader thread per pane behind the same `Pane` interface. Nothing so far has needed it: no async runtime, no threads in the server.

### Crates and layers

```
shroud (bin)     CLI, server/, client/, pty                      ──▶ shroud-vt, shroud-plugins, shroud-core
shroud-vt        the only code that calls libghostty-vt          ──▶ shroud-core
shroud-plugins   LuaJIT via mlua, run only in the plugin host    ──▶ shroud-core
shroud-core      pure logic and plain data, #![forbid(unsafe_code)]
shroud-smoke     end-to-end checks; drives the binary from outside
```

The crate graph enforces the coarse layering. Inside the crates, `crates/core/tests/layers.rs` checks every `use crate::…` against an allowed-edges table and fails if a colour value appears outside the theme, on every `cargo test`.

- **Pure** means no syscalls, no C and no `unsafe`. Core functions write into buffers the caller passes in (`&mut Vec<u8>`), so allocation is visible at the call site, and all of core is unit-tested without I/O.
- **`render/` never sees a pane** (panes copy their screen into a `render::Grid`), and **`layout/` never sees a PTY** (trees hold `PaneId`s).
- **Encoding lives in `shroud-vt`**: it is one C call per event, and keeping every C call in one crate keeps `input/` pure.
- **Pacing, shell integration and the theme are their own modules**: each is pure policy or data that several callers read and that tests drive with fakes.

Dependencies are deliberately few: `rustix` for PTYs, termios, `poll` and signals; `signal-hook` for a self-pipe; `serde_json`, `toml` and `kdl` for formats; `unicode-width`; `libc` for `localtime_r`; `mlua` with vendored LuaJIT; `insta` for golden frames.

## 4. Key mechanisms

### Output path

1. The loop reports a readable PTY master. The pane reads up to 64 KiB per wake, so one pane can't monopolise the loop, and calls `vt_write`.
2. Effects fire during that write. Query replies are queued for the PTY; titles, clipboard, notifications and bells are queued for clients; shell-integration marks update the tracker. **Callbacks only enqueue**, because libghostty-vt isn't re-entrant.
3. The pane is marked dirty and the pacer decides whether to render now.
4. At render time, each dirty pane outside a render hold copies its dirty rows into its `Grid`.
5. For each client, `compose` builds a frame from the layout, pane grids, borders, floats, overlays and the status bar, and `diff` turns the previous frame and this one into bytes: cursor moves only when the next changed cell isn't adjacent, SGR only when the style changes, wide cells written once, all wrapped in `CSI ? 2026 h … l`. Rows whose compose key (layout, theme and the generations of the pane rows drawn there) matches the previous frame's are skipped without comparing cells.

### Frame pacing and backpressure

**Backpressure is the main control.** A client whose socket still holds unsent bytes gets no new frame; pane state keeps advancing, and once the socket drains the client gets one current frame. `cat` of a huge file costs one parse and a few dozen frames, not a frame per read.

The pacer adds a **rate cap**: 120 fps, dropping to 30 fps once frames have run back to back for 250 ms, recovering after 250 ms idle. Two refinements keep typing fast:

- **A pane's first output after a key is expedited**: it renders at once and doesn't count towards flooding. Without this, fast typing tripped flood mode and every echo waited up to 33 ms.
- **Only frames that send bytes count against the pacer**, and a key that only goes to a pane doesn't redraw by itself; its echo brings its own frame.

### Input path

1. On attach, the client enables the alternate screen, kitty keyboard flags (`CSI > 31 u`), SGR any-event mouse, bracketed paste and focus events, and undoes them all on detach, exit or a signal.
2. Raw bytes go to the server, where a decoder turns them into events. It keeps partial sequences across reads; a lone ESC is safe to treat as a key at once because, under kitty's all-keys flag, the real Escape key arrives as `CSI 27 u`.
3. The keybind state machine checks each key against the bindings: leader, direct key, pending sequence, or passthrough.
4. Passthrough keys are encoded in whatever mode the focused pane's program asked for. **Modifiers that produced the text count as consumed:** on macOS, Option composes characters (`#` is Option+3 on a UK layout) and Ghostty still reports Alt with them, so a kitty-mode pane would otherwise get Alt+3 rather than `#`. Alt and Shift are consumed when the text isn't the key's own character, so an Option set to act as Alt, which sends the plain character, still arrives as a chord. Mouse events are hit-tested against the layout; a pane that tracks the mouse gets them translated to its coordinates, and otherwise the wheel scrolls and a drag selects. Pastes go through libghostty-vt's paste encoder, which applies bracketed paste.

### Keys (D4, D13, D14)

The leader is **`Ctrl+Space`**, and the keys after it follow **nvim's `CTRL-W` grammar**, so managing panes feels like managing nvim windows and `:help CTRL-W` settles edge cases. Keys nvim leaves free take shroud's own commands (tabs, sessions, floats, visual mode), using tmux's keys where tmux has them (`$`, `(`, `)`, `,`). As in nvim, an unbound `Ctrl+<letter>` or arrow after the leader means the plain letter.

- **Every command has a default key, and every key is rebindable**: a command without a key isn't accessible. Plugin commands suggest defaults too, used only when free, so installing a plugin never silently changes a key.
- **`Ctrl+Space 1`–`9` go to tabs, so counts start with `0`** (`03w`, `010+`). This beat dropping counts or waiting on a timer. Unbinding a digit makes it start a count again.
- **Keys bound to a prefix of a sequence are reported** as problems rather than resolved by a timeout as nvim does.
- **Jump mode's `[` / `]` and the leader-twice escape are grammar, not bindings.**
- **The live table is rebuilt, not edited**: config's bindings, then plugin defaults where free, then runtime binds from the API or Lua (latest wins, lasting while their connection does). Removing a bind can't leave a stale key or lose the config binding it covered.

### Theming

Theming applies to **shroud's own chrome only**: borders, the status bar, the switcher, badges and overlays. **Pane content is never recoloured.**

- **Colours are references.** `Color` is the terminal default, a palette index or RGB, and each is emitted as itself (`39`, `38;5;n`, `38;2;r;g;b`). Nothing is resolved to RGB on the way out, so Ghostty applies the user's theme and follows theme changes with no queries.
- **Pane cells keep the colours their program wrote**: the snapshot copies each cell's style colours, not libghostty-vt's resolved values.
- **Tokens are struct fields** (`border_active`, config key `border.active`), so a typo in code fails to compile. The default theme uses only `default` and palette 0–15.
- **Plugins can bring their own colours** for spans in the bar and overlays; colour parsing stays in `theme.rs`.
- Derived shades (dimmed borders, blends) were left out as a matter of taste that a theme plugin could add.

### Real terminal colours

Pane programs ask about colours too: nvim sends OSC 11 to choose a light or dark background. libghostty-vt answers from the default colours its terminal was given, and with none it doesn't answer at all. So at attach, and again whenever Ghostty reports a light/dark switch (mode 2031), the server asks the client's terminal for OSC 10, 11 and all 256 OSC 4 entries and makes them every pane's defaults; pane programs then get truthful answers. The palette is applied only once all 256 entries arrive, since setting it replaces every default. Mode 2031 reports are forwarded to panes that enabled it.

### Resize, detach and reattach

- A resize recomputes rects; each pane whose grid changed gets a terminal resize and a PTY resize (SIGWINCH). Pixel sizes are passed through so `CSI 14 t` is answered truthfully. The PTY isn't resized when only the cell size changed, since a needless SIGWINCH interrupts a read in progress. A resize also ends a pane's render hold at once.
- Detaching tears down the client's terminal; PTYs keep running. Attaching is a `hello` and a full redraw.
- If the server dies, the client sees EOF without an `exit` message, restores the terminal, prints `shroud: server exited unexpectedly (log: …)` and exits 1. The client never panics on bad input.

### The server's lifecycle

- `shroud` connects to the socket; on `ENOENT` or `ECONNREFUSED` it removes a stale socket, starts `shroud __server` (setsid, output to the log) and retries for up to 2 s.
- **The runtime directory** is `$XDG_RUNTIME_DIR/shroud` where the system provides one (private to the user and cleared at logout, as on most Linux systems), else `$TMPDIR/shroud-$UID` (per-user on macOS), else `/tmp/shroud-$UID`, also used when a path is too long for a socket (`sun_path` is 104 bytes on macOS). A client must see the same variables as the server to find it, as with tmux's `TMUX_TMPDIR`; panes inherit them. It is created `0700`, refused if it is a symlink or owned by someone else, and tightened if its mode is loose, because anyone who can reach the socket can type into every pane.
- The server outlives its windows on purpose and exits when its last session ends. SIGTERM and SIGHUP shut it down.
- **`shroud server restart`** can't keep shells, which belong to the server, but keeps windows: each client waits for the old server to release the socket and then runs the binary from disk again, so a rebuild takes effect.

### Protocols (D8)

There are two wire formats, one per audience.

**Client ⇄ server: a small binary protocol.** Every message is `u8 tag | u32 length | payload`, little-endian, and decoding never allocates. The handshake goes first: `hello{version, size, term, which session}` → `welcome`, or `exit(version_mismatch)` so an old client prints a clear "restart the server" rather than misreading bytes. `output` carries a rendered frame exactly as the outer terminal should see it. Bump `VERSION` whenever an old peer would misread a message.

**Scripting: NDJSON on a separate `control.sock`.** One JSON object per line each way; a request's `id` is echoed in its reply; errors are `{"ok":false,"error":"…"}` and never close the connection. A separate socket keeps the binary hot path from ever sniffing formats. Key order is preserved so replies read `id`, `ok`, then the payload. Request lines are length-limited, and a subscriber with 4 MiB unread is dropped rather than buffered without limit.

### Scripting semantics

- **Target pane:** `-p`, else `$SHROUD_PANE` (set in every pane), else the focused pane of the most recent session. A script run inside shroud acts on its own pane.
- **New panes don't take focus** unless asked, so a script never moves the user's cursor.
- **`split{command}` types the command into the new shell**, as layouts do, so the shell's environment and aliases apply.
- **`wait`** holds the reply until the command finishes and carries its exit code and output. The output is read at the next prompt, which bounds it (zsh's integration sends the finish mark before drawing the prompt). A pane that sends no OSC 133 mark within 10 s gets an error rather than a hang.
- **`run`** executes any bindable command as if a client pressed its key, so anything a key can do, a script or plugin can do. Commands that belong to a client pick one on the session, and fail when none is attached rather than guessing.
- **Events** for state (focus, layout, tabs, sessions, clients, mode) come from comparing snapshots each loop, and only while someone subscribes, so every way a state can change is covered by one rule and nothing is computed otherwise. Effect-driven events (bell, title, cwd, `output_started`) cost nothing extra.

## 5. Shell integration and config

### Shell integration in panes (D5, D6)

Ghostty doesn't load its integration into nested shells, so shroud injects Ghostty's own scripts into each pane the way Ghostty does (for zsh, pointing `ZDOTDIR` at the integration and passing the original along). It does so only when `GHOSTTY_RESOURCES_DIR` is set and ships no shell scripts of its own. Injection covers zsh and fish; Ghostty injects bash by restarting it with extra arguments, which doesn't carry over, so bash users source the script from `.bashrc`.

- **Panes get `TERM_PROGRAM=ghostty`.** Their programs talk to libghostty-vt, Ghostty's own parser, whatever the outer terminal is, and some gate features on that name: Claude Code only uses synchronized output for terminals it recognises, and tore its redraws otherwise. Dotfiles detect shroud by `$SHROUD`. `TERM_PROGRAM_VERSION` and an outer multiplexer's `TMUX*` / `ZELLIJ*` variables are removed. `TERM=xterm-ghostty` is passed only when its terminfo resolves.
- **OSC 7** is accepted as `file://` and as Ghostty's `kitty-shell-cwd://` form.
- **Badges need a command that ran for at least a second**, so a stray `ls` in a background pane stays quiet. A failure badge stays until the pane is focused.
- **Exit codes by prompt.** libghostty-vt reports exit codes but doesn't keep them, so each pane keeps the last 500 keyed by prompt row and command text; rows shifted by trimmed history can't pair a command with another's code.
- **Copying output** uses libghostty-vt's selection and formatter, so the text is what Ghostty itself would copy. A command's output is bounded by prompt rows, since glyphs printed before the first mark would otherwise count.

### Config: TOML for settings, KDL for layouts

Settings are flat (options, theme tokens, key tables), which TOML handles well and everyone knows. Layouts are trees of splits, which TOML expresses badly and KDL naturally. Pkl was rejected because Rust can't evaluate it without running a separate program.

- **A mistake costs only the setting it's in.** Core parses text into a `Config` plus problems with line numbers; each problem keeps that setting's default. A syntax error keeps the config already running, so a half-saved file never resets keybinds.
- **Hot reload by `stat` once a second.** A kqueue or inotify watch would have to follow editors that replace the file rather than write it.
- **`leader` is a top-level key** because TOML can't make `keys.leader` both a string and a table.
- **Layouts load only from the session's starting directory** (then the global file), never a parent, so nothing is picked up by surprise.

### Layout files are parsed in a child

Fuzzing found that the `kdl` crate's error recovery on malformed input re-parses a failed node's children once per alternative: roughly 2.75^depth × length. A 238-byte file ran for over 10 s with memory growing, and the crate offers no way to parse without recovery. So:

- **Limits first:** a file over 16 KiB or nested more than 8 deep is refused before anything parses it.
- **Then a child:** the server and `shroud config check` pipe the text to `shroud __layout`, which replies with JSON, and kill it after 1 s. A thread couldn't be stopped, and a bad file grows memory without bound. The session then starts with one pane and says why.
- Starting a session with a bad layout can therefore block the server for up to 1 s; making session creation asynchronous would defer every caller for a rare case. Writing our own parser for the subset was judged not worth it.

## 6. Plugins

### LuaJIT, in a host process (D11, D12)

Plugins are Lua. One language makes a package manager simple (plugins are source, with no build step or per-platform binaries), and Lua is what nvim users already write. Separate processes per plugin, Luau, Rhai and WebAssembly were considered. Rust plugins as shared libraries were ruled out: there's no stable ABI and loading them is unsafe. Heavy or stateful integrations can be separate programs on `control.sock`, which uses the same command and event names as the Lua API.

**LuaJIT can't interrupt JIT-compiled code.** Tested with mlua and LuaJIT 2.1: an instruction-count hook stopped an infinite loop with the JIT off but never fired with it on. LuaJIT's `ffi` can also crash whatever process runs it. So **plugins never run in the server**:

- `shroud __plugins` is one child process running every plugin, with the JIT on. It talks to the server over `control.sock` like any script; it is a privileged client, not a special case, and proves itself with a token from its environment.
- **The watchdog** (`core/watchdog.rs`): the host sends `plugin_running` before and after every slice of Lua, without waiting for a reply. A handler still running after 2 s gets the host killed and restarted and that plugin turned off until the config changes; a crash while a plugin runs blames it; three unattributed crashes in a minute turn plugins off.
- **Nothing synchronous.** No plugin hook can delay a key, a frame or pane output. Plugins see coarse events and make requests; they never see output byte by byte or individual frames. A slow plugin costs at most a stale overlay or bar.
- **Trust is nvim's:** a plugin runs as you, with `io`, `os` and `ffi`. LuaJIT can't be sandboxed with `ffi` present, so there's no pretence of one; `plugins.lock` makes what runs reviewable and repeatable.

### The API

- **Requests look synchronous.** Each handler runs in its own coroutine that yields until its reply, so a plugin writes `local id = shroud.split{dir = "down"}`. The host records whose coroutine is running, so modules a plugin `require`s need no handle passed in.
- **Primitives the core provides for plugins:** floating panes; overlays (styled text boxes, optionally modal, where Escape always closes so a broken plugin can't trap the keyboard); plugin commands with suggested keys; coarse events; layouts in and out (`layout`, `apply_layout`, `set_layout`); session requests; pane tags; a plugin-drawn status bar; `message`; and `bind`.
- **`shroud.system`** follows nvim's `vim.system`. Each job is waited on by its own thread, which holds no Lua and wakes the host's loop through a socketpair, so Lua stays single-threaded. Each job is its own process group and signals go to the group, so a timeout reaches children that hold the pipes. `io.popen` would block every plugin.
- **`shroud.keymap`** follows `vim.keymap.set`. A function becomes a command of its plugin, so hints and `bindings` can name it, and the server needs nothing Lua-specific.
- **A plugin-drawn status bar** is state out, spans back, never waiting: the server sends each client's bar state when it changes and draws the last bar the plugin sent. Until one arrives, and as soon as the owner goes away, the built-in bar draws.
- **`set_layout`** rearranges a tab in place so tiling layouts can be plugins. Every tile must be placed, since dropping one by omission would close a program by accident; nothing changes until the whole layout checks out.
- **Tags** are short strings a plugin keeps on a pane (tmux's `@user` options), limited in count and size so a buggy plugin can't bloat every `list` and saved session.
- Definitions for lua-language-server ship in the binary (`shroud plugin types`), and a test checks every API function has one.

### Packages

- A plugin is a directory with `plugin.toml` (name, version, the API version it needs, its commands and their suggested keys) and `init.lua` returning `{ setup = function(opts) end }`. Commands live in the manifest so they register before any Lua runs, and a broken `init.lua` still leaves its keys saying what's wrong.
- **`plugin sync` uses `git`**, since pins to tags, branches and commits need it, and **git runs in the CLI, never the server**, so the network can't stall a session. Commits are pinned in `plugins.lock`. git runs with `GIT_TERMINAL_PROMPT=0`; versions go after `--end-of-options` and the lockfile accepts only hex hashes, so neither can smuggle in an option. Two sources with the same repository name are refused rather than sharing a directory.
- **`config.toml` keeps what the server needs instantly and safely** (keys, theme, shell, simple `[[plugins]]`). **`init.lua` is a plugin named `user`**, run in the host with the whole API, and can declare plugins with functions in their options. The CLI reads init.lua's specs by running it in a child with requests going nowhere, killed after 1 s.
- **Built-in plugins** (`hints`, on by default because it makes keys discoverable; `sessions`, off because restoring layouts is a matter of taste) are compiled into the binary and load exactly as outside plugins do, so they can be configured, turned off or replaced.
- `sessions` saves layouts with each pane's directory and foreground program. On restore it types the program at the prompt without Enter, so you choose whether to rerun it. It never saves an empty list, so the last shell exiting can't erase what a restart needs.

## 7. Other features worth explaining

### Command shortcuts (D15)

A binding can be a table instead of a command name: `{ run = "nvim" }` opens a command pane, `{ exec = ["git", "pull"] }` runs a job with no pane. One parser (`core/input/shortcut.rs`) serves config, the API and Lua, so they accept the same keys and give the same errors. Shortcuts have no default keys: D13 is about shroud's own commands, and a shortcut exists only because a user wrote it.

- **`run` types into a new pane's shell**, so the shell's environment applies. **A command pane closes when its command succeeds and holds when it fails**: closing on success suits programs you quit on purpose, while a failing test stays visible. `keep = true` always holds. While held, Enter reruns, `q` closes and any other key makes it an ordinary shell, so a held pane never traps you.
- **`exec` runs in the server, single-threaded.** The child gets its own process group, `/dev/null` for input and an unlinked file for output (so a chatty command can't fill a pipe nobody reads), is reaped on SIGCHLD, and has a timeout that is one more deadline for `poll`. The result goes in the status bar.

### Floats, zoom and frames

- **Floats** sit beside the layout tree, not in it, placed in percentages of the tab so they survive resizes, and show or hide as a group. Directional focus from a float moves between floats, and past the last one returns to the tiles. A tab always keeps a tile.
- **Zoom** draws one pane over the tab and resizes it; the others keep their sizes underneath, so nothing reflows.
- **Pane frames** (off by default) put the tiles inside one frame around the tab, so every pane has a border line above it for its title. This costs less than a frame per pane and falls out of the existing border drawing.

### Scrollback

`scrollback` sets megabytes per pane, defaulting to Ghostty's 10 MB (libghostty-vt's own default keeps about one page). History is kept in pages of full-width rows, so cost tracks columns × lines. Once a pane has been quiet for 2 s, the server compresses its history with libghostty-vt in steps of at most 1 ms per loop pass, so input never queues behind it; at 200 columns, 10 MB of history compresses to about 2 MB.

### Visual mode

An nvim-style cursor over a pane's history, for scrolling, searching and copying. The core holds the cursor and selection in history rows (counted from the oldest kept, so new output doesn't move them) and reads text through a trait, so motions, search and text objects are unit-tested without a terminal. Search reads the whole history through libghostty-vt's formatter (100,000 rows in about 20 ms), which makes incremental search affordable. The selection is drawn by inverting cells and matches by underlining, so pane colours are never changed. A mouse drag in a pane that doesn't use the mouse makes the same selection.

### Links, widths and images

- **OSC 8 links** survive the redraw: each linked cell's URI is interned in a server-wide table and the id stored on the cell, so cells stay `Copy`. Ids are never reused, so stale ones in old frames are harmless.
- **Widths:** panes use the outer terminal's grapheme-clustering mode (2027), read at attach, because a disagreement about `👍🏽` shifts the rest of the row. As a backstop, `diff` positions the cell after any multi-codepoint cluster explicitly.
- **Images:** libghostty-vt implements the Kitty graphics protocol in each pane and lists placements in cells. shroud re-sends each image to each client once per change under an id that client chose, diffs placements frame to frame, and clips them to their pane in pixels. Kitty can't cut a hole in an image, so an image is hidden while a float or overlay covers it. Every command carries `q=2`, since a reply would arrive at the client as typed keys.

## 8. Testing

- **Unit tests** sit next to the code, and core's are the bulk: layout, keybinds, decoders, protocol round trips, the pacer with a fake clock, visual mode, config.
- **Golden frames** are `insta` snapshots of the exact bytes `compose` → `diff` produce, with escapes made visible. Review the bytes; don't accept blindly.
- **vt tests** feed known bytes into a real terminal and check the grid, replies, encoders and history.
- **`cargo test` never spawns a PTY or opens a socket**, so it stays fast and deterministic.
- **The smoke test** (`crates/smoke`) drives the real binary through PTYs, one isolated TMPDIR and server per section, all in parallel, and reads frames back through libghostty-vt, so its checks look at the screen. Every wait ends as soon as its condition holds; none sleeps a fixed time. It includes the idle-CPU budget (under 1% for an idle server).
- **Fuzzing** covers every decoder that reads untrusted bytes: input, protocol messages, the shell tracker, config and lockfiles, and API requests. The properties are that no input crashes or hangs, that the input decoder gives the same events however bytes are split, and that messages round-trip. `fuzz/` is its own workspace, so nightly never touches the main build; `-s none` turns off AddressSanitizer, since core has no `unsafe` and the bugs worth finding are panics. Crashing inputs become unit tests.
- **Performance floors:** `shroud bench --check` (parse rate, full-frame and one-key-echo frame times) runs in CI on release builds.

## 9. Decision record

| | Decision | Why, in short |
|---|---|---|
| D1 | Rust, binding libghostty-vt's C API | stable ABI, stable language, memory safety (§2) |
| D2 | The server renders; clients are dumb | reattach is a full redraw; nothing to drift (§3) |
| D3 | One thread, one `poll` loop | no locks; a reader thread per pane is held in reserve (§3) |
| D4 | Leader `Ctrl+Space`, then nvim's `CTRL-W` keys | pane management feels like nvim windows (§4) |
| D5 | Capture shell integration in every pane from the start | every feature that needs it has the data (§5) |
| D6 | Inject the outer terminal's shell integration | no dotfile edits; no scripts of shroud's own (§5) |
| D7 | Status bar at the bottom; prompt jumps repeat without the leader | out of the way; jumping feels like nvim's `[[` / `]]` |
| D8 | NDJSON API on a separate `control.sock` | the binary hot path never sniffs formats (§4) |
| D9 | Chrome themed by tokens; pane content never recoloured; real colours passed to panes | follows the user's theme with no queries (§4) |
| D10 | An unopinionated core; taste-driven features are built-in plugins | shroud's own plugins prove the API (§1) |
| D11 | Plugins are LuaJIT; heavier integrations talk to the socket | one language, no build step; nvim users know Lua (§6) |
| D12 | Lua runs in one plugin host process, never the server | JIT loops can't be interrupted; `ffi` can crash (§6) |
| D13 | Every command has a default key; every key is rebindable | a command without a key isn't accessible (§4) |
| D14 | `Ctrl+Space 1`–`9` go to tabs; counts start with `0` | fast tab switching without a timer (§4) |
| D15 | Shortcuts are config, with no default keys; command panes close on success | shortcuts exist only because a user wrote them (§7) |
