-- rtex-serve-context.lua: for rtex-serve.lua, the state a compile starts from and must leave
-- behind. The engine-state fingerprint (S.fingerprint), and replaying a unit's captured context
-- (install_context): its parameters, fonts, counters and macro meanings, against the bases
-- recorded on S.
--
-- Returns a function that takes the server table S, defines S.fingerprint and friends on it,
-- and returns what rtex-serve.lua uses directly.
local format = string.format

return function(S)
-- Engine-state fingerprint ------------------------------------------------------------------
-- Everything a paragraph compile must leave untouched. The current font is excluded: it is
-- legitimately changed at the outer level by the font-selection tokens, and tracked separately.
local FP_CATCODES = { 92, 123, 125, 36, 38, 35, 94, 95, 37, 126 }
local tex_get, getcatcode, getcount, gettoks = tex.get, tex.getcatcode, tex.getcount, tex.gettoks
local create = token.create
local IFTRUE_MODE = create("iftrue").mode
-- Token objects report the *current* meaning of their control sequence (E24), so the kernel
-- conditionals are created once and only their mode is read per compile.
local FP_IFS = {}
local function ifmode(name)
  local ok, t = pcall(create, name)
  return ok and t and t.mode or -1
end
local function fp_if(i, name)
  local t = FP_IFS[i]
  if t == nil then
    local ok, tok = pcall(create, name)
    t = ok and tok or false
    FP_IFS[i] = t
  end
  return t and t.mode or -1
end
local FP_N = 10 + #FP_CATCODES + 10
-- The fingerprint is an array (compared element-wise against the baseline: no string building
-- on the hot path); S.fp_string renders it for the fatal report.
function S.fingerprint(fp)
  fp = fp or {}
  fp[1], fp[2], fp[3] = tex.currentgrouplevel, tex.nest.ptr, tex.interactionmode
  fp[4], fp[5], fp[6], fp[7] = gettoks("everypar"), tex_get("hsize"), tex_get("parindent"), tex_get("language")
  fp[8], fp[9], fp[10] = fp_if(1, "if@inlabel"), fp_if(2, "if@newlist"), fp_if(3, "if@minipage")
  local n = 10
  for i = 1, #FP_CATCODES do n = n + 1; fp[n] = getcatcode(FP_CATCODES[i]) end
  for i = 0, 9 do n = n + 1; fp[n] = getcount(i) end
  return fp
end
function S.fp_equal(a, b)
  for i = 1, FP_N do if a[i] ~= b[i] then return false end end
  return true
end
function S.fp_string(fp) return table.concat(fp, "|", 1, FP_N) end
local fp_scratch = {}
-- Baseline taken once the server is idle at the outer level (init, and again after each
-- font change at the outer level); every compile must end in this state.
S.fp_base = nil
S.font_outer = nil

-- Context replay -------------------------------------------------------------------------------
-- \everypar contents that may be replayed verbatim (anything else makes the unit
-- background-only on the host side; the server refuses it too): the kernel's own patterns after
-- a heading (\@afterheading), after an environment (\@doendpe), in a box (\@setminipage), and
-- microtype's \leftprotrusion.
local EVERYPAR_PATTERNS = {
  [""] = true,
  ["\\if@nobreak \\@nobreakfalse \\clubpenalty \\@M \\if@afterindent \\else {\\setbox \\z@ \\lastbox }\\fi \\else \\clubpenalty \\@clubpenalty \\everypar {}\\fi "] = true,
  ["\\if@nobreak \\@nobreakfalse \\clubpenalty \\@M \\setbox \\z@ \\lastbox \\else \\clubpenalty \\@clubpenalty \\everypar {}\\fi "] = true,
  ["{\\setbox \\z@ \\lastbox }\\everypar {}\\@endpefalse "] = true,
  ["\\@minipagefalse \\everypar {}"] = true,
}
local function everypar_allowed(ep)
  local e = ep:gsub("^\\leftprotrusion ", ""):gsub("\\leftprotrusion $", "")
  return EVERYPAR_PATTERNS[e] or false
