# Plugins

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
