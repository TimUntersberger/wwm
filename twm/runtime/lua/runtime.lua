nog.inspect = dofile(nog.runtime_path .. "/lua/inspect.lua")
nog.uv = require 'luv'

function nog.clone(value, is_deep)
  local t = type(value)

  if t == "string" then
    local len = #value
    local res = ""
    local i = 1

    while i <= len do
      res = res .. string.char(value:byte(len))
      len = len - 1
    end

    return res
  end

  error("Unsupported type: " .. t)
end

function nog.tbl_filter(tbl, f)
  local res = {}
  for _, x in ipairs(tbl) do
    if f(x) then
      table.insert(res, x)
    end
  end
  return res
end

function nog.split(s, sep)
  if sep == nil then
    sep = "%s"
  end
  local t={}
  for str in string.gmatch(s, "([^"..sep.."]+)") do
    table.insert(t, str)
  end
  return t
end

local modes = {}
local previous_kbs = nil
local current_mode = nil

function nog.mode(name, cb)
  modes[name] = cb
end

function nog.toggle_mode(name)
  local cb = modes[name]

  assert(cb ~= nil, string.format("Mode '%s' has not been defined yet", name))

  if current_mode ~= nil then
    if current_mode == name then
      local mode_kbs = nog.get_keybindings()

      nog.__unbind_batch(nog.tbl_filter(mode_kbs, function(kb)
        return kb.mode == "n"
      end))

      nog.__bind_batch(nog.tbl_filter(previous_kbs, function(kb)
        return kb.mode == "n"
      end))

      current_mode = nil
    else
    end
  else
    previous_kbs = nog.get_keybindings()

    nog.__unbind_batch(nog.tbl_filter(previous_kbs, function(kb)
      return kb.mode == "n"
    end))

    cb()

    current_mode = name
  end
end

local function create_bind_tbl_fn(mode)
  return function(modifier, cb, tbl)
    for key, val in pairs(tbl) do
      local key = string.format("%s+%s", modifier, key)
      nog[mode .. "bind"](key, function()
        cb(val)
      end)
    end
  end
end

local function create_bind_fn(mode)
  return function(key, cb)
    nog.bind(mode, key, cb)
  end
end

nog.bind = function(m, k, f)
  table.insert(nog.__callbacks, f)
  nog.__bind(m, k, #nog.__callbacks)
end

nog.nbind = create_bind_fn("n")
nog.nbind_tbl = create_bind_tbl_fn("n")

nog.gbind = create_bind_fn("g")
nog.gbind_tbl = create_bind_tbl_fn("g")

nog.wbind = create_bind_fn("w")
nog.wbind_tbl = create_bind_tbl_fn("w")

nog.components = {}
nog.components.workspaces = function()
  return {
    name = "Workspaces",
    render = function(display_id)
      local c = nog.config
      local ws_ids = nog.get_active_ws_of_display(display_id)
      local result = {}
      local factor

      for _, ws_id in ipairs(ws_ids) do
        if c.light_theme then
          factor = nog.is_ws_focused(ws_id) and 0.75 or 0.9
        else
          factor = nog.is_ws_focused(ws_id) and 2.0 or 1.5
        end

        local bg = nog.scale_color(c.bar.color, factor)

        table.insert(result, {
          text = nog.get_ws_text(ws_id),
          value = ws_id,
          bg = bg
        })
      end

      return result
    end,
    on_click = function(display_id, payload)
      nog.ws_change(payload)
    end
  }
end

nog.components.datetime = function(format)
  return {
    name = "Datetime",
    render = function()
      return {{
        text = nog.fmt_datetime(format),
      }}
    end
  }
end

nog.components.padding = function(amount)
  return {
    name = "Padding",
    render = function()
      return {{
        text = string.rep(" ", amount),
      }}
    end
  }
end

nog.components.active_mode = function()
  return {
    name = "ActiveMode",
    render = function()
      local mode
      if current_mode ~= nil then
        mode = current_mode .. " is active"
      end
      return {{
        text = mode or "",
      }}
    end
  }
end

nog.components.current_window = function(max_width)
  max_width = max_width or 0

  return {
    name = "CurrentWindow",
    render = function(display_id)
      local win_id = nog.get_focused_win_of_display(display_id)

      if not win_id then
        return {{ text = "" }}
      end
      
      local title = win_id and nog.get_win_title(win_id) or ""

      if max_width ~= 0 then
        title = title:sub(1, max_width)
      end

      return {{
        text = title,
      }}
    end
  }
end

nog.components.split_direction = function(values)
  return {
    name = "SplitDirection",
    render = function(display_id)
      local ws_id = nog.get_focused_ws_of_display(display_id)
      
      if not ws_id then
        return {{ text = "" }}
      end

      local info = nog.get_ws_info(ws_id)

      return {{
        text = info.split_direction == "Vertical" and values[1] or values[2],
      }}
    end
  }
end

nog.components.fullscreen_indicator = function(indicator)
  return {
    name = "FullscreenIndicator",
    render = function(display_id)
      local ws_id = nog.get_focused_ws_of_display(display_id)

      if not ws_id then
        return {{ text = "" }}
      end

      local info = nog.get_ws_info(ws_id)

      return {{
        text = info.is_fullscreen and indicator or "",
      }}
    end
  }
end

-- This is used to create a proxy table which notifies nog when a config value changes
function create_proxy(path)
  path = path or {}

  local prefix = "nog.config"
  local tbl = nog.config
  local parts_len = #path
  local proxy = {}

  for _, part in ipairs(path) do
    prefix = prefix .. "." .. part
    tbl = tbl[part]
  end

  setmetatable(proxy, {
    __index = tbl,
    __newindex = function(t, k, v)
      nog.__on_config_updated(prefix, k, v, nog.__is_setup)
      tbl[k] = v
    end
  })

  local tmp_tbl = nog
  -- name of the field that gets replaced
  local last_part = "config"

  for i, part in ipairs(path) do
    if i == 1 then
      tmp_tbl = nog.config
    end
    if i == parts_len then
      last_part = part
      break
    else
      tmp_tbl = tmp_tbl[part]
    end
  end

  tmp_tbl[last_part] = proxy
end

create_proxy({"bar", "components"})
create_proxy({"bar"})
create_proxy({"rules"})
create_proxy({"workspaces"})
create_proxy({})

nog.config.bar.components = {
  left = {
    nog.components.workspaces()
  },
  center = {
    nog.components.datetime("%T")
  },
  right = {
    nog.components.active_mode(),
    nog.components.padding(5),
    nog.components.split_direction("V", "H"),
    nog.components.padding(5),
    nog.components.datetime("%e %b %Y"),
    nog.components.padding(1),
  }
}

