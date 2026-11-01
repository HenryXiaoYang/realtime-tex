-- rtex-math-codec.lua: the 0x29 math-ownership record (docs/display-list.md) and its JSON
-- shape, for the flat arrays kept by rtex-math.lua. Pure Lua: no node library needed.
local M = {}
local pack, concat, unpack = string.pack, table.concat, table.unpack
local GLYPH, FIELD = 7, 7
local ROLE_NAME = { 'num', 'den', 'radicand', 'sup', 'sub', 'degree', 'cell', 'math' }
M.ROLE_NAME = ROLE_NAME

-- Binary record 0x29 (docs/display-list.md): u32 glyph count, glyphs as i4 owner, u4 font,
-- u4 char, i4 x0 y0 x1 y1; u32 field count, fields as i4 owner, u1 role, u1 empty, u2 0,
-- i4 x0 y0 x1 y1. Packed in chunks so a large page never exceeds Lua's argument limit.
local CHUNK = 128
local function pack_rows(parts, fmt, flat, rows, stride)
  local done = 0
  while done < rows do
    local k = math.min(CHUNK, rows - done)
    local first = done * stride + 1
    parts[#parts + 1] = pack('<' .. fmt:rep(k), unpack(flat, first, first + k * stride - 1))
    done = done + k
  end
end

function M.encode(mo)
  local parts = { pack('<I4', mo.ng) }
  pack_rows(parts, 'i4I4I4i4i4i4i4', mo.g, mo.ng, GLYPH)
  parts[#parts + 1] = pack('<I4', mo.nf)
  if mo.nf > 0 then
    -- Insert the u2 padding after role/empty: expand rows to i4 B B I2 i4 i4 i4 i4.
    local f, rows = mo.f, {}
    for r = 0, mo.nf - 1 do
      local i, o = r * FIELD, r * 8
      rows[o + 1], rows[o + 2], rows[o + 3], rows[o + 4] = f[i + 1], f[i + 2], f[i + 3], 0
      rows[o + 5], rows[o + 6], rows[o + 7], rows[o + 8] = f[i + 4], f[i + 5], f[i + 6], f[i + 7]
    end
    pack_rows(parts, 'i4BBI2i4i4i4i4', rows, mo.nf, 8)
  end
  return concat(parts)
end

-- The documented JSON shape, for page files written by the background pass.
function M.to_table(mo)
  local glyphs, fields, g, f = {}, {}, mo.g, mo.f
  for r = 0, mo.ng - 1 do
    local i = r * GLYPH
    glyphs[r + 1] = { owner = g[i + 1], font = g[i + 2], char = g[i + 3],
      bbox = { g[i + 4], g[i + 5], g[i + 6], g[i + 7] } }
  end
  for r = 0, mo.nf - 1 do
    local i = r * FIELD
    fields[r + 1] = { owner = f[i + 1], role = ROLE_NAME[f[i + 2]], empty = f[i + 3] == 1,
      bbox = { f[i + 4], f[i + 5], f[i + 6], f[i + 7] } }
  end
  return { glyphs = glyphs, fields = fields }
end

return M
