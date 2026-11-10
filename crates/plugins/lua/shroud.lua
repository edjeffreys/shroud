---@meta shroud
-- Definitions for lua-language-server. Write them out with `shroud plugin types > shroud.lua`
-- and add that file's directory to `workspace.library`.
--
-- Requests yield the handler that makes them until the reply arrives. Call them from a
-- plugin's own code (`setup`, a command, an event, a timer), not from inside coroutines
-- the plugin created itself.

---@class shroud
---@field api string The plugin API version this host offers, e.g. "0.1.0".
---@field server {pid: integer, started: integer}? The server this host serves; `started` is in Unix milliseconds. Set before plugins load.
shroud = {}

---@alias shroud.event
---| "command_finished" # {pane, exit}
---| "pane_opened"      # {pane, session}
---| "pane_closed"      # {pane}
---| "focus_changed"    # {session, pane}
---| "session_opened"   # {session}
---| "session_closed"   # {session}
---| "session_renamed"  # {from, to}
---| "tab_changed"      # {session, tab}
---| "layout_changed"   # {session, tab}
---| "client_attached"  # {client, session}
---| "client_detached"  # {client}
---| "key_pending"      # {client, session, keys}
---| "key_done"         # {client}
---| "mode_changed"     # {client, mode}: normal, leader, jump or visual
---| "tag_changed"      # {pane, key, value}: value is nil when removed
---| "bell"             # {pane}
---| "title_changed"    # {pane, title}
---| "cwd_changed"      # {pane, cwd}
---| "output_started"   # {pane}: output after 2 s without any, not counting a key's echo
---| "overlay_key"      # {overlay, key}, only for this plugin's overlays
---| "overlay_closed"   # {overlay}, only for this plugin's overlays
---| "status"           # shroud.Status, only after this plugin took the bar

---@class shroud.CommandEvent
---@field name string `plugin.command`
---@field client integer
---@field session string
---@field count integer? A count typed before the key, as in `Ctrl+Space 03 g y`.

---@class shroud.Span
---@field text string
---@field style string? The text colour: a theme token ("badge.done") or a colour ("#82aaff", "4", "default").
---@field bg string? The background, the same way.
---@field bold boolean?
---@field italic boolean?

---@class shroud.Status What one client's status bar should show; sent as the `status` event.
---@field client integer
---@field session string
---@field mode "normal"|"leader"|"jump"|"visual"
---@field selection ("chars"|"lines"|"block")? In visual mode, the kind of selection being made.
---@field zoomed boolean
---@field tabs {name: string?, active: boolean, badge: ("failed"|"done")?}[]
---@field message string?
---@field alert string? "config" or "plugin" when something needs attention.
---@field clock string
---@field cwd string?
---@field branch string?
---@field badges {pane: integer, badge: "failed"|"done"}[]
---@field width integer The client's width in cells.

---@alias shroud.Line string|shroud.Span[]

---@class shroud.Float
---@field x integer? Percent of the tab, default 10.
---@field y integer?
---@field w integer? Default 80.
---@field h integer?

--- A key that runs a program: `run` opens a pane, `exec` runs with none and reports to the
--- status bar. The same table config.toml takes under `[keys.leader]`.
---@class shroud.Shortcut
---@field run string? Typed into a new pane's shell. The pane closes if it succeeds and holds if not.
---@field open ("float"|"right"|"down"|"tab")? Where `run` opens, default "float".
---@field keep boolean? Hold a `run` pane even when its command succeeds.
---@field exec (string[]|string)? A program and its arguments, or a line for sh.
---@field timeout integer? Seconds before `exec` is stopped, default 600.
---@field cwd string? Default the focused pane's directory; `~/` works.

---@class shroud.Pane
---@field id integer
---@field focused boolean
---@field float boolean
---@field cwd string?
---@field title string
---@field running boolean
---@field last_exit integer?

---@class shroud.Tab
---@field active boolean
---@field panes shroud.Pane[]

---@class shroud.Session
---@field name string
---@field tabs shroud.Tab[]

---@class shroud.LayoutNode
---@field split ("right"|"down")?
---@field parts shroud.LayoutNode[]?
---@field size number? Percent of the parent split.
---@field cwd string?
---@field command string?
---@field pane integer? Reported only.
---@field title string? Reported only.

---@class shroud.Layout
---@field root shroud.LayoutNode
---@field floats (shroud.LayoutNode|shroud.Float)[]?

