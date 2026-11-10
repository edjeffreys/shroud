-- Session persistence, built only on the public API. Off by default; turn on with
--   [[plugins]]
--   src = "builtin:sessions"
-- It restores shells in the right directories, not the programs that were running in them.
-- With `auto_restore = true` it restores once when the server starts. Programs that were
-- running are typed at their pane's prompt without Enter (`retype = false` turns that off),
-- so nothing reruns unless you say so.
local opts = { debounce_ms = 2000, auto_restore = false, retype = true }

local function file()
  return shroud.data_dir() .. "/sessions.json"
end

local function snapshot()
  local out = {}
  for _, s in ipairs(shroud.list() or {}) do
    local tabs = {}
    for t = 1, #s.tabs do
      local layout = shroud.layout({ session = s.name, tab = t })
      if layout then
        tabs[#tabs + 1] = layout
      end
    end
    if #tabs > 0 then
      out[#out + 1] = { name = s.name, tabs = tabs }
    end
  end
  return out
end

-- Never overwrites with nothing: the last shell exiting would otherwise erase what a
-- restart needs.
local function save()
  local sessions = snapshot()
  if #sessions == 0 then
    return 0
  end
  local path = file()
  local f, err = io.open(path .. ".tmp", "w")
  if not f then
    error("can't save sessions: " .. err)
  end
  f:write(shroud.json.encode({ version = 1, sessions = sessions }))
  f:close()
  assert(os.rename(path .. ".tmp", path))
  return #sessions
end

local queued = false
local function save_soon()
  if queued then
    return
  end
  queued = true
  shroud.after(opts.debounce_ms, function()
    queued = false
    save()
  end)
end

local function first_cwd(node)
  if node.cwd then
    return node.cwd
  end
  for _, part in ipairs(node.parts or {}) do
    local cwd = first_cwd(part)
    if cwd then
      return cwd
    end
  end
end

-- Saved and rebuilt layouts share a shape, so walking them together pairs each saved pane
-- with the pane that replaced it.
local function retype(saved, built)
  if not (saved and built) then
    return
  end
  if saved.process and built.pane then
    shroud.send({ pane = built.pane, input = { { text = saved.process } } })
  end
  for i, part in ipairs(saved.parts or {}) do
    retype(part, (built.parts or {})[i])
  end
end

local function retype_tab(session, tab, saved)
  local built = shroud.layout({ session = session, tab = tab })
  if not built then
    return
  end
  retype(saved.root, built.root)
  for i, f in ipairs(saved.floats or {}) do
    retype(f, (built.floats or {})[i])
  end
end

local function restore(at_start)
  local f = io.open(file())
  if not f then
    return nil, "nothing saved yet"
  end
  local ok, data = pcall(shroud.json.decode, f:read("*a"))
  f:close()
  if not ok or type(data) ~= "table" then
    return nil, "the saved sessions file is damaged"
  end
  local running = {}
  for _, s in ipairs(shroud.list() or {}) do
    running[s.name] = s
  end
  local restored, failed = 0, 0
  for _, s in ipairs(data.sessions or {}) do
    local live = running[s.name]
    -- At server start the first client's session is a lone new shell; it gives way.
    local fresh = at_start and live and #live.tabs == 1 and #live.tabs[1].panes == 1 and not live.tabs[1].panes[1].running
    if (fresh or not live) and #(s.tabs or {}) > 0 then
      local made = fresh
        or shroud.session_new({ name = s.name, cwd = first_cwd(s.tabs[1].root) })
        or shroud.session_new({ name = s.name })
      if made then
        local built = 0
        for _, layout in ipairs(s.tabs) do
          local made_tab = shroud.apply_layout({ session = s.name, layout = layout })
          if made_tab then
            built = built + 1
            if opts.retype then
              retype_tab(s.name, made_tab.tab, layout)
            end
          else
            failed = failed + 1
          end
        end
        -- The session's own first tab goes once the saved ones exist.
        if built > 0 then
          for _, listed in ipairs(shroud.list() or {}) do
            if listed.name == s.name then
              for _, pane in ipairs(listed.tabs[1].panes) do
                shroud.close({ pane = pane.id })
              end
            end
          end
        end
        restored = restored + 1
      end
    end
  end
  local msg = ("restored %d session%s"):format(restored, restored == 1 and "" or "s")
  if failed > 0 then
    msg = msg .. (", %d tab%s failed"):format(failed, failed == 1 and "" or "s")
  end
  return msg
end

shroud.command("save", function(ev)
  local n = save()
  shroud.message({ session = ev.session, text = n > 0 and ("saved %d session%s"):format(n, n == 1 and "" or "s") or "nothing to save" })
end)

shroud.command("restore", function(ev)
  local msg, err = restore()
  shroud.message({ session = ev.session, text = msg or err })
end)

for _, event in ipairs({ "layout_changed", "tab_changed", "session_opened", "session_closed", "session_renamed", "command_finished", "tag_changed" }) do
  shroud.on(event, save_soon)
end
shroud.on("client_detached", function()
  save()
end)

-- The host restarts whenever the config changes, so a marker keeps this to once per server.
local function restore_at_start()
  local server = shroud.server
  if not server then
    return
  end
  local id = ("%d:%d"):format(server.pid, server.started)
  local marker = shroud.data_dir() .. "/restored"
  local f = io.open(marker)
  local done = f and f:read("*a") == id
  if f then
    f:close()
  end
  if done then
    return
  end
  f = assert(io.open(marker, "w"))
  f:write(id)
  f:close()
  local msg = restore(true)
  if msg then
    shroud.log(msg)
  end
end

return {
  setup = function(o)
    for k, v in pairs(o or {}) do
      opts[k] = v
    end
    if opts.auto_restore then
      restore_at_start()
    end
  end,
}
