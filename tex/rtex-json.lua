-- rtex-json.lua: minimal JSON encoder/decoder (Lua 5.3, LuaTeX). No external deps.
local M = {}

local type, mtype, fmt, tostring, tonumber, next = type, math.type, string.format, tostring, tonumber, next
local find, gsub, sort, concat, huge = string.find, string.gsub, table.sort, table.concat, math.huge

local escapes = { ['"'] = '\\"', ['\\'] = '\\\\', ['\b'] = '\\b', ['\f'] = '\\f',
                  ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t' }
local function esc(s)
  if not find(s, '[%c"\\]') then return s end
  return (gsub(s, '[%c"\\]', function(c) return escapes[c] or fmt('\\u%04x', c:byte()) end))
end

local function is_array(t)
  local n = #t
  if n == 0 then return next(t) == nil end
  for k in next, t do
    if mtype(k) ~= "integer" or k < 1 or k > n then return false end
  end
  return true
end

-- integers go into the buffer as they are: table.concat writes them as "%d" does
local function num(v)
  if mtype(v) == "integer" then return v end
  if v ~= v or v == huge or v == -huge then return "null" end
  return fmt("%.17g", v)
end

-- `x` as the integer `num` would print it as ("%d", or "%.17g" of a whole float), else nil
local tointeger = math.tointeger
local function as_int(x)
  local i = tointeger(x)
  if i and i == x and i < 1e15 and i > -1e15 and (i ~= 0 or 1 / x > 0) then return i end
end

-- The output goes to `buf` (pieces 1..nb): page display lists have hundreds of thousands of
-- pieces, and `#buf` per append costs a binary search each time.
local buf, nb
local function encode_value(v)
  local tv = type(v)
  if tv == "number" then nb = nb + 1; buf[nb] = num(v)
  elseif tv == "string" then nb = nb + 1; buf[nb] = '"' .. esc(v) .. '"'
  elseif tv == "table" then
    if is_array(v) then
      local n = #v
      if n == 8 and v[1] == "g" then
        -- a glyph item of a display list (most of a page): one format call
        local f, c, x, y, w, ef = as_int(v[2]), as_int(v[3]), as_int(v[5]), as_int(v[6]), as_int(v[7]), as_int(v[8])
        if f and c and x and y and w and ef then
          local gi = v[4]
          if gi == nil then -- TFM fonts
            nb = nb + 1; buf[nb] = fmt('["g",%d,%d,null,%d,%d,%d,%d]', f, c, x, y, w, ef)
            return
          end
          gi = as_int(gi)
          if gi then
            nb = nb + 1; buf[nb] = fmt('["g",%d,%d,%d,%d,%d,%d,%d]', f, c, gi, x, y, w, ef)
            return
          end
        end
      end
      nb = nb + 1; buf[nb] = "["
      for i = 1, n do
        if i > 1 then nb = nb + 1; buf[nb] = "," end
        -- display-list items are flat arrays of numbers and strings: no call per element
        local x = v[i]
        local tx = type(x)
        if tx == "number" then nb = nb + 1; buf[nb] = num(x)
        elseif tx == "string" then nb = nb + 1; buf[nb] = '"' .. esc(x) .. '"'
        else encode_value(x) end
      end
      nb = nb + 1; buf[nb] = "]"
    else
      nb = nb + 1; buf[nb] = "{"
      local keys, nk = {}, 0
      for k in next, v do nk = nk + 1; keys[nk] = tostring(k) end
      sort(keys)
      for i = 1, nk do
        local k = keys[i]
        nb = nb + 1; buf[nb] = (i > 1 and ',"' or '"') .. esc(k) .. '":'
        encode_value(v[k] == nil and v[tonumber(k)] or v[k])
      end
      nb = nb + 1; buf[nb] = "}"
    end
  elseif tv == "nil" then nb = nb + 1; buf[nb] = "null"
  elseif tv == "boolean" then nb = nb + 1; buf[nb] = v and "true" or "false"
  else nb = nb + 1; buf[nb] = '"' .. esc(tostring(v)) .. '"' end
end
function M.encode(v)
  local outer, outer_n = buf, nb -- reentrant (a __tostring calling encode)
  buf, nb = {}, 0
  encode_value(v)
  local s = concat(buf, "", 1, nb)
  buf, nb = outer, outer_n
  return s
end

-- Decoder -------------------------------------------------------------------
local function decode_error(s, i, msg) error(string.format("json: %s at %d: %s", msg, i, s:sub(i, i + 20))) end
local decode_value
local function skip(s, i) return s:find("%S", i) or #s + 1 end
local function decode_string(s, i)
  local j = i + 1
  local buf = {}
  while true do
    local c = s:sub(j, j)
    if c == "" then decode_error(s, i, "unterminated string") end
    if c == '"' then return table.concat(buf), j + 1 end
    if c == "\\" then
      local e = s:sub(j + 1, j + 1)
      local map = { ['"'] = '"', ['\\'] = '\\', ['/'] = '/', b = '\b', f = '\f', n = '\n', r = '\r', t = '\t' }
      if map[e] then buf[#buf + 1] = map[e]; j = j + 2
      elseif e == "u" then
        local hex = s:sub(j + 2, j + 5)
        local cp = tonumber(hex, 16)
        j = j + 6
        if cp >= 0xD800 and cp <= 0xDBFF and s:sub(j, j + 1) == "\\u" then
          local lo = tonumber(s:sub(j + 2, j + 5), 16)
          cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
          j = j + 6
        end
        buf[#buf + 1] = utf8.char(cp)
      else decode_error(s, j, "bad escape") end
    else
      local k = s:find('["\\]', j) or (#s + 1)
      buf[#buf + 1] = s:sub(j, k - 1)
      j = k
    end
  end
end
function decode_value(s, i)
  i = skip(s, i)
  local c = s:sub(i, i)
  if c == "{" then
    local obj = {}
    i = skip(s, i + 1)
    if s:sub(i, i) == "}" then return obj, i + 1 end
    while true do
      i = skip(s, i)
      if s:sub(i, i) ~= '"' then decode_error(s, i, "expected key") end
      local k; k, i = decode_string(s, i)
      i = skip(s, i)
      if s:sub(i, i) ~= ":" then decode_error(s, i, "expected :") end
      local v; v, i = decode_value(s, i + 1)
      obj[k] = v
      i = skip(s, i)
      local d = s:sub(i, i)
      if d == "," then i = i + 1 elseif d == "}" then return obj, i + 1 else decode_error(s, i, "expected , or }") end
    end
  elseif c == "[" then
    local arr = {}
    i = skip(s, i + 1)
    if s:sub(i, i) == "]" then return arr, i + 1 end
    while true do
      local v; v, i = decode_value(s, i)
      arr[#arr + 1] = v
      i = skip(s, i)
      local d = s:sub(i, i)
      if d == "," then i = i + 1 elseif d == "]" then return arr, i + 1 else decode_error(s, i, "expected , or ]") end
    end
  elseif c == '"' then return decode_string(s, i)
  elseif s:sub(i, i + 3) == "true" then return true, i + 4
  elseif s:sub(i, i + 4) == "false" then return false, i + 5
  elseif s:sub(i, i + 3) == "null" then return nil, i + 4
  else
    local num = s:match("^-?%d+%.?%d*[eE]?[-+]?%d*", i)
    if not num or num == "" then decode_error(s, i, "unexpected token") end
    local v = math.tointeger(tonumber(num)) or tonumber(num)
    return v, i + #num
  end
end
function M.decode(s)
  local v = decode_value(s, 1)
  return v
end

return M
