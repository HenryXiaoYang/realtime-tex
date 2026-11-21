-- rtex-serve-source.lua: for rtex-serve.lua, reading a unit's source text: where the vertical
-- material a unit starts with ends, and where a float environment begins.
-- The vertical material a unit may start with (eligibility/shape.rs vertical_prefix_len): where the
-- command at p and its {argument} end, or nil.
local VERTICAL = { "vspace%*", "vspace", "noindent", "newpage", "clearpage", "pagebreak",
                   "smallskip", "medskip", "bigskip", "vfill", "centering" }
local function vertical_end(s, p)
  for _, v in ipairs(VERTICAL) do
    local q = s:match("^\\" .. v .. "()", p)
    if q and not s:find("^%a", q) then return s:match("^%b{}()", q) or q end
  end
end

-- The float environment of a unit's source, from its \begin: blank lines, comment lines and
-- vertical material before the \begin act on the page, not on the float box, and go; they are
-- replaced by their newlines, so the content keeps its source line numbers (cur.lines,
-- pics_by_line). A source that does not start with the environment is returned as it is.
local function float_content(source, name)
  local n = name:gsub("%*", "%%*")
  local p = 1
  while true do
    p = source:match("^%s*()", p)
    if source:find("^%%", p) then
      p = source:match("^[^\n]*()", p)
    else
      local q = vertical_end(source, p)
      if not q then break end
      p = q
    end
  end
  if not source:find("^\\begin%s*{" .. n .. "}", p) then return source end
  return source:sub(1, p - 1):gsub("[^\n]+", "") .. source:sub(p)
end

return { float_content = float_content }
