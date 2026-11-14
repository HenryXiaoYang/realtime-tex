-- rtex-serve-source.lua: for rtex-serve.lua, reading a unit's source text: a line without its
-- comment, where the vertical material a unit starts with ends, and the content of a float
-- environment.
-- A line without its % comment (\% is a percent sign).
local function uncommented(line)
  local i = 1
  while true do
    local j = line:find("[\\%%]", i)
    if not j then return line end
    if line:byte(j) == 37 then return line:sub(1, j - 1) end
    i = j + 2
  end
end

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

-- Strip the \begin{float}[opt] ... \end{float} wrapper: the content is typeset directly in the
-- unit box with the float-box state replayed by head_extra. Blank lines, comment lines and
-- vertical material before the \begin act on the page, not on the float box, and go too, as
-- do comments after the \end; what goes before the content is replaced by its newlines, so
-- the content keeps its source line numbers (cur.lines, pics_by_line).
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
  local b = source:match("^\\begin%s*{" .. n .. "}%s*%b[]()", p) or source:match("^\\begin%s*{" .. n .. "}()", p)
  if not b then return source end
  -- the \end ends the last line with anything before its comment
  local inner, last = source:sub(b), nil
  for s, line in (inner .. "\n"):gmatch("()([^\n]*)\n") do
    if uncommented(line):find("%S") then last = s end
  end
  local head = last and uncommented(inner:match("^[^\n]*", last)):match("^(.-)\\end%s*{" .. n .. "}%s*$")
  if not head then return source end
  return source:sub(1, b - 1):gsub("[^\n]+", "") .. inner:sub(1, last - 1) .. head
end

return { float_content = float_content }
