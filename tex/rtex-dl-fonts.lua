-- rtex-dl-fonts.lua: what rtex-dl.lua and rtex-dl-walk.lua need to know about fonts, and TeX's
-- rounding. Glyph widths and indices per font (TFM virtual fonts resolved to their base fonts),
-- font expansion, and the font descriptions a display list carries. Results are cached per
-- font id for the life of the process.
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

-- TeX's round(): web2c zround.
local function tex_round(r)
  if r >= 0 then return floor(r + 0.5) else return ceil(r - 0.5) end
end
local BILLION = 1000000000.0
local function vet_glue(g)
  if g > BILLION then return BILLION elseif g < -BILLION then return -BILLION else return g end
end
-- TeX's round_xn_over_d (tex.web §107 variant used by pdfTeX/LuaTeX for expansion).
local function round_xn_over_d(x, n, d)
  local positive = x >= 0
  if not positive then x = -x end
  local t = (x % 32768) * n
  local u = (x // 32768) * n + (t // 32768)
  local v = (u % d) * 32768 + (t % 32768)
  u = 32768 * (u // d) + (v // d)
  v = v % d
  if 2 * v >= d then u = u + 1 end
  if positive then return u else return -u end
end
-- Font metrics cache: width per (font, char) and glyph index.
local font_cache = {}

-- Base fonts of TFM virtual fonts, defined on first use: "name@size" -> font id, or false when
-- the font cannot be loaded.
-- font.getfont does not return a font defined from Lua with its name, so the cache entry is built
-- from the TFM table itself.
local vf_base_ids = {}
local function vf_base_font(name, size)
  local key = name .. "@" .. size
  local id = vf_base_ids[key]
  if id == nil then
    id = false
    local ok, t = pcall(font.read_tfm, name, size)
    if ok and t then
      local chars = t.characters
      local ok2, nid = pcall(font.define, t)
      if ok2 and nid then
        id = nid
        font_cache[nid] = { tfm = { name = name, size = size, designsize = t.designsize,
                                    type = "real", format = "unknown",
                                    slant = 0, extend = 1000, squeeze = 1000 },
                            chars = chars or {}, widths = {}, idx = {}, ew = {}, virtual = false }
      end
    end
    vf_base_ids[key] = id
  end
  return id or nil
end

-- A TFM font with a .vf file (newtxmath's ntxmi/ntxsy/ntxexx, ...) comes back from font.getfont
-- as type "real" without commands: the PDF backend reads the .vf only at shipout. Read it here
-- so emit_virtual can expand its characters into the real fonts. Returns chars, fonts or nil.
local function tfm_virtual(tfm)
  if tfm.type == "virtual" or tfm.filename or not tfm.name or not tfm.size then return nil end
  if not kpse.find_file(tfm.name, "vf") then return nil end
  local ok, vf = pcall(font.read_vf, tfm.name, tfm.size)
  if not ok or not vf or not vf.characters or not vf.fonts then return nil end
  local fonts = {}
  for i, bf in ipairs(vf.fonts) do
    local id = bf.name and vf_base_font(bf.name, bf.size or tfm.size)
    if not id then return nil end
    fonts[i] = { id = id }
  end
  return vf.characters, fonts
end

local function font_entry(f)
  local e = font_cache[f]
  if not e then
    local tfm = font.getfont(f)
    e = { tfm = tfm, chars = tfm and tfm.characters or {}, widths = {}, idx = {}, ew = {},
          virtual = tfm and tfm.type == "virtual" or false,
          fonts = tfm and tfm.fonts or nil }
    if tfm and not e.virtual then
      local vchars, vfonts = tfm_virtual(tfm)
      if vchars then e.chars, e.fonts, e.virtual = vchars, vfonts, true end
    end
    font_cache[f] = e
  end
  return e
end
-- Glyph index for the renderer; the advance width comes from the engine (node.direct.getwidth
-- on the glyph node), which is the integer TeX uses, not the possibly fractional font table value.
local function glyph_index(f, c)
  local e = font_entry(f)
  local gi = e.widths[c]
  if gi == nil then
    local ch = e.chars[c]
    gi = ch and ch.index or false
    e.widths[c] = gi
  end
  return gi or nil
end

-- Expansion: LuaTeX's glyph/kern expansion_factor (verified against PDF output in M1/E6).
local EXP_DENOM = 1000000
local function expand(w, ef)
  if ef == 0 then return w end
  return round_xn_over_d(w, EXP_DENOM + ef, EXP_DENOM)
end
local function set_expansion_denominator(d) EXP_DENOM = d end

-- Font descriptors for the "fonts" table of a display list. Cached per font id: font ids are
-- never reused within a process, and font.getfont() builds a fresh (large) table for TFM
-- fonts on every call.
local descriptor_cache = {}
local function font_descriptor(f)
  local d = descriptor_cache[f]
  if d then return d end
  local t = font_entry(f).tfm
  if not t then
    d = { id = f }
  else
    d = {
      id = f, name = t.name, fullname = t.fullname, psname = t.psname, filename = t.filename,
      format = t.format, type = t.type, size = t.size, designsize = t.designsize,
      slant = t.slant, extend = t.extend, squeeze = t.squeeze, subfont = t.subfont,
      encodingbytes = t.encodingbytes, embedding = t.embedding,
    }
  end
  descriptor_cache[f] = d
  return d
end

return {
  BILLION = BILLION,
  font_cache = font_cache,
  tex_round = tex_round,
  vet_glue = vet_glue,
  round_xn_over_d = round_xn_over_d,
  vf_base_font = vf_base_font,
  tfm_virtual = tfm_virtual,
  font_entry = font_entry,
  glyph_index = glyph_index,
  expand = expand,
  font_descriptor = font_descriptor,
  set_expansion_denominator = set_expansion_denominator,
}
