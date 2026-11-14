-- rtex-serve.lua: persistent paragraph server running inside a live LuaLaTeX document.
-- Requests arrive on stdin (Windows: on the named pipe $RTEX_REQ, opened in binary mode):
--   C <req> <ctx> <len>\n<len bytes>   compile <len> bytes of paragraph source (no JSON)
--   {...}\n                            any other op as a JSON object (context, ping, stats, ...)
-- Responses are frames on the FIFO (Windows: named pipe) named by $RTEX_RESP: u32-LE length, u8 kind, payload
-- (kind 0: JSON; kind 1: u32 json_len, JSON header, binary display list). See docs/engine-protocol.md.
local S = { contexts = {}, errors = {}, requests = 0 }
local json = dofile(kpse.find_file("rtex-json.lua", "lua") or "rtex-json.lua")
local dl = dofile(kpse.find_file("rtex-dl.lua", "lua") or "rtex-dl.lua")
local dlbin = dofile(kpse.find_file("rtex-dl-bin.lua", "lua") or "rtex-dl-bin.lua")
local P = dofile(kpse.find_file("rtex-pic.lua", "lua") or "rtex-pic.lua")
local gettime = os.gettimeofday
local pack, format, floor = string.pack, string.format, math.floor
local find, match, sub = string.find, string.match, string.sub
local stdin = io.stdin

local resp

