-- The public `shroud` table, built over the host's raw calls. Requests yield the handler's
-- coroutine until the reply arrives, so plugin code reads as if it were synchronous.
local raw, api = ...

local shroud = { api = api }

function shroud.request(req)
  if type(req) ~= "table" or type(req.cmd) ~= "string" then
    error("shroud.request needs a table with a `cmd` field", 2)
  end
  local reply = coroutine.yield(raw.send(req))
  if reply.ok then
    reply.ok, reply.id = nil, nil
    return reply
  end
  return nil, reply.error
end

-- Each request is a function of the same name; `field` picks the useful part of the reply.
local function wrap(cmd, field)
  shroud[cmd] = function(opts)
    local req = {}
    for k, v in pairs(opts or {}) do
      req[k] = v
    end
    req.cmd = cmd
    local reply, err = shroud.request(req)
    if not reply then
      return nil, err
    end
    if field then
      return reply[field]
    end
    return reply
  end
end

wrap("list", "sessions")
wrap("split", "pane")
wrap("send")
wrap("output", "text")
wrap("capture", "text")
wrap("close")
wrap("focus")
wrap("overlay_open", "overlay")
wrap("overlay_update")
wrap("overlay_close")
wrap("bindings")
wrap("layout", "layout")
wrap("apply_layout")
wrap("set_layout")
wrap("tag")
wrap("pane_title")
wrap("session_new")
wrap("session_kill")
wrap("session_rename")
wrap("tab_new")
wrap("message")
wrap("status_take")
wrap("status_release")
wrap("status_set")

-- `shroud.run("zoom")` and `shroud.run({ exec = { "git", "pull" } })` as well as the table form.
function shroud.run(spec, opts)
  local shortcut = type(spec) == "table" and (spec.run ~= nil or spec.exec ~= nil)
  local req = {}
  for k, v in pairs((type(spec) == "table" and not shortcut) and spec or opts or {}) do
    req[k] = v
  end
  if type(spec) == "string" or shortcut then
    req.command = spec
  end
  req.cmd = "run"
  local reply, err = shroud.request(req)
  if not reply then
    return nil, err
  end
  return true
end

-- Like nvim's vim.system. Without `on_exit`, `:wait()` yields the handler until the job ends.
function shroud.system(cmd, opts, on_exit)
  if type(opts) == "function" then
    opts, on_exit = nil, opts
  end
  if type(cmd) ~= "table" or type(cmd[1]) ~= "string" then
    error("shroud.system needs a list: the program, then its arguments", 2)
  end
  local id, pid = raw.system(cmd, opts, on_exit)
  local job = { pid = pid }
  function job:wait()
    local done = raw.system_result(id)
    if done ~= nil then
      return done
    end
    return coroutine.yield(raw.system_wait(id))
  end
  function job:kill(signal)
    return raw.system_kill(id, signal)
  end
  return job
end

shroud.keymap = raw.keymap
shroud.on = raw.on
shroud.command = raw.command
shroud.after = raw.after
shroud.log = raw.log
shroud.data_dir = raw.data_dir
shroud.json = raw.json

-- `print` would go to /dev/null; the log is where a plugin author will look.
print = function(...)
  local parts = {}
  for i = 1, select("#", ...) do
    parts[#parts + 1] = tostring((select(i, ...)))
  end
  raw.log(table.concat(parts, "\t"))
end

package.loaded.shroud = shroud
_G.shroud = shroud

shroud.plugins = raw.plugins

-- `config`, from a spec in init.lua, replaces the usual `setup(opts)` call.
return function(chunk, opts, config)
  local m = chunk()
  if config then
    config(m, opts)
  elseif type(m) == "table" and type(m.setup) == "function" then
    m.setup(opts == nil and {} or opts)
  end
end