---@class shroud.PluginSpec
---@field [1] string The source: "github:owner/repo", "git:<url>", "path:<dir>" or "builtin:<name>".
---@field version string? A tag, branch or commit, for git sources.
---@field opts any? Handed to the plugin's `setup`; may hold functions, unlike config.toml's.
---@field config fun(plugin: table, opts: any)? Called instead of `setup(opts)`, with what the plugin's init.lua returned.
---@field enabled boolean? false turns it off, including a built-in on by default.

--- Declares plugins, nvim-style, from `~/.config/shroud/init.lua`. A spec replaces any
--- `[[plugins]]` entry in config.toml with the same source.
---@param specs (string|shroud.PluginSpec)[]
function shroud.plugins(specs) end

--- Runs `handler` in its own coroutine whenever the event fires.
---@param event shroud.event
---@param handler fun(ev: table)
function shroud.on(event, handler) end

--- Handles `<plugin>.<name>`. Declare it in plugin.toml's `[commands]` to give it a default key.
---@param name string
---@param handler fun(ev: shroud.CommandEvent)
function shroud.command(name, handler) end

---@class shroud.SystemResult
---@field code integer? nil when a signal ended it
---@field signal integer?
---@field stdout string
---@field stderr string

---@class shroud.SystemJob
---@field pid integer
local SystemJob = {}

--- Yields the handler until the program ends. Not for a job started with `on_exit`.
---@return shroud.SystemResult
function SystemJob:wait() end

--- False once the job has ended.
---@param signal ("term"|"kill"|"int"|"hup")? default "term"
---@return boolean
function SystemJob:kill(signal) end

--- Starts a program without blocking, like nvim's `vim.system`. With `on_exit`, that runs as
--- this plugin when it ends; otherwise call `:wait()`. Output is text; invalid UTF-8 is replaced.
---@param cmd string[] the program, then its arguments; no shell unless you start one
---@param opts {cwd: string?, env: table<string, string>?, stdin: string?, timeout: integer?}? timeout in ms, then SIGTERM
---@param on_exit fun(result: shroud.SystemResult)?
---@return shroud.SystemJob
---@overload fun(cmd: string[], on_exit: fun(result: shroud.SystemResult)): shroud.SystemJob
function shroud.system(cmd, opts, on_exit) end

--- Binds keys from Lua, like nvim's `vim.keymap.set`. `keys` is a sequence after the leader
--- (`"g y"`), or with `direct` one key that acts without it. `target` is a command
--- (`"zoom"`, `"goto-tab 3"`, `"sessions.save"`), a shortcut table as in config.toml
--- (`{ run = "nvim" }`, `{ exec = { "git", "pull" } }`), a function, or nil to unbind. A function
--- becomes the command `<plugin>.<name>`, `keymap-N` unless named. Binds sit over config.toml and
--- plugins' default keys, and the latest wins.
---@param keys string
---@param target string|shroud.Shortcut|fun(ev: shroud.CommandEvent)|nil
---@param opts {direct: boolean?, name: string?}?
function shroud.keymap(keys, target, opts) end

--- Runs `fn` once, `ms` milliseconds from now.
---@param ms integer
---@param fn fun()
function shroud.after(ms, fn) end

--- Writes to shroud's server log, prefixed with the plugin's name. `print` does the same.
---@param message string
function shroud.log(message) end

--- This plugin's own directory for state, created on first use.
---@return string
function shroud.data_dir() end

--- Any control.sock request, by its `cmd`. Returns the reply without `ok` and `id`.
---@param req {cmd: string, [string]: any}
---@return table? reply
---@return string? err
function shroud.request(req) end

---@return shroud.Session[]? sessions
---@return string? err
function shroud.list() end

--- Opens a pane; `pane` defaults to the focused one. With `close_on_exit` the pane closes
--- when `command` exits, like a popup. With `keep` it holds instead: Enter runs the command
--- again and `q` closes the pane.
---@param opts {pane: integer?, dir: ("right"|"down")?, cwd: string?, command: string?, focus: boolean?, float: shroud.Float?, close_on_exit: boolean?, keep: boolean?}?
---@return integer? pane
---@return string? err
function shroud.split(opts) end

---@param opts {pane: integer?, input: ({text: string}|{key: string})[]}
---@return table? ok
---@return string? err
function shroud.send(opts) end

--- The pane's last finished command output.
---@param opts {pane: integer?}?
---@return string? text
---@return string? err
function shroud.output(opts) end

---@param opts {pane: integer?}?
---@return table? ok
---@return string? err
function shroud.close(opts) end

--- The pane's text, unwrapped as Ghostty would copy it: the screen, or with `history`
--- everything kept.
---@param opts {pane: integer?, history: boolean?}?
---@return string? text
---@return string? err
function shroud.capture(opts) end

