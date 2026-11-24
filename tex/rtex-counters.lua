-- rtex-counters.lua: the counters the capture records per unit and the live server replays and
-- restores (shared, so both sides use the same names).
--
-- LaTeX counters come from \cl@@ckpt, which every \newcounter extends. Packages also number
-- things with a count register named like a counter but made with \newcount\c@name (ltmermaid's
-- \c@mermaid@diag, which names its generated PDFs). Such a register is not in \cl@@ckpt; left out,
-- each live compile advances it from wherever the previous one left it.
local M = {}

-- Names (without the c@ prefix): the \cl@@ckpt counters in their order, then the other count
-- registers whose control sequence is c@<letters or @>, sorted.
function M.names()
  local names, seen = {}, {}
  local ck = token.get_macro("cl@@ckpt") or ""
  for n in ck:gmatch("\\@elt%s*{([^}]*)}") do
    if not seen[n] then seen[n] = true; names[#names + 1] = n end
  end
  local extra = {}
  for _, cs in ipairs(tex.hashtokens()) do
    local n = cs:match("^c@([%a@]+)$")
    if n and not seen[n] then
      local ok, t = pcall(token.create, cs)
      if ok and t and t.cmdname == "assign_int" then seen[n] = true; extra[#extra + 1] = n end
    end
  end
  table.sort(extra)
  for i = 1, #extra do names[#names + 1] = extra[i] end
  return names
end

return M
