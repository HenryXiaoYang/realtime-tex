-- rtex-serve-profile.lua: the `profile` op of rtex-serve.lua (rtex probe): how long replaying a
-- context, typesetting and traversing a unit take inside the engine.
--
-- Returns a function that takes the server table S and what the op uses from rtex-serve.lua,
-- and defines S.profile.
local gettime = os.gettimeofday
local pack, format, floor = string.pack, string.format, math.floor

return function(S, dl, dlbin, send)
-- Profile the replay overhead (diagnostic only; uses tex.runtoks, which leaks one input level per
-- call, so do not run it thousands of times in a long-lived server).
function S.profile(req)
  local ctx = S.contexts[req.ctx]
  local n = req.n or 50
  local function med(t) table.sort(t); return floor(t[(#t + 1) // 2] * 1e6 + 0.5) end
  local ptrs = {}
  local function timeit(f)
    local r = {}
    local p0 = status.input_ptr
    for _ = 1, n do local a = gettime(); f(); r[#r + 1] = gettime() - a end
    ptrs[#ptrs + 1] = format("%d->%d", p0, status.input_ptr)
    return med(r)
  end
  local lines = {}
  for line in ((req.source or "") .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line end
  local out = {}
  S.current = { ctx = ctx }
  out.empty_runtoks = timeit(function() tex.runtoks(function() tex.print("\\relax") end) end)
  out.apply_only = timeit(function() S.apply() end)
  out.fingerprint = timeit(function() S.fingerprint() end)
  out.group_box_empty = timeit(function()
    tex.runtoks(function() tex.print("\\begingroup\\global\\setbox\\rtexbox\\vbox\\bgroup\\luafunction" .. S.fn_apply .. " \\par\\egroup\\endgroup") end)
  end)
  out.print_source_only = timeit(function()
    tex.runtoks(function() tex.print("\\begingroup\\global\\setbox\\rtexbox\\vbox\\bgroup") tex.print(lines) tex.print("\\par\\egroup\\endgroup") end)
  end)
  out.full_compile = timeit(function()
    tex.runtoks(function()
      tex.print(S.cct, "\\begingroup\\global\\setbox\\rtexbox\\vbox\\bgroup\\luafunction" .. S.fn_apply .. " " .. (ctx.head_extra or ""))
      tex.print(lines)
      tex.print(S.cct, (ctx.tail_extra or "") .. "\\par\\egroup\\endgroup")
    end)
  end)
  S.current = nil
  -- bare walk: node ids and next pointers only, recursing into boxes (lower bound for traversal)
  local D = node.direct
  local getid, getnext, getlist = D.getid, D.getnext, D.getlist
  local hl, vl, gl = node.id("hlist"), node.id("vlist"), node.id("glyph")
  local counts = { nodes = 0, glyphs = 0, boxes = 0 }
  local function bare(list)
    local n = list
    while n do
      local id = getid(n)
      counts.nodes = counts.nodes + 1
      if id == hl or id == vl then counts.boxes = counts.boxes + 1; bare(getlist(n))
      elseif id == gl then counts.glyphs = counts.glyphs + 1 end
      n = getnext(n)
    end
  end
  out.walk_only = timeit(function() counts = { nodes = 0, glyphs = 0, boxes = 0 }; bare(getlist(D.todirect(tex.box[S.boxnum]))) end)
  out.node_counts = counts
  local getfont, getchar, getexpansion, getoffsets, getwidth = D.getfont, D.getchar, D.getexpansion, D.getoffsets, D.getwidth
  local pack = string.pack
  local function fields(list)
    local n = list
    while n do
      local id = getid(n)
      if id == hl or id == vl then fields(getlist(n))
      elseif id == gl then local f, c, ef = getfont(n), getchar(n), getexpansion(n); local xo, yo = getoffsets(n); local w = getwidth(n); local s = pack("<I4I4i4i4", c, 1, w, w) end
      n = getnext(n)
    end
  end
  out.walk_fields_pack = timeit(function() fields(getlist(D.todirect(tex.box[S.boxnum]))) end)
  -- section timing of the binary traversal
  do
    local box = D.todirect(tex.box[S.boxnum])
    local hl2 = node.id("hlist")
    local lines_t, nl = 0, 0
    local gt = os.gettimeofday
    local a = gt()
    for _ = 1, n do
      local st = dl._new_state_bin()
      local m = getlist(box)
      local cur_v = 0
      while m do
        if getid(m) == hl2 then
          local w, h, d = D.getwhd(m)
          cur_v = cur_v + h
          local b = gt()
          st:begin_line(m, 0, 1, 0, cur_v, w, h, d)
          dl._hlist_out(st, m, 0, cur_v)
          st:end_line()
          lines_t = lines_t + (gt() - b)
          nl = nl + 1
          cur_v = cur_v + d
        end
        m = getnext(m)
      end
      st:bin_flush_run()
    end
    out.sections = { per_line_us = math.floor(lines_t / nl * 1e6 + 0.5), lines = nl // n, loop_total_us = math.floor((gt() - a) / n * 1e6 + 0.5) }
  end
  out.traverse = timeit(function() dl.paragraph(tex.box[S.boxnum]) end)
  out.traverse_binary = timeit(function() dl.paragraph_binary(tex.box[S.boxnum]) end)
  collectgarbage("stop")
  out.traverse_binary_nogc = timeit(function() dl.paragraph_binary(tex.box[S.boxnum]) end)
  out.walk_fields_pack_nogc = timeit(function() fields(getlist(D.todirect(tex.box[S.boxnum]))) end)
  out.full_compile_nogc = timeit(function()
    tex.runtoks(function()
      tex.print("\\begingroup\\global\\setbox\\rtexbox\\vbox\\bgroup\\luafunction" .. S.fn_apply .. " ")
      tex.print(lines)
      tex.print("\\par\\egroup\\endgroup")
    end)
  end)
  collectgarbage("restart")
  out.lua_heap_kb = math.floor(collectgarbage("count"))
  local r = dl.paragraph(tex.box[S.boxnum])
  out.encode = timeit(function() dlbin.encode(r) end)
  out.n = n
  send{ op = "profile", req = req.req, us = out, input_ptr_track = ptrs, input_ptr = status.input_ptr }
end
end