local function send(tbl)
  local s = json.encode(tbl)
  resp:write(pack("<I4B", #s + 1, 0), s)
  resp:flush()
end
local function send_result_json(s, dlbytes)
  resp:write(pack("<I4B", 1 + 4 + #s + #dlbytes, 1), pack("<I4", #s), s, dlbytes)
  resp:flush()
end
local function send_result(tbl, dlbytes)
  send_result_json(json.encode(tbl), dlbytes)
end
local function send_error_result(req, ctx, message)
  send_result({ op = "result", req = req, ctx = ctx, status = "error", errors = { { message = message } },
                lines = 0, glyphs = 0, t_tex_us = 0, t_traverse_us = 0, t_pack_us = 0, dl_bytes = 0 }, "")
end

local function log(msg) texio.write_nl("log", "rtex-serve: " .. msg) end

-- Test seam: with $RTEX_TEST_FINISH_ERROR set to a marker string, a compile whose source
-- contains the marker fails inside finish (tests of the internal-error path).
local test_error_marker = os.getenv("RTEX_TEST_FINISH_ERROR")
if test_error_marker == "" then test_error_marker = nil end

-- Debug trace ($RTEX_TRACE, set by the session's debug setting): one line per request stage,
-- flushed at once, so the stage a hung request reached survives the process being killed
-- (the TeX log's tail does not).
local trace_file = os.getenv("RTEX_TRACE") and io.open(os.getenv("RTEX_TRACE"), "a") or nil
local function trace(msg)
  if not trace_file then return end
  trace_file:write(format("%.6f %s\n", gettime(), msg))
  trace_file:flush()
end

-- Error collection -------------------------------------------------------------------------
function S.on_error(...)
  S.errors[#S.errors + 1] = { message = status.lasterrorstring, context = status.lasterrorcontext,
                              line = tex.inputlineno }
end

local C = dofile(kpse.find_file("rtex-serve-context.lua", "lua") or "rtex-serve-context.lua")(S)
local IFTRUE_MODE, create, fp_scratch, getcount, ifmode, install_context, setglue, tex_set =
  C.IFTRUE_MODE, C.create, C.fp_scratch, C.getcount, C.ifmode, C.install_context, C.setglue, C.tex_set

local float_content = dofile(kpse.find_file("rtex-serve-source.lua", "lua") or "rtex-serve-source.lua").float_content

-- An error in a Lua function TeX calls during a compile is reported by TeX and swallowed in
-- batch mode: the compile goes on and, if the error was in S.finish, no result is ever sent,
-- so the host waits out its watchdog and kills the engine. Every such entry point therefore
-- runs protected; an error is traced (the trace survives the process) and recorded on the
-- request, and S.finish answers with an internal-error result instead of staying silent.
local function internal_error(cur, where, err)
  local msg = where .. ": " .. tostring(err)
  if trace_file then trace("internal error req " .. tostring(cur and cur.req) .. " in " .. msg) end
  log("internal error in " .. msg)
  if cur and not cur.internal then cur.internal = msg end
end

-- Called from TeX (\luafunction) inside the paragraph \vbox group, before any text.
local function apply_body(cur)
  cur.t_apply = gettime()
  local ctx = cur.ctx
  local a = ctx.a_ints
  for i = 1, #a, 2 do tex_set(a[i], a[i + 1]) end
  a = ctx.a_dims
  for i = 1, #a, 2 do tex_set(a[i], a[i + 1]) end
  a = ctx.a_glues
  for i = 1, #a do local g = a[i]; setglue(g[1], g[2], g[3], g[4], g[5], g[6]) end
  if ctx.parshape and #ctx.parshape > 0 then tex.parshape = ctx.parshape end
  cur.t_applied = gettime()
end
function S.apply()
  local cur = S.current
  if not cur then return end
  local ok, err = xpcall(apply_body, debug.traceback, cur)
  if not ok then internal_error(cur, "apply", err) end
end

-- The font selected at the server's outer level persists across requests; re-select only when
-- the context's NFSS state differs from the last one selected (saves ~0.3-0.5 ms per request).
local last_nfss = nil

-- Compile -----------------------------------------------------------------------------------------
-- A request is served in two halves without any nested TeX main loop (tex.runtoks leaks one
-- input level per call when invoked from a \directlua that never returns, E14):
--   step()   reads the request and *prints* the tokens that build the paragraph box, ending with
--            \luafunction<finish>; then returns to TeX, which executes them;
--   finish() traverses the box, checks the state fingerprint and sends the result.
-- The TeX side runs `\loop\rtexstep\ifnum\rtexcontinue>0 \repeat`.
S.current = nil
local head_tokens, tail_after_group  -- built in init once the \luafunction slots are known

-- Leak check: the meanings of the control sequences a unit's source mentions, before and
-- after its compile (every distinct \name in the source; a name built with \csname is not
-- seen — documented limitation). A unit that defines or \lets one of them (\newcommand inside a
-- paragraph, \global\let\emph\relax) changes the server for every later compile; the result
-- reports the names and the session demotes the unit and restarts the engine.
local function cs_names(source)
  local seen, names = {}, {}
  -- what gmatch("\\(%a+)") finds, with a plain search for each backslash (several times faster)
  local pos = 1
  while true do
    local b = find(source, "\\", pos, true)
    if not b then break end
    local name = match(source, "^%a+", b + 1)
    if name then
      if not seen[name] then seen[name] = true; names[#names + 1] = name end
      pos = b + 1 + #name
    else
      pos = b + 1
    end
  end
  return names
end
local function meaning_of(name)
  local ok, t = pcall(create, name)
  if not ok or not t then return "?" end
  local cmd = t.cmdname
  if cmd == "call" or cmd == "long_call" or cmd == "outer_call" or cmd == "long_outer_call" then
    local ok2, body = pcall(token.get_macro, name)
    return cmd .. ":" .. (ok2 and tostring(body) or "?")
  end
  return cmd .. ":" .. tostring(t.mode)
end
local function meanings_of(names)
  local m = {}
  for i = 1, #names do m[i] = meaning_of(names[i]) end
  return m
end

-- Picture cache in the live engine: a compile may come with cache entries for picture
-- environments of its source, each naming the 1-based source line its \begin starts (an
-- entry is matched by line, never by position: a picture an entry does not name is drawn).
-- The wrapped begin macro (rtex-pic.tex) takes the entry of the line it runs on when the
-- \begin starts that line (as the source scan keyed it: not a wrapper environment's inner
-- begin) and the font/color/width state matches, and places the cached region instead of
-- drawing. The result reports how many pictures were seen and how many came from the cache.
S.cache_images = {}
function S.pic_arm(env)
  local ok, err = pcall(P.arm, S.picctl, env)
  if not ok then internal_error(S.current, "pic_arm", err) end
end
function S.pic_begin(env)
  local cur = S.current
  local lookup = function(_, state)
    if not cur then return nil end
    cur.pics_seen = cur.pics_seen + 1
    -- line 1 of the input is the replay head, the source starts on line 2
    local ln = tex.inputlineno - 1
    local e = cur.pics_by_line and cur.pics_by_line[ln] or nil
    local text = cur.lines and cur.lines[ln] or ""
    if e and (e.state or "") == state and e.env == env
       and text:match("^%s*\\begin%s*{" .. env .. "}") then
      cur.pics_used = cur.pics_used + 1
      return e
    end
    return nil
  end
  local ok, err = xpcall(P.begin, debug.traceback, S.picctl, env, lookup, nil)
  if not ok then
    -- P.begin prints its continuation last: nothing was printed, so draw the picture
    internal_error(cur, "pic_begin", err)
    S.picctl.armed = nil
    S.picctl.pending = nil
    tex.sprint(S.cct, "\\csname rtex@orig@" .. env .. "\\endcsname")
  end
end
function S.pic_write()
  local ok, err = xpcall(P.write, debug.traceback, S.picctl, S.cache_images)
  if not ok then internal_error(S.current, "pic_write", err) end
end
function S.pic_end() end

function S.begin_compile(req_id, ctx_id, source, pics)
  local ctx = S.contexts[ctx_id]
  if not ctx then
    send_error_result(req_id, ctx_id, "unknown context " .. tostring(ctx_id))
    return
  end
  if not ctx.everypar_ok then
    send_error_result(req_id, ctx_id, "context has a non-replayable \\everypar")
    return
  end
  S.errors = {}
  S.margin_notes = 0
  local ftoks = ctx.nfss_tokens
  local font_changed = ftoks ~= last_nfss
  if font_changed then last_nfss = ftoks else ftoks = "" end
  if ctx.float then source = float_content(source, ctx.name) end
  local lines = {}
  local n = 0
  -- the lines of the source (a plain search: gmatch("(.-)\n") is several times slower)
  local pos = 1
  while true do
    local e = find(source, "\n", pos, true)
    n = n + 1
    if not e then lines[n] = sub(source, pos); break end
    lines[n] = sub(source, pos, e - 1)
    pos = e + 1
  end
  if n > 0 and lines[n] == "" then lines[n] = nil end
  local pics_by_line = nil
  if type(pics) == "table" then
    pics_by_line = {}
    for _, e in ipairs(pics) do
      if type(e) == "table" and type(e.line) == "number" then pics_by_line[e.line] = e end
    end
  end
  S.current = { req = req_id, ctx_id = ctx_id, ctx = ctx, font_changed = font_changed, t0 = 0,
                t_read = S.t_read, t_decoded = S.t_decoded, names = cs_names(source),
                lines = lines, pics_by_line = pics_by_line, pics_seen = 0, pics_used = 0 }
  S.current.meanings = meanings_of(S.current.names)
  if test_error_marker and source:find(test_error_marker, 1, true) then S.current.test_error = true end
  S.images_used = false
  if trace_file then trace(format("begin req %s ctx %s bytes %d lines %d pics %s", tostring(req_id), tostring(ctx_id), #source, n, pics and #pics or 0)) end
  -- our own tokens use @ as a letter (kernel switches, float emulation); the source keeps the
  -- document's catcodes
  tex.print(S.cct, ftoks .. head_tokens .. ctx.head_extra)
  tex.print(lines)
  tex.print(S.cct, ctx.tail_extra .. "\\par\\egroup\\endgroup" .. ctx.after_group .. tail_after_group)
  S.current.t_printed = gettime()
end

-- graphicx hook (driver): image resource index -> file
S.images = {}
-- The graphicx hook passes \lastsavedimageresourceindex, which is the image's PDF object
-- number, while IMAGE display-list items carry the image rule's index (LuaTeX's image index,
-- counted over all images, cached pictures included). An \useimageresource in a scratch box
-- gives the rule, and so the index; it creates no image and ships nothing.
function S.image_rule_index(objnum)
  S._rule_index = nil
  tex.runtoks(function()
    tex.sprint("\\begingroup\\setbox0\\hbox{\\useimageresource " .. objnum ..
               "}\\directlua{rtex_serve._rule_index = rtex_serve.box_image_index(0)}\\endgroup")
  end)
  return S._rule_index or objnum
end
function S.box_image_index(b)
  local box = tex.box[b]
  if not box then return nil end
  for n in node.traverse_id(node.id("rule"), box.head) do
    if n.subtype == 2 and n.index then return n.index end
  end
  return nil
end
function S.image(index, file, page, pages)
  index = S.image_rule_index(index)
  S.images[tostring(index)] = { index = index, file = file, page = tonumber(page) or 1, pages = pages }
  S.images_used = true
end

-- \marginpar in a unit (hook in rtex-serve-patches.tex): LaTeX refuses it inside the unit box,
-- so the note never becomes an insert there; it is counted as one, like footnote text, so the
-- result says that the page builder places material of this unit.
function S.margin_note() S.margin_notes = (S.margin_notes or 0) + 1 end

function S.mark()
  local cur = S.current
  if cur then
    cur.t_mark = gettime()
    if trace_file then trace("mark (context replayed) req " .. tostring(cur.req)) end
    if cur.font_changed then
      -- the font-selection tokens ran at the outer level: re-baseline
      S.font_outer = font.current()
      S.fp_base = S.fingerprint()
    end
    cur.t0 = gettime()
  end
end

local HEADER_FMT = '{"op":"result","req":%d,"ctx":%d,"status":"%s","errors":[],"lines":%d,"glyphs":%d,%s' ..
  '"width":%d,"height":%d,"depth":%d,"t_tex_us":%d,"t_traverse_us":%d,"t_pack_us":0,"font_changed":%s,"dl_bytes":%d,' ..
  '"stages_us":{"decode":%d,"prepare":%d,"to_mark":%d,"mark_to_apply":%d,"apply":%d,"apply_to_finish":%d,"fingerprint":%d}}'

-- Counters back to their idle values (also the cleanup after an internal error: idempotent).
local function restore_counters(cur)
  local cb = S.counter_base
  local start = cur.ctx and cur.ctx.counters or {}
  local advanced = nil
  local reg = S.counter_reg
  for name, v in pairs(cb) do
    local r = reg[name]
    local now = getcount(r)
    if now ~= v then
      tex.setcount("global", r, v)
      if name ~= "page" and now ~= (start[name] or v) then
        advanced = advanced or {}
        advanced[name] = now
      end
    end
  end
  return advanced
end

local finish_body

function S.finish()
  local cur = S.current
  S.current = nil
  if not cur then return end
  if trace_file then trace(format("finish req %s errors %d", tostring(cur.req), #S.errors)) end
  local ok, err = true, nil
  if not cur.internal then ok, err = xpcall(finish_body, debug.traceback, cur) end
  if ok and not cur.internal then return end
  if not ok then internal_error(cur, "finish", err) end
  -- answer, whatever failed: the host must not wait out its watchdog. Counters go back to the
  -- idle values; state the error left behind is caught by the fingerprint (fatal: the host
  -- restarts the engine), anything else is reported as an internal error for this unit.
  pcall(restore_counters, cur)
  local fok, fp1 = pcall(S.fingerprint, fp_scratch)
  if not fok or font.current() ~= S.font_outer or not S.fp_equal(fp1, S.fp_base) then
    send{ op = "fatal", req = cur.req, reason = "state_mismatch", internal = cur.internal,
          before = S.fp_string(S.fp_base), after = fok and S.fp_string(fp1) or "?", errors = S.errors }
    resp:flush()
    os.exit(3)
  end
  send_result({ op = "result", req = cur.req, ctx = cur.ctx_id, status = "error", internal = cur.internal,
    errors = { { message = "internal error: " .. tostring(cur.internal):match("^[^\n]*") } },
    lines = 0, glyphs = 0, t_tex_us = 0, t_traverse_us = 0, t_pack_us = 0, dl_bytes = 0 }, "")
  S.requests = S.requests + 1
end

finish_body = function(cur)
  if cur.test_error then error("test: injected error in finish") end
  local t1 = gettime()
  -- counters the unit advanced globally (\refstepcounter, \stepcounter …) are reported
  -- (value at the end, when it differs from the replayed start value) and go back to the idle
  -- values
  cur.advanced = restore_counters(cur)
  if trace_file then trace("finish: counters done") end
  -- control sequences of the source whose meaning changed: a definition leaked
  local leaks = nil
  if cur.names then
    for i = 1, #cur.names do
      if meaning_of(cur.names[i]) ~= cur.meanings[i] then
        leaks = leaks or {}
        leaks[#leaks + 1] = cur.names[i]
      end
    end
  end
  cur.leaks = leaks
  if trace_file then trace("finish: leak check done") end
  local fp1 = S.fingerprint(fp_scratch)
  local t1b = gettime()
  if font.current() ~= S.font_outer or not S.fp_equal(fp1, S.fp_base) then
    send{ op = "fatal", req = cur.req, reason = "state_mismatch", before = S.fp_string(S.fp_base), after = S.fp_string(fp1), errors = S.errors }
    resp:flush()
    os.exit(3)
  end
  if trace_file then trace("finish: fingerprint done") end
  local box = tex.box[S.boxnum]
  local bytes, nlines, nglyphs, bw, bh, bd, flags = "", 0, 0, 0, 0, 0, nil
  if box then
    -- traversal and serialization fused: records are written while walking the nodes
    bytes, nlines, nglyphs, bw, bh, bd, flags = dl.paragraph_binary(box, cur.ctx.color, S.margin_notes)
  end
  local t2 = gettime()
  if trace_file then trace(format("finish: traversal done, %d bytes", #bytes)) end
  local st = (#S.errors > 0) and "error" or ((flags and next(flags)) and "ok_degraded" or "ok")
  local t_tex = floor((t1 - cur.t0) * 1e6 + 0.5)
  local t_trav = floor((t2 - t1b) * 1e6 + 0.5)
  if #S.errors > 0 then
    send_result({ op = "result", req = cur.req, ctx = cur.ctx_id, status = st, errors = S.errors, counters = cur.advanced, leaks = cur.leaks,
      lines = nlines, glyphs = nglyphs, width = bw, height = bh, depth = bd,
      t_tex_us = t_tex, t_traverse_us = t_trav, t_pack_us = 0, font_changed = cur.font_changed, dl_bytes = #bytes }, bytes)
  else
    local images = ""
    if S.images_used and next(S.images) then images = '"images":' .. json.encode(S.images) .. ',' end
    if cur.advanced then images = images .. '"counters":' .. json.encode(cur.advanced) .. ',' end
    if cur.leaks then images = images .. '"leaks":' .. json.encode(cur.leaks) .. ',' end
    if cur.pics_by_line then images = images .. '"pics_seen":' .. cur.pics_seen .. ',"pics_used":' .. cur.pics_used .. ',' end
    send_result_json(format(HEADER_FMT, cur.req, cur.ctx_id, st, nlines, nglyphs, images, bw, bh, bd, t_tex, t_trav,
      cur.font_changed and "true" or "false", #bytes,
      floor(((cur.t_decoded or 0) - (cur.t_read or 0)) * 1e6 + 0.5), floor(((cur.t_printed or 0) - (cur.t_decoded or 0)) * 1e6 + 0.5),
      floor(((cur.t_mark or 0) - (cur.t_printed or 0)) * 1e6 + 0.5),
      floor(((cur.t_apply or 0) - (cur.t0 or 0)) * 1e6 + 0.5), floor(((cur.t_applied or 0) - (cur.t_apply or 0)) * 1e6 + 0.5),
      floor((t1 - (cur.t_applied or 0)) * 1e6 + 0.5), floor((t1b - t1) * 1e6 + 0.5)), bytes)
  end
  if trace_file then trace("finish: result sent req " .. tostring(cur.req)) end
  S.requests = S.requests + 1
end

dofile(kpse.find_file("rtex-serve-profile.lua", "lua") or "rtex-serve-profile.lua")(S, dl, dlbin, send)

function S.stats()
  send{ op = "stats", requests = S.requests, font_nextid = font.nextid(), node_mem = status.node_mem_usage,
        grouplevel = tex.currentgrouplevel, nest = tex.nest.ptr, luastate = collectgarbage("count"),
        input_ptr = status.input_ptr, max_in_stack = status.max_in_stack, save_size = status.save_size,
        cs_count = status.cs_count, buf_size = status.buf_size }
end

function S.dispatch(req)
  if trace_file and req.op ~= "compile" then trace("op " .. tostring(req.op) .. " id " .. tostring(req.id)) end
  if req.op == "context" then
    install_context(req.id, req.ctx)
    send{ op = "ok", id = req.id }
  elseif req.op == "compile" then
    S.begin_compile(req.req, req.ctx, req.source or "")
  elseif req.op == "labels" then
    -- \newlabel / \bibcite definitions from the last pass's aux: \r@name, \b@key
    local n = 0
    for _, kv in ipairs(req.labels or {}) do
      token.set_macro(kv[1], kv[2], "global")
      n = n + 1
    end
    send{ op = "ok", id = n }
  elseif req.op == "ping" then
    send{ op = "pong" }
  elseif req.op == "stats" then
    S.stats()
  elseif req.op == "profile" then
    S.profile(req)
  else
    send{ op = "error", message = "unknown op " .. tostring(req.op) }
  end
end

-- Main loop ----------------------------------------------------------------------------------------
-- init(boxnum, countnum): open the FIFO, register hooks and \luafunction slots, send ready.
-- step(): read and dispatch ONE request, then return to TeX (which executes any printed tokens
-- and re-enters step() through the \loop in the driver). Sets \count<countnum> to 0 to stop.
function S.init(boxnum, countnum, cctnum)
  S.boxnum = boxnum
  S.countnum = countnum
  -- the request pipe first: the host waits for it only after the ready frame
  local req_path = os.getenv("RTEX_REQ")
  if req_path and req_path ~= "" then
    stdin = assert(io.open(req_path, "rb"), "cannot open request pipe " .. req_path)
  end
  local path = os.getenv("RTEX_RESP")
  local err
  resp, err = io.open(path, "wb")
  assert(resp, "cannot open response channel " .. tostring(path) .. ": " .. tostring(err))
  resp:setvbuf("full", 1 << 16)
  luatexbase.add_to_callback("show_error_hook", S.on_error, "rtex-serve")
  -- \luafunction slots: no chunk compilation per call, unlike \directlua{...}
  local fns = lua.get_functions_table()
  local base = #fns
  fns[base + 1] = function() S.step() end
  fns[base + 2] = function() S.mark() end
  fns[base + 3] = function() S.apply() end
  fns[base + 4] = function() S.finish() end
  S.fn_step, S.fn_mark, S.fn_apply, S.fn_finish = base + 1, base + 2, base + 3, base + 4
  token.set_macro("rtexstep", "\\luafunction" .. S.fn_step .. " ", "global")
  -- catcode table for the tokens we print around the source: LaTeX's catcodes with @ a letter
  -- (the driver allocates and saves it: \newcatcodetable\rtexcct{\makeatletter\savecatcodetable\rtexcct})
  S.cct = cctnum
  -- picture cache: wrap the picture environments' begin macros (rtex-pic.tex) and mark cached
  -- images for the display-list traversal
  S.picctl = P.new(cctnum)
  dl.pic_images = S.cache_images
  tex.print(cctnum, "\\def\\rtex@picns{rtex_serve}\\def\\rtex@picenvs{" .. table.concat(P.PICTURE_ENVS, ",") ..
            "}\\input{rtex-pic.tex}\\rtex@wrappicall ")
  -- LaTeX counters: idle values restored after every compile
  local ck = token.get_macro("cl@@ckpt") or ""
  for name in ck:gmatch("\\@elt%s*{([^}]*)}") do
    local ok, v = pcall(getcount, "c@" .. name)
    if ok then
      S.counter_names[#S.counter_names + 1] = name; S.counter_base[name] = v
      local okt, t = pcall(create, "c@" .. name)
      local reg = okt and t and t.cmdname == "assign_int" and t.index
      S.counter_reg[name] = (reg and getcount(reg) == v) and reg or ("c@" .. name)
      local okm, body = pcall(token.get_macro, "the" .. name)
      S.the_base[name] = okm and body or nil
    end
  end
  local nobreak_idle = ifmode("if@nobreak") == IFTRUE_MODE
  -- Not \globaldefs=-1 inside the box (tried as a universal leak barrier): LaTeX's own
  -- internals rely on \begingroup…\global…\endgroup to publish results (NFSS font definitions,
  -- enumitem's keyval, hyperref, array), so making every assignment local breaks fonts and
  -- tables. Leaks are caught by the state fingerprint and the counter restore instead.
  head_tokens = "\\luafunction" .. S.fn_mark .. " \\begingroup\\global\\setbox\\rtexbox\\vbox\\bgroup\\luafunction" .. S.fn_apply .. " "
  -- the title block's kernel macros (rtex-serve-patches.tex, 5) come back after every compile
  local restore_title = token.is_defined("rtex@restoretitle") and "\\rtex@restoretitle " or ""
  tail_after_group = (nobreak_idle and "\\global\\@nobreaktrue " or "\\global\\@nobreakfalse ") .. restore_title .. "\\luafunction" .. S.fn_finish .. " "
  S.fp_base = S.fingerprint()
  S.font_outer = font.current()
  send{ op = "ready", banner = status.banner, luatex_version = status.luatex_version,
        fingerprint = S.fp_string(S.fp_base), font_nextid = font.nextid() }
  log("ready")
  if trace_file then
    trace("ready; trace " .. tostring(os.getenv("RTEX_TRACE")))
    -- Debug extras, after "ready" and under pcall: nothing here may keep the server from
    -- starting. Font loads with their cost (a first use of a big font can take seconds): the
    -- define_font callback is wrapped whoever registered it (luaotfload, or luatexja which
    -- replaces luaotfload's). With $RTEX_TRACE_MACROS (opt-in) every macro expansion goes to
    -- the TeX log (\tracingmacros), so a loop shows in the log's last flushed block even though
    -- the killed process loses the tail; it makes heavy compiles many times slower and the log
    -- grows without bound, so it is not part of the default trace.
    local ok, err = pcall(function()
      local descs = luatexbase.callback_descriptions("define_font")
      local name = descs and descs[1]
      if not name then return end
      local prev = luatexbase.remove_from_callback("define_font", name)
      if not prev then return end
      luatexbase.add_to_callback("define_font", function(fname, size, id)
        local t = gettime()
        local f = prev(fname, size, id)
        trace(format("font %s size %s id %s: %.1f ms", tostring(fname), tostring(size), tostring(id), (gettime() - t) * 1000))
        return f
      end, name)
      local tm = os.getenv("RTEX_TRACE_MACROS")
      if tm and tm ~= "" and tm ~= "0" then
        tex.tracingmacros = 1
        tex.tracingonline = 0
        trace("macro tracing on")
      end
    end)
    if not ok then trace("debug extras not installed: " .. tostring(err)) end
  end
end

function S.stop()
  tex.setcount(S.countnum, 0)
  if resp then resp:close() end
end

local function handle_json(line)
  local ok, req = pcall(json.decode, line)
  S.t_decoded = gettime()
  if not ok then
    send{ op = "error", message = "bad request: " .. tostring(req) }
  elseif req.op == "shutdown" then
    send{ op = "bye" }
    S.stop()
  else
    local hok, herr = pcall(S.dispatch, req)
    if not hok then
      log("handler error: " .. tostring(herr))
      S.current = nil
      if req.op == "compile" then
        send_error_result(req.req, req.ctx, "lua: " .. tostring(herr))
      else
        send{ op = "error", message = "lua: " .. tostring(herr) }
      end
    end
  end
end

function S.step()
  local line = stdin:read("l")
  S.t_read = gettime()
  if not line then log("stdin closed"); S.stop(); return end
  local c = line:byte(1)
  if c == 67 then -- 'C': compile frame
    local req_id, ctx_id, len, plen = line:match("^C (%d+) (%-?%d+) (%d+) ?(%d*)$")
    if not req_id then
      send{ op = "error", message = "bad compile frame: " .. line }
      return
    end
    len = tonumber(len)
    plen = tonumber(plen) or 0
    local source = len > 0 and stdin:read(len) or ""
    if not source or #source ~= len then log("stdin closed mid-frame"); S.stop(); return end
    -- optional picture cache entries (JSON array) after the source
    local pics = nil
    if plen > 0 then
      local pj = stdin:read(plen)
      if not pj or #pj ~= plen then log("stdin closed mid-frame"); S.stop(); return end
      local ok, v = pcall(json.decode, pj)
      if ok and type(v) == "table" then pics = v end
    end
    S.t_decoded = gettime()
    local hok, herr = xpcall(S.begin_compile, debug.traceback, tonumber(req_id), tonumber(ctx_id), source, pics)
    if not hok then
      log("handler error: " .. tostring(herr))
      if trace_file then trace("internal error req " .. tostring(req_id) .. " in begin_compile: " .. tostring(herr)) end
      S.current = nil
      send_result({ op = "result", req = tonumber(req_id), ctx = tonumber(ctx_id), status = "error",
        internal = "begin_compile: " .. tostring(herr),
        errors = { { message = "internal error: begin_compile: " .. tostring(herr):match("^[^\n]*") } },
        lines = 0, glyphs = 0, t_tex_us = 0, t_traverse_us = 0, t_pack_us = 0, dl_bytes = 0 }, "")
    end
  elseif c == nil then
    return
  else
    handle_json(line)
  end
end

return S
