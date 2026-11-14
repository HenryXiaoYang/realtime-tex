-- rtex-dl.lua: node-list traversal that replicates LuaTeX's hlist_out/vlist_out position
-- arithmetic and emits a display list. Shared by the fast server (paragraph boxes) and by
-- the capture package (shipped pages). Coordinates in scaled points, y grows downward.
--
-- Item encodings (compact arrays):
--   {"g", font_id, char, glyph_index, x, y_baseline, width, expansion_factor}
--   {"r", x, y_top, width, height}                      rule
--   {"c", stack, cmd, data}                             pdf_colorstack whatsit
--   {"l", mode, data}                                   pdf_literal whatsit (host passthrough)
--   {"u", kind, detail}                                 unsupported node (page degraded)
--   {"m", "on"|"off", x}                                math boundary marker
--   {"i", index, x, y_top, width, height}               image (engine resource index)
--   {"M", "save"|"set"|"restore", x, y, data}           pdf_save / pdf_setmatrix / pdf_restore:
--                                                       `set` transforms what follows by the
--                                                       matrix "a b c d" about the point (x, y)
local M = {}
local math_owner
if os.getenv("RTEX_MATH_OWNER_ATTRIBUTE") then
  math_owner = package.loaded["rtex-math"]
  if not math_owner then
    math_owner = dofile(kpse.find_file("rtex-math.lua", "lua") or "rtex-math.lua")
    package.loaded["rtex-math"] = math_owner
  end
end

local node = node
local D = node.direct
local todirect = D.todirect
local getid, getnext, getlist, getfield, getsubtype = D.getid, D.getnext, D.getlist, D.getfield, D.getsubtype
local getwhd, getwidth = D.getwhd, D.getwidth
local getfont, getchar, getkern, getshift = D.getfont, D.getchar, D.getkern, D.getshift
local getglue, getexpansion, getoffsets = D.getglue, D.getexpansion, D.getoffsets
local getattribute = D.getattribute or D.get_attribute
local getdisc, getdata = D.getdisc, D.getdata
local getleader = D.getleader
local floor, ceil = math.floor, math.ceil
local pack, concat = string.pack, table.concat

local T = {}
for id, name in pairs(node.types()) do T[name] = id end
local glyph_id, glue_id, kern_id, margin_kern_id = T.glyph, T.glue, T.kern, T.margin_kern
local hlist_id, vlist_id, rule_id, disc_id, math_id = T.hlist, T.vlist, T.rule, T.disc, T.math
local whatsit_id, penalty_id, local_par_id, dir_id = T.whatsit, T.penalty, T.local_par, T.dir
local ins_id, mark_id, adjust_id, boundary_id = T.ins, T.mark, T.adjust, T.boundary
local W = {}
for st, name in pairs(node.whatsits()) do W[name] = st end
local ws_colorstack, ws_literal, ws_special = W.pdf_colorstack, W.pdf_literal, W.special
local ws_late_lua, ws_user = W.late_lua, W.user_defined
local ws_setmatrix, ws_save, ws_restore = W.pdf_setmatrix, W.pdf_save, W.pdf_restore
local ws_open, ws_write, ws_close, ws_savepos = W.open, W.write, W.close, W.save_pos
-- hyperref's link/destination/annotation whatsits carry no ink (link borders are PDF viewer
-- decorations); the engine still typesets everything around them identically
local SILENT = { [ws_user] = true, [ws_late_lua] = true, [ws_open] = true, [ws_write] = true, [ws_close] = true, [ws_savepos] = true }
for _, name in ipairs({ "pdf_dest", "pdf_annot", "pdf_start_link", "pdf_end_link", "pdf_refobj", "pdf_thread", "pdf_start_thread", "pdf_end_thread", "pdf_action", "pdf_link_data" }) do
  if W[name] then SILENT[W[name]] = true end
end

local F = dofile(kpse.find_file("rtex-dl-fonts.lua", "lua") or "rtex-dl-fonts.lua")
local font_descriptor =
  F.font_descriptor
M.round_xn_over_d = F.round_xn_over_d
M.tex_round = F.tex_round
M.set_expansion_denominator = F.set_expansion_denominator

---------------------------------------------------------------------------------------------
-- Traversal state. One state per traversal; items are appended to the current line or to
-- "other" (page material outside tagged lines).
---------------------------------------------------------------------------------------------
local State = {}
State.__index = State

