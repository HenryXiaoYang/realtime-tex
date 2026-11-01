-- texlua tex/tests/run.lua : unit tests for the Lua side that need no TeX run.
package.path = "tex/?.lua;" .. package.path
local failures = 0
local function check(name, cond, detail)
  if cond then print("ok   " .. name) else failures = failures + 1; print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or "")) end
end

-- JSON round trip
local json = dofile("tex/rtex-json.lua")
local v = { a = 1, b = { 1, 2, 3 }, c = "x\"y\\z\n", d = true, e = 2.5, f = {} }
local s = json.encode(v)
local back = json.decode(s)
check("json roundtrip scalars", back.a == 1 and back.c == v.c and back.d == true and back.e == 2.5)
check("json roundtrip arrays", #back.b == 3 and back.b[3] == 3)
check("json decode unicode escape", json.decode('"\\u00e9\\ud83d\\ude00"') == "é😀")
check("json decode nested", json.decode('{"x":[{"y":null},{"y":-3e2}]}').x[2].y == -300)

-- The table encoder (page path) must carry the same optional record as the direct
-- paragraph encoder. Scan framed records, rather than matching a byte inside a payload,
-- and decode the packed payload independently of the codec.
local bin = dofile("tex/rtex-dl-bin.lua")
local codec = dofile("tex/rtex-math-codec.lua")
local find_file = kpse.find_file
kpse.find_file = function(name) return "tex/" .. name end
local function owners_record(list)
  local bytes = bin.encode(list)
  local pos = 17
  while pos <= #bytes do
    local tag, len, payload = string.unpack("<BI4", bytes, pos)
    if tag == 0x29 then return bytes:sub(payload, payload + len - 1) end
    pos = payload + len
  end
end
local function decode_owners(p)
  local glyphs, fields, pos = {}, {}, 1
  local ng; ng, pos = string.unpack("<I4", p, pos)
  for i = 1, ng do
    local o, f, c, x0, y0, x1, y1
    o, f, c, x0, y0, x1, y1, pos = string.unpack("<i4I4I4i4i4i4i4", p, pos)
    glyphs[i] = { owner = o, font = f, char = c, bbox = { x0, y0, x1, y1 } }
  end
  local nf; nf, pos = string.unpack("<I4", p, pos)
  for i = 1, nf do
    local o, r, e, pad, x0, y0, x1, y1
    o, r, e, pad, x0, y0, x1, y1, pos = string.unpack("<i4BBI2i4i4i4i4", p, pos)
    fields[i] = { owner = o, role = codec.ROLE_NAME[r], empty = e == 1, pad = pad, bbox = { x0, y0, x1, y1 } }
  end
  return { glyphs = glyphs, fields = fields }, pos == #p + 1
end
local list = { kind = "page", fonts = {}, lines = {}, other = {} }
check("ownership record absent by default", owners_record(list) == nil)
-- 300 glyphs and 3 fields: crosses the codec's 128-row packing chunks.
local mo = { g = {}, ng = 300, f = { -7, 1, 0, 1, 2, 3, 4, 0, 4, 1, 5, 6, 5, 6, 2147483647, 8, 0, -1, -2, 9, 10 }, nf = 3 }
for r = 0, 299 do
  local i = r * 7
  mo.g[i + 1], mo.g[i + 2], mo.g[i + 3] = r - 42, 20 + r % 3, 97 + r
  mo.g[i + 4], mo.g[i + 5], mo.g[i + 6], mo.g[i + 7] = r, -r, r + 10, r + 20
end
list.math_owners = mo
local decoded, exact = decode_owners(owners_record(list))
check("ownership record has no trailing bytes", exact)
check("ownership record decodes independently", json.encode(decoded.glyphs) == json.encode(codec.to_table(mo).glyphs))
check("ownership fields keep role, empty flag and zero padding",
  decoded.fields[1].role == "num" and decoded.fields[2].role == "sup" and decoded.fields[2].empty
  and decoded.fields[3].role == "math" and decoded.fields[3].owner == 2147483647
  and decoded.fields[1].pad == 0 and decoded.fields[3].pad == 0)
kpse.find_file = find_file

-- rtex-dl arithmetic (stub the node library enough to load the module)
node = node or {}
node.types = node.types or function() return { [0] = "hlist", [1] = "vlist", [2] = "rule", [29] = "glyph", [12] = "glue", [13] = "kern", [28] = "margin_kern", [7] = "disc", [11] = "math", [8] = "whatsit", [14] = "penalty", [9] = "local_par", [10] = "dir", [3] = "ins", [4] = "mark", [5] = "adjust", [6] = "boundary" } end
node.whatsits = node.whatsits or function() return { [0] = "open", [1] = "write", [2] = "close", [8] = "late_lua", [22] = "pdf_literal", [28] = "pdf_colorstack", [3] = "special", [6] = "save_pos", [7] = "user_defined", [29] = "pdf_setmatrix", [30] = "pdf_save", [31] = "pdf_restore" } end
node.subtypes = node.subtypes or function() return { [0] = "normal", [1] = "box", [2] = "image", [3] = "empty", [4] = "user", [9] = "outline" } end
-- direct nodes are plain tables, enough to walk boxes of whatsits
local function field(k) return function(n) return n[k] end end
local function dim(k) return function(n) return n[k] or 0 end end
node.direct = node.direct or setmetatable({
  todirect = function(n) return n end, getid = field("id"), getnext = field("next"),
  getlist = field("list"), getsubtype = field("subtype"), getfield = function(n, k) return n[k] end,
  getwidth = dim("width"), getshift = dim("shift"),
  getwhd = function(n) return n.width or 0, n.height or 0, n.depth or 0 end,
}, { __index = function() return function() end end })
node.type = node.type or function(id) return node.types()[id] end
font = font or { getfont = function() return nil end }
tex = tex or { hoffset = 0, voffset = 0, pagewidth = 0, pageheight = 0 }
local dl = dofile("tex/rtex-dl.lua")
check("tex_round positive", dl.tex_round(2.5) == 3 and dl.tex_round(2.4999) == 2)
check("tex_round negative", dl.tex_round(-2.5) == -3 and dl.tex_round(-2.4999) == -2)
-- round_xn_over_d against exact rational rounding
local function ref(x, n, d) local q = x * n / d; return q >= 0 and math.floor(q + 0.5) or -math.floor(-q + 0.5) end
local okc = true
for _, c in ipairs{ {431102, 1020000, 1000000}, {431102, 980000, 1000000}, {705419, 983000, 1000000}, {123456789, 1001000, 1000000}, {-358810, 1017000, 1000000}, {1, 1, 3}, {2, 1, 4} } do
  local a, b = dl.round_xn_over_d(c[1], c[2], c[3]), ref(c[1], c[2], c[3])
  if a ~= b then okc = false; print("  mismatch", c[1], c[2], c[3], a, b) end
end
check("round_xn_over_d matches exact rounding", okc)
-- expansion formula: ef in millionths, 2% stretch of 431102 sp
check("expansion 2%", dl.round_xn_over_d(431102, 1020000, 1000000) == 439724)

-- specials: LuaTeX's PDF backend draws only those starting with "pdf:" or "PDF:" and ignores
-- the rest ("Non-PDF special ignored!"), so only those make the display list inexact
local function special(data) return { id = 8, subtype = 3, data = data } end
local function special_flags(data)
  local in_row = dl.paragraph({ id = 1, list = { id = 0, list = special(data) } }).flags.special
  local in_vlist = dl.paragraph({ id = 1, list = special(data) }).flags.special
  return in_row, in_vlist
end
for _, data in ipairs{ "papersize=597.50787pt,845.04684pt", "Pdf: 0 0 m", " pdf: 0 0 m", "pdf", "color push gray 0" } do
  local a, b = special_flags(data)
  check("ignored special not flagged: " .. data, a == nil and b == nil, tostring(a) .. " " .. tostring(b))
end
for _, data in ipairs{ "pdf: 0 0 m 1 1 l S", "PDF:direct:0 0 m", "pdf:" } do
  local a, b = special_flags(data)
  check("pdf special flagged: " .. data, a == 1 and b == 1, tostring(a) .. " " .. tostring(b))
end

if failures > 0 then print(failures .. " failure(s)"); os.exit(1) else print("all Lua tests passed") end
