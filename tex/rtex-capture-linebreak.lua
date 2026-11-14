-- rtex-capture-linebreak.lua: for rtex-capture.lua, what the capture records when TeX breaks
-- a paragraph into lines: the parameters in force before it (C.pre_linebreak) and the line
-- count and flags after it (C.post_linebreak).
--
-- Returns a function that takes the capture table C and what the callbacks use from
-- rtex-capture.lua, and defines the two callbacks on C.
return function(C, params)
local GLYPH, WHATSIT, WHATSITS = node.id("glyph"), node.id("whatsit"), node.whatsits()
local FLAGGED = {}
for _, name in ipairs({ "ins", "mark", "adjust", "dir", "hlist", "vlist", "rule", "math" }) do FLAGGED[node.id(name)] = name end
local function scan_flags(head)
  local flags, glyphs = {}, 0
  for _, t, sub in node.direct.traverse(node.direct.todirect(head)) do
    if t == GLYPH then glyphs = glyphs + 1
    elseif t == WHATSIT then
      local st = WHATSITS[sub] or tostring(sub)
      flags["whatsit_" .. st] = (flags["whatsit_" .. st] or 0) + 1
    elseif FLAGGED[t] then
      local name = FLAGGED[t]
      flags[name] = (flags[name] or 0) + 1
    end
  end
  flags.glyphs = glyphs
  return flags
end

function C.pre_linebreak(head, groupcode)
  C.seq = C.seq + 1
  local seq = C.seq
  local nest = tex.nest.ptr
  local p = { seq = seq, groupcode = groupcode, nest = nest, end_line = tex.inputlineno, file = status.filename }
  p.ints, p.dims, p.glues = params()
  p.parshape = tex.parshape
  p.everypar = tex.gettoks("everypar")
  p.flags = scan_flags(head)
  -- pair with the most recent para/begin at the same nesting level
  local b = C.begins
  local top = b[#b]
  if top and top.nest == nest then
    p.begin = top
    b[#b] = nil
  end
  local cur = C.cur
  -- paragraphs built inside the output routine (fancyhdr's running head and foot are
  -- paragraphs in boxes) belong to the page, not to the unit that happened to be open
  if cur and not C.in_output then
    p.unit = cur.uid
    cur.seqs[#cur.seqs + 1] = seq
  end
  C.paras[seq] = p
  C.stack[#C.stack + 1] = seq
  return true
end

function C.post_linebreak(head, groupcode)
  local seq = C.stack[#C.stack]
  C.stack[#C.stack] = nil
  local p = C.paras[seq]
  local i = 0
  local hl = node.id("hlist")
  local unit = p and p.unit
  for n in node.traverse(head) do
    if n.id == hl then
      i = i + 1
      node.set_attribute(n, C.attr_par, seq)
      node.set_attribute(n, C.attr_line, i)
      -- the line boxes are built here, possibly after the group the paragraph started in
      if unit then node.set_attribute(n, C.attr_unit, unit) end
    end
  end
  if p then p.lines = i end
  return true
end
end