end
local FLOAT_ENVS = { figure = "columnwidth", table = "columnwidth", ["figure*"] = "textwidth", ["table*"] = "textwidth" }
-- LaTeX counters (c@<name>) known at start-up; replayed per unit and restored after each compile
S.counter_names = {}
S.counter_base = {}
-- name -> what tex.getcount takes for it: the register number of a \countdef'd counter (no
-- name lookup per compile), else the name of its count
S.counter_reg = {}
S.the_base = {}
-- idle meanings of the macros the capture reports (looked up lazily)
-- \iftrue / \iffalse for a \newif switch (no token.get_meaning for a primitive), else nil
local IFTRUE, IFFALSE
local function if_meaning(name)
  local ok, t = pcall(create, name)
  if not ok or not t or t.cmdname ~= "if_test" then return nil end
  IFTRUE = IFTRUE or create("iftrue")
  IFFALSE = IFFALSE or create("iffalse")
  if t.mode == IFTRUE.mode then return "\\iftrue" elseif t.mode == IFFALSE.mode then return "\\iffalse" end
  return nil
end
S.if_meaning = if_meaning
S.macro_base = setmetatable({}, { __index = function(t, name)
  local m = nil
  if token.get_meaning then
    local ok, mm = pcall(token.get_meaning, name)
    if ok then m = mm end
  end
  if m == nil then
    local ok, b = pcall(token.get_macro, name)
    m = ok and b and ("->" .. b) or if_meaning(name) or false
  end
  rawset(t, name, m)
  return m
end })
-- token.get_meaning gives "#1#2->body" (no "macro:" prefix, no \long): rebuild the
-- definition as \def\name#1#2{body}, \long when the server's current meaning is a long macro
-- or the name is undefined here (\newcommand defines long macros); nil when not a macro meaning
function S.def_from_meaning(name, m)
  local params, body = m:match("^(.-)%->(.*)$")
  if not params then return nil end
  local pre = "\\long"
  local ok, t = pcall(create, name)
  if ok and t and t.cmdname == "call" then pre = "" end
  return pre .. "\\def\\" .. name .. params .. "{" .. body .. "}"
end