--- Any command a key can be bound to (`"zoom"`, `"goto-tab 3"`, `"sessions.save"`, or a
--- shortcut table like `{ exec = { "git", "pull" } }`), as if `client` pressed it. Without
--- `client`, a client on `session`, or the most recent session.
---@param command string|shroud.Shortcut|{command: string|shroud.Shortcut, client: integer?, session: string?}
---@param opts {client: integer?, session: string?}?
---@return boolean? ok
---@return string? err
function shroud.run(command, opts) end

---@param opts {pane: integer?}?
---@return table? ok
---@return string? err
function shroud.focus(opts) end

--- A modal overlay gets this plugin's `overlay_key` events; Escape always closes it.
---@param opts {lines: shroud.Line[], title: string?, anchor: ("centre"|"top_left"|"top_right"|"bottom_left"|"bottom_right"|"cursor")?, modal: boolean?, session: string?, client: integer?}
---@return integer? overlay
---@return string? err
function shroud.overlay_open(opts) end

---@param opts {overlay: integer, lines: shroud.Line[], title: string?, anchor: string?, modal: boolean?}
---@return table? ok
---@return string? err
function shroud.overlay_update(opts) end

---@param opts {overlay: integer}
---@return table? ok
---@return string? err
function shroud.overlay_close(opts) end

---@return {leader: string, after_leader: {keys: string, command: string}[], direct: {key: string, command: string}[]}? bindings
---@return string? err
function shroud.bindings() end

--- `tab` is 1-based and defaults to the active one.
---@param opts {session: string?, tab: integer?}?
---@return shroud.Layout? layout
---@return string? err
function shroud.layout(opts) end

--- Builds a new tab from a layout without switching to it.
---@param opts {session: string?, layout: shroud.Layout}
---@return {tab: integer}? reply
---@return string? err
function shroud.apply_layout(opts) end

--- Sets a tag on a pane (default the focused one), or removes it with no `value`. Tags are
--- strings plugins keep on panes, like tmux's `@user` options: they show in `list` and `layout`,
--- `apply_layout` and `set_layout` take them for new panes, and `sessions` saves them. Name
--- them `plugin.name`; a pane holds at most 32, each up to 1 KB.
---@param opts {pane: integer?, key: string, value: string?}
---@return table? reply
---@return string? err
function shroud.tag(opts) end

--- Names a pane (default the focused one) on its border when `pane_frames` is on, in place of
--- the title its program set; no `title` goes back to that.
---@param opts {pane: integer?, title: string?}
---@return table? reply
---@return string? err
function shroud.pane_title(opts) end

--- Rearranges an existing tab in place, for tiling layouts. A pane node with `pane` moves that
--- pane; one without starts a new pane. Every tile must be placed, as a tile or a float; floats
--- left out stay where they are. `shroud.layout()` gives the current arrangement to edit.
---@param opts {session: string?, tab: integer?, layout: shroud.Layout}
---@return table? reply
---@return string? err
function shroud.set_layout(opts) end

---@param opts {name: string?, cwd: string?}?
---@return {session: string}? reply
---@return string? err
function shroud.session_new(opts) end

---@param opts {session: string}
---@return table? ok
---@return string? err
function shroud.session_kill(opts) end

---@param opts {session: string?, to: string}
---@return table? ok
---@return string? err
function shroud.session_rename(opts) end

---@param opts {session: string?}?
---@return {tab: integer}? reply
---@return string? err
function shroud.tab_new(opts) end

--- Shows `text` in the status bar's message slot for a few seconds.
---@param opts {text: string, session: string?}
---@return table? ok
---@return string? err
function shroud.message(opts) end

--- Draw the status bar from now on. A `status` event arrives whenever a client's bar
--- would change; answer it with `shroud.status_set`. Until then, and if this plugin stops,
--- the built-in bar shows.
---@return table? ok
---@return string? err
function shroud.status_take() end

---@return table? ok
---@return string? err
function shroud.status_release() end

--- `left` starts at the left edge and `right` ends at the right one, which wins where they meet.
---@param opts {client: integer, left: shroud.Span[]?, right: shroud.Span[]?, fill: string?} `fill` colours the bar behind and between the spans.
---@return table? ok
---@return string? err
function shroud.status_set(opts) end

shroud.json = {}

--- Arrays stay arrays, even empty ones that came from `decode`.
---@param value any
---@return string
function shroud.json.encode(value) end

--- `null` becomes nil.
---@param text string
---@return any
function shroud.json.decode(text) end

return shroud
