-- Opt-in host ownership. IDs are opaque: source trees and revisions stay with the host.
-- Loaded only when RTEX_MATH_OWNER_ATTRIBUTE is set by the session.
--
-- Observations are kept in flat arrays (no table per glyph or field) and packed straight
-- into the binary record 0x29; the JSON shape is built only for page files.
local M = {}
local attr = assert(tonumber(os.getenv('RTEX_MATH_OWNER_ATTRIBUTE')))
local D = node.direct
local getattribute = D.getattribute or D.get_attribute
local getid, getsubtype, getlist, getnext, getfield = D.getid, D.getsubtype, D.getlist, D.getnext, D.getfield
local getwhd, getfont, getchar = D.getwhd, D.getfont, D.getchar
local codec = package.loaded['rtex-math-codec']
if not codec then
  codec = dofile(kpse.find_file('rtex-math-codec.lua', 'lua') or 'rtex-math-codec.lua')
  package.loaded['rtex-math-codec'] = codec
end

local ID_HLIST, ID_VLIST, ID_WHATSIT = node.id('hlist'), node.id('vlist'), node.id('whatsit')
local ID_NOAD, ID_ACCENT, ID_FRACTION = node.id('noad'), node.id('accent'), node.id('fraction')
local ID_RADICAL, ID_SUB_MLIST = node.id('radical'), node.id('sub_mlist')
local SUB_USER_DEFINED = node.subtype('user_defined')

-- Field roles, as stable codes in the binary record and names in JSON.
local ROLE = { num = 1, den = 2, radicand = 3, sup = 4, sub = 5, degree = 6, cell = 7, math = 8 }

local box_role, fraction_subtype, cell_subtype = {}, nil, nil
local hlist_roles = { numerator = ROLE.num, denominator = ROLE.den, nucleus = ROLE.radicand,
  sup = ROLE.sup, sub = ROLE.sub, degree = ROLE.degree }
for code, name in pairs(node.subtypes('hlist')) do
  box_role[code] = hlist_roles[name]
  if name == 'fraction' then fraction_subtype = code elseif name == 'cell' then cell_subtype = code end
end
local marker_id = luatexbase.new_whatsit('math_owner', 'rtex')

-- Strides of the flat arrays: glyph = owner, font, char, x0, y0, x1, y1;
-- field = owner, role code, empty (0/1), x0, y0, x1, y1.
local GLYPH, FIELD = 7, 7

local function state(st)
  local mo = st.math_owners
  if not mo then
    mo = { g = {}, ng = 0, f = {}, nf = 0 }
    st.math_owners = mo
  end
  return mo
end

local function field(st, owner, role, x, y, w, h, d, empty)
  local mo = state(st)
  local f, i = mo.f, mo.nf * FIELD
  f[i + 1], f[i + 2], f[i + 3] = owner, role, empty and 1 or 0
  f[i + 4], f[i + 5], f[i + 6], f[i + 7] = x, y - h, x + w, y + d
  mo.nf = mo.nf + 1
end

local function is_marker(n)
  return getid(n) == ID_WHATSIT and getsubtype(n) == SUB_USER_DEFINED
    and getfield(n, 'user_id') == marker_id
end

local function list_owner(head)
  local n = head
  while n do
    local owner = getattribute(n, attr)
    if owner ~= nil then return owner end
    local id = getid(n)
    if id == ID_HLIST or id == ID_VLIST then
      owner = list_owner(getlist(n))
      if owner ~= nil then return owner end
    end
    n = getnext(n)
  end
end

function M.glyph(st, n, x, y, w)
  local owner = getattribute(n, attr)
  if owner == nil then return end
  local _, h, d = getwhd(n)
  local mo = state(st)
  local g, i = mo.g, mo.ng * GLYPH
  g[i + 1], g[i + 2], g[i + 3] = owner, getfont(n), getchar(n)
  g[i + 4], g[i + 5], g[i + 6], g[i + 7] = x, y - h, x + w, y + d
  mo.ng = mo.ng + 1
end