-- Contexts are preprocessed when installed: parameter tables become flat arrays, and the
-- NFSS selection string is built once.
local tex_set, setglue = tex.set, tex.setglue
local function install_context(id, ctx)
  local ints, dims, glues = {}, {}, {}
  -- Contexts are installed while the server idles at the outer level, and the unit box
  -- inherits that level's parameters, so only integer and dimen parameters that differ from
  -- the idle values need replaying. Glue parameters are always replayed: \selectfont at the
  -- outer level (font changes between requests) rewrites \baselineskip.
  for k, v in pairs(ctx.ints or {}) do
    local ok, cur = pcall(tex_get, k)
    if not ok or cur ~= v then ints[#ints + 1] = k; ints[#ints + 1] = v end
  end
  for k, v in pairs(ctx.dims or {}) do
    local ok, cur = pcall(tex_get, k)
    if not ok or cur ~= v then dims[#dims + 1] = k; dims[#dims + 1] = v end
  end
  for k, v in pairs(ctx.glues or {}) do glues[#glues + 1] = { k, v[1] or 0, v[2] or 0, v[3] or 0, v[4] or 0, v[5] or 0 } end
  ctx.a_ints, ctx.a_dims, ctx.a_glues = ints, dims, glues
  -- tokens after \bgroup: counters (TeX assignments, local to the group; `page` is never
  -- replayed), \if@nobreak and friends, \everypar replay, float-box emulation
  local kind, name = ctx.kind or "par", ctx.name
  local extra = {}
  local names = {}
  for k, v in pairs(ctx.counters or {}) do
    -- only counters that are valid control words (biblatex has `bbx:relatedcount`), that the
    -- server knows, that differ from its idle value, and never the page counter
    if S.counter_base[k] ~= nil and S.counter_base[k] ~= v and k ~= "page" and k:match("^[%a@]+$") then names[#names + 1] = k end
  end
  table.sort(names)
  for _, k in ipairs(names) do extra[#extra + 1] = format("\\c@%s=%d ", k, ctx.counters[k]) end
  -- \the<counter> formats that differ from the idle ones (\appendix, \renewcommand{\thesection})
  local fnames = {}
  for k, v in pairs(ctx.thefmt or {}) do
    if S.the_base[k] ~= nil and S.the_base[k] ~= v and k:match("^[%a@]+$") then fnames[#fnames + 1] = k end
  end
  table.sort(fnames)
  for _, k in ipairs(fnames) do extra[#extra + 1] = format("\\def\\the%s{%s}", k, ctx.thefmt[k]) end
  -- macros the body (re)defines, in the meaning they had at the unit ("macro:#1->body")
  local mnames = {}
  for k, v in pairs(ctx.macros or {}) do
    if k:match("^[%a@]+$") and S.macro_base[k] ~= v then mnames[#mnames + 1] = k end
  end
  table.sort(mnames)
  for _, k in ipairs(mnames) do
    local d = S.def_from_meaning(k, ctx.macros[k])
    if d then extra[#extra + 1] = d end
  end
  -- \newif switches (glossaries' first-use \ifglo@<label>@flag): their value at the unit,
  -- local to the compile; a compile that sets one globally (\gls) has it reset to the idle
  -- value right after its group, so no compile sees another's first use
  local snames, after = {}, {}
  for k, v in pairs(ctx.macros or {}) do
    if (v == "\\iftrue" or v == "\\iffalse") and k:match("^[%w@:%-_.]+$") then snames[#snames + 1] = k end
  end
  table.sort(snames)
  for _, k in ipairs(snames) do
    local base = S.macro_base[k]
    if base == "\\iftrue" or base == "\\iffalse" then
      if base ~= ctx.macros[k] then
        extra[#extra + 1] = "\\expandafter\\let\\csname " .. k .. "\\endcsname" .. ctx.macros[k] .. " "
      end
      after[#after + 1] = "\\expandafter\\global\\expandafter\\let\\csname " .. k .. "\\endcsname" .. base .. " "
    end
  end
  ctx.after_group = table.concat(after)
  extra[#extra + 1] = ctx.nobreak and "\\@nobreaktrue " or "\\@nobreakfalse "
  extra[#extra + 1] = ctx.afterindent and "\\@afterindenttrue " or "\\@afterindentfalse "
  extra[#extra + 1] = ctx.noskipsec and "\\@noskipsectrue " or "\\@noskipsecfalse "
  local ep = ctx.everypar or ""
  if ep ~= "" then extra[#extra + 1] = "\\everypar{" .. ep .. "}" end
  ctx.float = kind == "env" and name and FLOAT_ENVS[name] or nil
  if ctx.float then
    -- what \@xfloat does to the float box content; \@floatboxreset is the kernel's
    -- \reset@font\normalsize\@setminipage, or the class's version replayed with the macros above
    extra[#extra + 1] = "\\def\\@captype{" .. name:gsub("%*$", "") .. "}\\hsize\\" .. ctx.float .. "\\@parboxrestore\\@floatboxreset "
  end
  ctx.head_extra = table.concat(extra)
  ctx.tail_extra = ctx.float and "\\par\\vskip\\z@skip" or ""
  local b = ctx.begin
  local n = b and b.nfss
  if n and n.family then
    ctx.nfss_tokens = format("\\fontencoding{%s}\\fontfamily{%s}\\fontseries{%s}\\fontshape{%s}\\fontsize{%s}{%s}\\selectfont",
      n.enc or "TU", n.family, n.series or "m", n.shape or "n", n.size or "10", n.baselineskip or "12pt")
  else
    ctx.nfss_tokens = ""
  end
  ctx.color = b and b.color
  if ctx.color == "0 g 0 G" or ctx.color == "" then ctx.color = nil end
  ctx.everypar_ok = everypar_allowed(ctx.everypar or "")
  S.contexts[id] = ctx
end

return {
  IFTRUE_MODE = IFTRUE_MODE,
  create = create,
  fp_scratch = fp_scratch,
  getcount = getcount,
  ifmode = ifmode,
  install_context = install_context,
  setglue = setglue,
  tex_set = tex_set,
}
end