local function new_state(opts)
  return setmetatable({
    opts = opts or {}, fonts = {}, lines = {}, other = {}, cur = nil, flags = {},
    attr_par = opts and opts.attr_par, attr_line = opts and opts.attr_line,
    attr_unit = opts and opts.attr_unit, is_insert = opts and opts.is_insert,
    glyph_attr = opts and opts.glyph_attr,
    -- picture cache: boxes tagged with attr_pic are reported with their page position
    -- (outermost box per id); images in pic_images are cached pictures (page degraded for
    -- hosts, which then use the PDF)
    attr_pic = opts and opts.attr_pic, pic_images = (opts and opts.pic_images) or M.pic_images, pics = {}, pic_seen = {}, pic_in = 0,
    glyphs = 0, images = 0, inserts = 0,
  }, State)
end

local function bstr(s)
  s = tostring(s or "")
  if #s > 65535 then
    -- cut before a character, not inside one: the reader rejects invalid UTF-8
    local n = 65535
    while n > 0 and s:byte(n + 1) & 0xC0 == 0x80 do n = n - 1 end
    s = s:sub(1, n)
  end
  return pack("<s2", s)
end

-- Binary sink (docs/display-list.md): records are appended to self.buf as they are produced;
-- consecutive glyphs with the same font/baseline/expansion are coalesced into one GLYPHS run.
-- A glyph run is a flat integer array (char, glyph index, x, width per glyph) packed once at
-- flush time with a cached repeated format: cheaper than one string.pack per glyph (E22).
local RUN_FMT = setmetatable({}, { __index = function(t, k) local f = "<" .. ("I4I4i4i4"):rep(k); t[k] = f; return f end })
local RUN_CHUNK = 256  -- glyphs per pack call (bounds the format string and the unpack size)
local unpack = table.unpack
function State:bin_flush_run()
  local run = self.run
  if run then
    local k = self.rn
    local n = k // 4
    self.glyphs = self.glyphs + n
    local head = pack("<I4i4i4I4", self.run_font, self.run_y, self.run_ef, n)
    local body
    if n <= RUN_CHUNK then
      body = pack(RUN_FMT[n], unpack(run, 1, k))
    else
      local parts, np = {}, 0
      for i = 1, k, RUN_CHUNK * 4 do
        local j = i + RUN_CHUNK * 4 - 1
        if j > k then j = k end
        np = np + 1
        parts[np] = pack(RUN_FMT[(j - i + 1) // 4], unpack(run, i, j))
      end
      body = concat(parts, "", 1, np)
    end
    self.buf[#self.buf + 1] = pack("<Bs4", 0x20, head .. body)
    self.run = nil
    self.rn = 0
  end
end
function State:bin_rec(tag, payload)
  self:bin_flush_run()
  self.buf[#self.buf + 1] = pack("<Bs4", tag, payload)
end

function State:emit(item)
  if self.bin then
    local t = item[1]
    if t == "g" then
      local f, y, ef = item[2], item[6], item[8] or 0
      if not self.run or f ~= self.run_font or y ~= self.run_y or ef ~= self.run_ef then
        self:bin_flush_run()
        self.run, self.rn, self.run_font, self.run_y, self.run_ef = {}, 0, f, y, ef
      end
      local n = self.rn
      local run = self.run
      run[n + 1], run[n + 2], run[n + 3], run[n + 4] = item[3] or 0, item[4] or 0xFFFFFFFF, item[5], item[7]
      self.rn = n + 4
      -- (binary mode counts glyphs at flush time)
    elseif t == "r" then self:bin_rec(0x21, pack("<i4i4i4i4", item[2], item[3], item[4], item[5]))
    elseif t == "c" then
      local cmd = item[3]; if type(cmd) ~= "number" then cmd = 255 end
      self:bin_rec(0x22, pack("<BBI2", cmd, 0, item[2] or 0) .. bstr(item[4]))
    elseif t == "l" then self:bin_rec(0x23, pack("<i4", item[2] or 0) .. bstr(item[3]))
    elseif t == "u" then self:bin_rec(0x24, bstr(item[2]) .. bstr(type(item[3]) == "table" and "" or item[3]))
    elseif t == "m" then self:bin_rec(0x25, pack("<Bi4", item[2] == "on" and 1 or 0, item[3]))
    elseif t == "i" then self:bin_rec(0x26, pack("<i4i4i4i4i4", item[2] or 0, item[3], item[4], item[5], item[6]))
    elseif t == "M" then
      local op = item[2] == "save" and 0 or (item[2] == "set" and 1 or 2)
      self:bin_rec(0x28, pack("<Bi4i4", op, item[3], item[4]) .. bstr(item[5]))
    end
    return
  end
  local tgt = self.cur and self.cur.items or self.other
  tgt[#tgt + 1] = item
end
function State:flag(kind, detail)
  local f = self.flags
  f[kind] = (f[kind] or 0) + 1
  if detail and not f[kind .. "_detail"] then f[kind .. "_detail"] = detail end
end
function State:usefont(f)
  if not self.fonts[f] then self.fonts[f] = font_descriptor(f) end
end

local hlist_out, vlist_out =
  dofile(kpse.find_file("rtex-dl-walk.lua", "lua") or "rtex-dl-walk.lua")(State, F)

function State:begin_line(n, par, line, x, baseline, w, h, d, unit)
  local gs = getfield(n, "glue_set")
  if self.bin then
    self.nlines = self.nlines + 1
    self.cur = true
    self:bin_rec(0x10, pack("<i4i4i4i4i4i4i4dBB", par or 0, line or 0, x, baseline, w, h, d, gs or 0.0,
      getfield(n, "glue_sign") or 0, getfield(n, "glue_order") or 0))
    if unit then
      local row_of = self.opts.row_of
      self:bin_rec(0x12, pack("<i4i4", unit, row_of and row_of(unit, par, line, x, baseline, w, h, d) or 0))
    end
    return
  end
  self.cur = { par = par, i = line, x = x, y = baseline, w = w, h = h, d = d, unit = unit,
               gs = gs, gsign = getfield(n, "glue_sign"), gorder = getfield(n, "glue_order"), items = {} }
end
function State:end_line()
  if self.bin then
    self:bin_rec(0x11, "")
    self.cur = nil
    return
  end
  self.lines[#self.lines + 1] = self.cur
  self.cur = nil
end

local function result(st, kind, extra)
  local fonts = {}
  for id, d in pairs(st.fonts) do fonts[tostring(id)] = d end
  local r = { kind = kind, unit = "sp", fonts = fonts, lines = st.lines, other = st.other,
              flags = st.flags, glyphs = st.glyphs, images = st.images, inserts = st.inserts }
  if st.math_owners then r.math_owners = st.math_owners end
  if extra then for k, v in pairs(extra) do r[k] = v end end
  return r
end

local function font_kind(d)
  if d.type == "virtual" then return 5 end
  local fmt = d.format
  if fmt == "opentype" then return 1 elseif fmt == "truetype" then return 2
  elseif fmt == "type1" then return 3 elseif fmt == "type3" then return 4 end
  return 0
end

-- The rest of a binary list whose item records are in st.buf: META (`m4`..`m8` are its page
-- number, page width, page height and origin slots), FONT, FLAG, IMAGE_INFO and math owner
-- records, the end record and the header. Fonts, flags and images go out in key order, so the
-- same content always gives the same bytes (the session compares pages by their bytes).
local function finish_binary(st, page_bit, w, h, d, m4, m5, m6, m7, m8, images)
  st:bin_flush_run()
  local buf = st.buf
  buf[#buf + 1] = pack("<Bs4", 0x01, pack("<i4i4i4i4i4i4i4i4I4I4", w, h, d, m4, m5, m6, m7, m8, st.glyphs, st.images))
  local ids = {}
  for id in pairs(st.fonts) do ids[#ids + 1] = id end
  table.sort(ids)
  for _, id in ipairs(ids) do
    local fd = st.fonts[id]
    buf[#buf + 1] = pack("<Bs4", 0x02, pack("<I4i4BBI2i4i4i4i4", fd.id or id, floor((fd.size or 0) + 0.5), font_kind(fd), 0,
      fd.subfont or 0, floor((fd.slant or 0) + 0.5), floor((fd.extend or 0) + 0.5), floor((fd.squeeze or 0) + 0.5),
      floor((fd.designsize or 0) + 0.5)) .. bstr(fd.filename) .. bstr(fd.psname) .. bstr(fd.name) .. bstr(fd.fullname) .. bstr(fd.format))
  end
  local keys = {}
  for k in pairs(st.flags) do keys[#keys + 1] = k end
  table.sort(keys)
  for _, k in ipairs(keys) do
    local v = st.flags[k]
    buf[#buf + 1] = pack("<Bs4", 0x03, bstr(k) .. pack("<i4", type(v) == "number" and floor(v) or 1))
  end
  if images then
    local idx = {}
    for _, im in pairs(images) do idx[#idx + 1] = im end
    table.sort(idx, function(a, b) return a.index < b.index end)
    for _, im in ipairs(idx) do
      buf[#buf + 1] = pack("<Bs4", 0x27, pack("<I4I4I4", im.index, im.page or 0, tonumber(im.pages) or 0) .. bstr(im.file))
    end
  end
  if st.math_owners then buf[#buf + 1] = pack("<Bs4", 0x29, math_owner.encode(st.math_owners)) end
  buf[#buf + 1] = pack("<Bs4", 0xFF, "")
  local body = concat(buf)
  return pack("<c4I2I2I4I4", "RTDL", 1, page_bit, 16 + #body, 0) .. body
end

local function new_state_bin(opts)
  local st = new_state(opts)
  st.bin = true; st.buf = {}; st.rn = 0; st.nlines = 0
  return st
end

-- Paragraph box straight to binary display-list bytes (encoding revision 1). Same traversal as
-- M.paragraph; records are written as they are produced, META/FONT/FLAG records last.
-- Returns bytes, line count, glyph count, width, height, depth, flags table.
-- `extra_inserts`: inserts the box cannot hold (margin notes refused in a box), counted in META.
function M.paragraph_binary(boxnode, initial_color, extra_inserts)
  local box = todirect(boxnode)
  local st = new_state_bin({ lines_at_top = true })
  if initial_color and initial_color ~= "" then st:emit({ "c", 0, 0, initial_color }) end
  local w, h, d = getwhd(box)
  vlist_out(st, box, 0, 0)
  st.inserts = st.inserts + (extra_inserts or 0)
  local bytes = finish_binary(st, 0, w, h, d, 0, 0, 0, 0, st.inserts)
  return bytes, st.nlines, st.glyphs, w, h, d, st.flags, st.inserts
end

-- Internals exposed for profiling (rtex-serve.lua `profile` op).
M._hlist_out = function(st, box, left, base_v) return hlist_out(st, box, left, base_v) end
M._new_state_bin = function() return new_state_bin({ lines_at_top = true }) end

-- Paragraph box (a \vbox whose top-level hlists are the lines). Origin: top-left of the box.
-- `initial_color` (raw PDF color operators) is the color in force when the paragraph starts.
function M.paragraph(boxnode, initial_color)
  local box = todirect(boxnode)
  local st = new_state({ lines_at_top = true })
  if initial_color and initial_color ~= "" then st.other[1] = { "c", 0, 0, initial_color } end
  local w, h, d = getwhd(box)
  vlist_out(st, box, 0, 0)
  return result(st, "paragraph", { width = w, height = h, depth = d })
end

-- Shipped page box straight to binary display-list bytes (a page list). `attr_par`/`attr_line`
-- identify tagged lines. Origin: page top-left; the box is offset by (1in + \hoffset,
-- 1in + \voffset) like the PDF backend does. `opts`: the traversal options (`attr_unit`,
-- `is_insert`, `attr_pic`, `pic_images`), `row_of(unit, par, line, x, y, w, h, d)` giving a
-- unit line's row, `flags` to add to the page's own, and `images` (IMAGE_INFO records).
-- Returns bytes, the pictures seen (`note_pic`) and the page height.
function M.page_binary(boxnode, attr_par, attr_line, page_no, opts)
  local box = todirect(boxnode)
  opts.attr_par, opts.attr_line = attr_par, attr_line
  local st = new_state_bin(opts)
  local one_inch = 4736286  -- 72.27pt in sp
  local ox = one_inch + tex.hoffset
  local oy = one_inch + tex.voffset
  local w, h, d = getwhd(box)
  vlist_out(st, box, ox, oy)
  for k, v in pairs(opts.flags or {}) do st.flags[k] = v end
  local ph = tex.pageheight
  return finish_binary(st, 1, w, h, d, page_no, tex.pagewidth, ph, ox, oy, opts.images), st.pics, ph
end

return M