function M.box(st, n, x, y, w, h, d, parent)
  local role = box_role[getsubtype(n)]
  -- Repacking a narrower numerator/denominator to the fraction width can reset
  -- its subtype to unknown. Their order in the fraction vlist is still stable.
  if parent and getid(parent) == ID_VLIST and getsubtype(parent) == fraction_subtype then
    local child, first = getlist(parent), nil
    while child do
      if getid(child) == ID_HLIST then
        first = first or child
        if child == n then role = child == first and ROLE.num or ROLE.den; break end
      end
      child = getnext(child)
    end
  end
  if not role then return end
  local head = getlist(n)
  -- The marker carries a more precise zero-size anchor than its packed box (which
  -- can include scriptspace). Do not report the same empty field twice.
  if head and is_marker(head) then return end
  local owner = getattribute(n, attr)
  if owner == nil then owner = list_owner(head) end
  if owner ~= nil then field(st, owner, role, x, y, w, h, d, head == nil) end
end

-- Math-on/off pairs also bound inline formulas and matrix cells, including empty ones.
-- Pair within the containing hlist: nested lists have their own stack.
function M.boundary(st, n, box, x, y, h, d)
  local starts = st.math_starts
  if not starts then starts = {}; st.math_starts = starts end
  if getsubtype(n) == 0 then
    local mo = st.math_owners
    starts[box] = { getattribute(n, attr), x, mo and mo.ng or 0 }
    return
  end
  local start = starts[box]
  starts[box] = nil
  if not start then return end
  local owner = getattribute(n, attr) or start[1]
  local mo = st.math_owners
  if mo and mo.ng > start[3] then
    local g = mo.g
    local first = g[start[3] * GLYPH + 1]
    local uniform = true
    for i = (start[3] + 1) * GLYPH + 1, mo.ng * GLYPH, GLYPH do
      if g[i] ~= first then uniform = false; break end
    end
    if uniform then owner = first end
  end
  if owner == nil then return end
  local sx = start[2]
  local empty = sx == x
  field(st, owner, getsubtype(box) == cell_subtype and ROLE.cell or ROLE.math, sx, y, x - sx,
    empty and 0 or h, empty and 0 or d, empty)
end

-- Empty packed fields inherit the attribute state at conversion time, so their box's
-- attributes cannot identify them. Only truly empty sub-mlists get a zero-size marker.
-- Inserting anything before a nonempty field can change clean_box's italic correction.
-- Field lists per noad kind are static: no table is built while walking. A sub_mlist keeps
-- its list in `head`; set it with setfield, since setlist does not target that field.
local NOAD_FIELDS = { 'nucleus', 0, 'sup', ROLE.sup, 'sub', ROLE.sub }
local FRACTION_FIELDS = { 'num', ROLE.num, 'denom', ROLE.den }
local RADICAL_FIELDS = { 'nucleus', ROLE.radicand, 'degree', ROLE.degree, 'sup', ROLE.sup, 'sub', ROLE.sub }
local KIND_FIELDS = { [ID_NOAD] = NOAD_FIELDS, [ID_ACCENT] = NOAD_FIELDS,
  [ID_FRACTION] = FRACTION_FIELDS, [ID_RADICAL] = RADICAL_FIELDS }

local mark_list
local function mark_fields_of(n, fields)
  for i = 1, #fields, 2 do
    local child = getfield(n, fields[i])
    if child and getid(child) == ID_SUB_MLIST then
      local role, head = fields[i + 1], getfield(child, 'head')
      local owner = getattribute(child, attr)
      if owner == nil then owner = getattribute(n, attr) end
      if head == nil and role ~= 0 and owner ~= nil then
        local marker = D.new(ID_WHATSIT, SUB_USER_DEFINED)
        D.setfield(marker, 'user_id', marker_id)
        D.setfield(marker, 'type', 100)
        D.setfield(marker, 'value', role)
        -- Markers are rare (empty fields only). node.direct.setattribute on a fresh node without an
        -- attribute list corrupts the shared list and hangs TeX; the node API handles it.
        node.set_attribute(D.tonode(marker), attr, owner)
        D.setfield(child, 'head', marker)
      else
        mark_list(head)
      end
    end
  end
end

mark_list = function(n)
  while n do
    local id = getid(n)
    local fields = KIND_FIELDS[id]
    if fields then mark_fields_of(n, fields)
    elseif id == ID_SUB_MLIST then mark_list(getfield(n, 'head')) end
    n = getnext(n)
  end
end

luatexbase.add_to_callback('pre_mlist_to_hlist_filter', function(head)
  if head then mark_list(D.todirect(head)) end
  return head
end, 'rtex.math-owner')

function M.marker(st, n, x, y)
  if not is_marker(n) then return false end
  field(st, getattribute(n, attr), getfield(n, 'value'), x, y, 0, 0, 0, true)
  return true
end

M.encode, M.to_table = codec.encode, codec.to_table

return M
