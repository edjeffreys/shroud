-- Key hints, built only on the public API. On by default; turn off with
--   [[plugins]]
--   src = "builtin:hints"
--   enabled = false
local opts = { columns = 3, anchor = "bottom_right" }

-- `want` is what should be on screen; `sync` moves the overlay towards it. Replies arrive
-- later than events, so every change goes through here rather than straight to requests.
local want, shown, drawn, busy = nil, nil, nil, false
-- Bumped by every event, so a handler that waited on a reply can tell it's stale.
local gen = 0

local function sync()
  if busy then
    return
  end
  busy = true
  while true do
    local w = want
    if shown and (not w or w.session ~= shown.session or w.client ~= shown.client) then
      local id = shown.id
      shown, drawn = nil, nil
      shroud.overlay_close({ overlay = id })
    elseif w and not shown then
      local id = shroud.overlay_open({ session = w.session, client = w.client, title = w.title, lines = w.lines, anchor = opts.anchor })
      if not id then
        break
      end
      shown, drawn = { id = id, session = w.session, client = w.client }, w
    elseif w and drawn ~= w then
      shroud.overlay_update({ overlay = shown.id, title = w.title, lines = w.lines, anchor = opts.anchor })
      drawn = w
    else
      break
    end
  end
  busy = false
end

local function split(keys)
  local out = {}
  for k in keys:gmatch("%S+") do
    out[#out + 1] = k
  end
  return out
end

-- Single characters first, lowercase before its capital, then named keys.
local function before(a, b)
  if (#a == 1) ~= (#b == 1) then
    return #a == 1
  end
  local la, lb = a:lower(), b:lower()
  if la ~= lb then
    return la < lb
  end
  return a > b
end

local function next_keys(bindings, typed)
  local labels, order = {}, {}
  for _, b in ipairs(bindings.after_leader or {}) do
    local seq = split(b.keys)
    local match = #seq > #typed
    for i = 1, #typed do
      if seq[i] ~= typed[i] then
        match = false
        break
      end
    end
    if match then
      local key = seq[#typed + 1]
      local label = #seq == #typed + 1 and b.command or "+more"
      if labels[key] == nil then
        order[#order + 1] = key
        labels[key] = label
      elseif labels[key] ~= label then
        labels[key] = "+more"
      end
    end
  end
  table.sort(order, before)
  return order, labels
end

-- Nine `goto-tab N` lines say less than one `1-9 goto-tab`.
local function fold_digits(order, labels)
  local base
  for d = 1, 9 do
    local l = labels[tostring(d)]
    local b = l and l:match("^(.-) " .. d .. "$")
    if not b or (base and b ~= base) then
      return order, labels
    end
    base = b
  end
  local out = { "1-9" }
  labels["1-9"] = base
  for _, k in ipairs(order) do
    if not k:match("^[1-9]$") then
      out[#out + 1] = k
    end
  end
  return out, labels
end

local function render(order, labels)
  local cols = math.max(1, opts.columns)
  local rows = math.ceil(#order / cols)
  local kw, lw = 0, 0
  for _, k in ipairs(order) do
    kw = math.max(kw, #k)
    lw = math.max(lw, #labels[k])
  end
  local lines = {}
  for r = 1, rows do
    local line = {}
    for c = 0, cols - 1 do
      local k = order[c * rows + r]
      if k then
        local gap = c > 0 and "  " or ""
        line[#line + 1] = { text = gap .. string.rep(" ", kw - #k) .. k, style = "overlay.title", bold = true }
        local label = labels[k]
        local pad = (c < cols - 1 and order[(c + 1) * rows + r]) and string.rep(" ", lw - #label) or ""
        line[#line + 1] = { text = " " .. label .. pad, style = label == "+more" and "status.mode" or nil }
      end
    end
    lines[#lines + 1] = line
  end
  return lines
end

shroud.on("key_pending", function(ev)
  gen = gen + 1
  local mine = gen
  local bindings = shroud.bindings()
  if not bindings or mine ~= gen then
    return
  end
  local typed = {}
  for i = 2, #ev.keys do
    typed[#typed + 1] = ev.keys[i]
  end
  local order, labels = fold_digits(next_keys(bindings, typed))
  if #order == 0 then
    return
  end
  want = { client = ev.client, session = ev.session, title = table.concat(ev.keys, " "), lines = render(order, labels) }
  sync()
end)

shroud.on("key_done", function(ev)
  if want and want.client ~= ev.client then
    return
  end
  gen = gen + 1
  want = nil
  sync()
end)

shroud.on("overlay_closed", function(ev)
  if shown and ev.overlay == shown.id then
    shown, drawn = nil, nil
  end
end)

return {
  setup = function(o)
    for k, v in pairs(o or {}) do
      opts[k] = v
    end
  end,
}
