-- rtex-dl-walk.lua: the box traversal of rtex-dl.lua. hlist_out and vlist_out replicate
-- LuaTeX's functions of the same names, position for position, and hand what they find to the
-- state's sink (State:emit and friends in rtex-dl.lua). They are long because they follow the
-- engine's own structure: one branch per node type, in the engine's order.
--
-- Returns a function: given the State class and the font module (rtex-dl-fonts.lua), it adds
-- State:emit_virtual and returns hlist_out and vlist_out.
local math_owner
if os.getenv("RTEX_MATH_OWNER_ATTRIBUTE") then
  math_owner = package.loaded["rtex-math"]
  if not math_owner then
    math_owner = dofile(kpse.find_file("rtex-math.lua", "lua") or "rtex-math.lua")
    package.loaded["rtex-math"] = math_owner
  end
end

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

return function(State, F)
local BILLION, font_cache, tex_round, vet_glue, font_entry, glyph_index, expand =
  F.BILLION, F.font_cache, F.tex_round, F.vet_glue, F.font_entry, F.glyph_index, F.expand

local function silent_whatsit(sub)
  return SILENT[sub] == true
end
-- LuaTeX's PDF backend draws only specials that start with "pdf:" or "PDF:" (case and leading
-- spaces matter) and drops the rest with "Non-PDF special ignored!", e.g. the papersize=
-- special a class writes under LuaLaTeX
local function drawn_special(data)
  local p = (data or ""):sub(1, 4)
  return p == "pdf:" or p == "PDF:"
end

local RUNNING = -1073741824  -- null_flag: running dimension for rules
local RS = node.subtypes("rule")
local RULE_IMAGE, RULE_EMPTY, RULE_USER, RULE_OUTLINE = 2, 3, 4, 9
for k, v in pairs(RS) do
  if v == "image" then RULE_IMAGE = k elseif v == "empty" then RULE_EMPTY = k
  elseif v == "user" then RULE_USER = k elseif v == "outline" then RULE_OUTLINE = k end
end

-- Record a box carrying the picture attribute: the picture's output is the union of its
-- outermost tagged boxes (page coordinates; one baseline, else `multi`). Every node made
-- inside the picture carries the attribute (set while the environment runs), so boxes inside
-- a recorded box are skipped (`st.pic_in`), as is the paragraph indent box the picture's
-- \leavevmode made. Returns true when the caller is entering a recorded box.
local HLIST_INDENT = 3
local function note_pic(st, n, x, baseline, w, h, d)
  if not st.attr_pic or st.pic_in > 0 then return false end
  local id = getattribute(n, st.attr_pic)
  if not id or id < 0 then return false end
  if getid(n) == hlist_id and getsubtype(n) == HLIST_INDENT then return false end
  local r = st.pic_seen[id]
  if not r then
    r = { id = id, x = x, y = baseline, right = x + w, top = baseline - h, bottom = baseline + d, multi = false }
    st.pic_seen[id] = r
    st.pics[#st.pics + 1] = r
  else
    if baseline ~= r.y then r.multi = true end
    if x < r.x then r.x = x end
    if x + w > r.right then r.right = x + w end
    if baseline - h < r.top then r.top = baseline - h end
    if baseline + d > r.bottom then r.bottom = baseline + d end
  end
  r.w = r.right - r.x
  r.h = r.y - r.top
  r.d = r.bottom - r.y
  return true
end

local hlist_out, vlist_out

-- Walk an hlist's content list. Returns final cur_h. `walk` is re-entered for disc replace lists.
-- The current glyph run (binary mode) is mirrored in locals while walking; `sync_out`/`sync_in`
-- move it to/from the state around calls that may emit records or recurse.
hlist_out = function(st, box, left, base_v)
  local last_f, last_e = nil, nil
  local ew, ew_f, ew_ef = nil, nil, nil  -- advance-width cache for the current (font, expansion)
  local bin = st.bin
  local g_order, g_sign, g_set = getfield(box, "glue_order"), getfield(box, "glue_sign"), getfield(box, "glue_set")
  local box_w, box_h, box_d = getwhd(box)
  local cur_h = left
  local cur_glue, cur_g = 0.0, 0
  local run, rn, run_font, run_y, run_ef = st.run, st.rn, st.run_font, st.run_y, st.run_ef

  local function sync_out() st.run, st.rn, st.run_font, st.run_y, st.run_ef = run, rn, run_font, run_y, run_ef end
  local function sync_in() run, rn, run_font, run_y, run_ef = st.run, st.rn, st.run_font, st.run_y, st.run_ef end

  local function glue_advance(wd, st_, sh, sto, sho)
    local rule_wd = wd - cur_g
    if g_sign ~= 0 then
      if g_sign == 1 then
        if sto == g_order then
          cur_glue = cur_glue + st_
          cur_g = tex_round(vet_glue(g_set * cur_glue))
        end
      elseif sho == g_order then
        cur_glue = cur_glue - sh
        cur_g = tex_round(vet_glue(g_set * cur_glue))
      end
    end
    return rule_wd + cur_g
  end

  -- discretionary replace lists are walked in place: the node after the disc is pushed on a
  -- small stack and resumed when the replace list ends
  local dstack, ddepth = nil, 0
  local n = getlist(box)
  while n do
      local id = getid(n)
      local nxt = getnext(n)
      if id == glyph_id then
        local f, c = getfont(n), getchar(n)
        local ef = getexpansion(n) or 0
        local xo, yo = getoffsets(n)
        if f ~= last_f then
          last_e = font_cache[f] or font_entry(f)
          last_f = f
          if not st.fonts[f] then st:usefont(f) end
        end
        local e = last_e
        -- advance width: the engine's integer char width, expanded by ef; the same (font,
        -- char, ef) always yields the same value, so it is computed once per font entry
        if f ~= ew_f or ef ~= ew_ef then
          ew = e.ew[ef]
          if not ew then ew = {}; e.ew[ef] = ew end
          ew_f, ew_ef = f, ef
        end
        local w = ew[c]
        if w == nil then
          w = getwidth(n)
          if ef ~= 0 then w = expand(w, ef) end
          ew[c] = w
        end
        if math_owner then math_owner.glyph(st, n, cur_h + (xo or 0), base_v - (yo or 0), w) end
        if e.virtual then
          sync_out(); st:emit_virtual(f, c, cur_h + (xo or 0), base_v - (yo or 0), ef); sync_in()
        elseif bin then
          -- hot path: append to the current glyph run (flat integer array)
          local y = base_v - yo
          if not run or f ~= run_font or y ~= run_y or ef ~= run_ef then
            sync_out(); st:bin_flush_run()
            run, rn, run_font, run_y, run_ef = {}, 0, f, y, ef
          end
          local gi = e.idx[c]
          if gi == nil then
            local ch = e.chars[c]
            gi = (ch and ch.index) or 0xFFFFFFFF
            e.idx[c] = gi
          end
          run[rn + 1], run[rn + 2], run[rn + 3], run[rn + 4] = c, gi, cur_h + xo, w
          rn = rn + 4
        else
          local gi = glyph_index(f, c)
          st.glyphs = st.glyphs + 1
          local item = { "g", f, c, gi, cur_h + (xo or 0), base_v - (yo or 0), w, ef }
          if st.glyph_attr then item[9] = getattribute(n, st.glyph_attr) end
          st:emit(item)
        end
        cur_h = cur_h + w
      elseif id == glue_id then
        local wd, st_, sh, sto, sho = getglue(n)
        -- inlined glue_advance (hot: every inter-word space)
        local rule_wd = wd - cur_g
        if g_sign ~= 0 then
          if g_sign == 1 then
            if sto == g_order then
              cur_glue = cur_glue + st_
              local g = g_set * cur_glue
              if g > BILLION then g = BILLION elseif g < -BILLION then g = -BILLION end
              if g >= 0 then cur_g = floor(g + 0.5) else cur_g = ceil(g - 0.5) end
            end
          elseif sho == g_order then
            cur_glue = cur_glue - sh
            local g = g_set * cur_glue
            if g > BILLION then g = BILLION elseif g < -BILLION then g = -BILLION end
            if g >= 0 then cur_g = floor(g + 0.5) else cur_g = ceil(g - 0.5) end
          end
        end
        rule_wd = rule_wd + cur_g
        local sub = getsubtype(n)
        if sub >= 100 then
          -- leaders (a_leaders=100, c_leaders=101, x_leaders=102, g_leaders=103)
          local lb = getleader(n)
          if lb and getid(lb) == rule_id then
            local lw, lh, ld = getwhd(lb)
            if lh == RUNNING then lh = box_h end
            if ld == RUNNING then ld = box_d end
            if rule_wd > 0 and (lh + ld) > 0 then sync_out(); st:emit({ "r", cur_h, base_v - lh, rule_wd, lh + ld }); sync_in() end
          elseif lb then
            local leader_wd = getwidth(lb)
            if leader_wd > 0 and rule_wd > 0 then
              rule_wd = rule_wd + 10
              local edge = cur_h + rule_wd
              local lx = 0
              local save_h = cur_h
              if sub == 100 then
                -- aligned leaders: boxes at multiples of leader_wd from the enclosing box's left edge
                cur_h = left + leader_wd * ((cur_h - left) // leader_wd)
                if cur_h < save_h then cur_h = cur_h + leader_wd end
              else
                local lq = rule_wd // leader_wd
                local lr = rule_wd % leader_wd
                if sub == 101 then
                  cur_h = cur_h + lr // 2
                else
                  lx = lr // (lq + 1)
                  cur_h = cur_h + (lr - (lq - 1) * lx) // 2
                end
              end
              local lid = getid(lb)
              local lw, lh, ld = getwhd(lb)
              local lshift = getshift(lb)
              sync_out()
              while cur_h + leader_wd <= edge do
                if lid == hlist_id then
                  hlist_out(st, lb, cur_h, base_v + lshift)
                else
                  vlist_out(st, lb, cur_h, base_v + lshift - lh)
                end
                cur_h = cur_h + leader_wd + lx
              end
              sync_in()
              cur_h = edge - 10
              goto continue
            end
          end
        end
        cur_h = cur_h + rule_wd
      elseif id == kern_id then
        -- LuaTeX: kern_width(q) = width(q) + ex_kern(q); for kern nodes the Lua field
        -- `expansion_factor` *is* ex_kern, an amount in sp precomputed by the packer.
        cur_h = cur_h + getkern(n) + (getexpansion(n) or 0)
      elseif id == disc_id then
        local pre, post, replace = getdisc(n)
        if replace then
          ddepth = ddepth + 1
          if not dstack then dstack = {} end
          dstack[ddepth] = nxt
          nxt = replace
        end
      elseif id == penalty_id or id == boundary_id or id == mark_id then
        -- no output
      elseif id == margin_kern_id then
        cur_h = cur_h + getwidth(n)
      elseif id == hlist_id or id == vlist_id then
        local w, h, d = getwhd(n)
        local sh = getshift(n)
        if math_owner then math_owner.box(st, n, cur_h, base_v + sh, w, h, d, box) end
        local par = st.attr_par and getattribute(n, st.attr_par)
        local line = par and st.attr_line and getattribute(n, st.attr_line)
        sync_out()
        -- a raised/lowered box (TikZ `baseline`): extents relative to the line's baseline
        local pic = note_pic(st, n, cur_h, base_v, w, h - sh, d + sh)
        if pic then st.pic_in = st.pic_in + 1 end
        if par and par >= 0 and id == hlist_id and not st.cur then
          st:begin_line(n, par, line, cur_h, base_v + sh, w, h, d)
          hlist_out(st, n, cur_h, base_v + sh)
          st:end_line()
        elseif id == hlist_id then
          hlist_out(st, n, cur_h, base_v + sh)
        else
          vlist_out(st, n, cur_h, base_v + sh - h)
        end
        if pic then st.pic_in = st.pic_in - 1 end
        sync_in()
        cur_h = cur_h + w
      elseif id == rule_id then
        local w, h, d = getwhd(n)
        local sub = getsubtype(n)
        if h == RUNNING then h = box_h end
        if d == RUNNING then d = box_d end
        if w == RUNNING then w = 0 end
        sync_out()
        if sub == RULE_IMAGE then
          local idx = getfield(n, "index")
          if st.pic_images and st.pic_images[idx] then
            -- a picture taken from the cache: not an image hosts can draw (a region of an
            -- internal PDF); the page is degraded and rendered from the pass PDF. The detail
            -- gives the picture's rectangle (index, x, top, width, height in sp) so a host
            -- can copy it from its rendering of the page when the unit moves live.
            st:emit({ "u", "cached_picture", string.format("%d %d %d %d %d", idx, cur_h, base_v - h, w, h + d) }); st:flag("pic_cache")
          else
            st:emit({ "i", idx, cur_h, base_v - h, w, h + d }); st.images = st.images + 1
          end
        elseif sub == RULE_EMPTY then
          -- \nullfont / empty rule: occupies space, draws nothing
        elseif sub == RULE_USER or sub == RULE_OUTLINE then
          st:emit({ "u", "rule_subtype", sub }); st:flag("rule_subtype", sub)
        elseif w > 0 and (h + d) > 0 then
          st:emit({ "r", cur_h, base_v - h, w, h + d })
        end
        sync_in()
        cur_h = cur_h + w
      elseif id == math_id then
        if math_owner then math_owner.boundary(st, n, box, cur_h, base_v, box_h, box_d) end
        local wd, st_, sh, sto, sho = getglue(n)
        local sub = getsubtype(n)
        sync_out(); st:emit({ "m", sub == 0 and "on" or "off", cur_h }); sync_in()
        if wd == 0 and st_ == 0 and sh == 0 then
          cur_h = cur_h + getfield(n, "surround")
        else
          cur_h = cur_h + glue_advance(wd, st_, sh, sto, sho)
        end
      elseif id == whatsit_id then
        if math_owner and math_owner.marker(st, n, cur_h, base_v) then goto continue end
        local sub = getsubtype(n)
        sync_out()
        if sub == ws_colorstack then
          st:emit({ "c", getfield(n, "stack"), getfield(n, "cmd"), getfield(n, "data") })
        elseif sub == ws_literal then
          st:emit({ "l", getfield(n, "mode"), getfield(n, "data") }); st:flag("literal")
        elseif sub == ws_special then
          local data = getfield(n, "data")
          if drawn_special(data) then st:emit({ "l", -1, data }); st:flag("special") end
        elseif silent_whatsit(sub) then
          -- bookkeeping whatsits (\write, luaotfload, hyperref) produce no output
        elseif sub == ws_save then
          st:emit({ "M", "save", cur_h, base_v, "" })
        elseif sub == ws_setmatrix then
          st:emit({ "M", "set", cur_h, base_v, getfield(n, "data") or "" })
        elseif sub == ws_restore then
          st:emit({ "M", "restore", cur_h, base_v, "" })
        else
          st:emit({ "u", "whatsit", sub }); st:flag("whatsit", sub)
        end
        sync_in()
      elseif id == local_par_id then
        local bl = getfield(n, "box_left_width") or 0
        local br = getfield(n, "box_right_width") or 0
        if bl ~= 0 or br ~= 0 then st:flag("local_boxes") end
      elseif id == dir_id then
        local dir = getfield(n, "dir")
        if dir and dir ~= "TLT" and dir ~= "+TLT" and dir ~= "-TLT" then sync_out(); st:flag("dir", dir); st:emit({ "u", "dir", dir }); sync_in() end
      elseif id == ins_id then
        st.inserts = st.inserts + 1
        if not st.opts.lines_at_top then sync_out(); st:flag("ins"); st:emit({ "u", "ins", cur_h }); sync_in() end
      elseif id == adjust_id then
        sync_out(); st:flag("adjust"); st:emit({ "u", "adjust", cur_h }); sync_in()
      else
        st:flag("node_" .. (node.type(id) or tostring(id)))
      end
      ::continue::
      if nxt == nil and ddepth > 0 then
        nxt = dstack[ddepth]
        ddepth = ddepth - 1
      end
      n = nxt
  end
  sync_out()
  return cur_h
end

vlist_out = function(st, box, left, top)
  local g_order, g_sign, g_set = getfield(box, "glue_order"), getfield(box, "glue_sign"), getfield(box, "glue_set")
  local box_w = getwidth(box)
  local cur_v = top
  local cur_glue, cur_g = 0.0, 0
  local n = getlist(box)
  while n do
    local id = getid(n)
    if id == hlist_id or id == vlist_id then
      local w, h, d = getwhd(n)
      local s = getshift(n)
      cur_v = cur_v + h
      if math_owner then math_owner.box(st, n, left + s, cur_v, w, h, d, box) end
      local par = st.attr_par and getattribute(n, st.attr_par)
      local line = par and st.attr_line and getattribute(n, st.attr_line)
      local pic = note_pic(st, n, left + s, cur_v, w, h, d)
      if pic then st.pic_in = st.pic_in + 1 end
      if id == hlist_id then
        -- Rows: hlists reached from the traversal root through vlists only. Paragraph mode
        -- (fast path) takes every such hlist; page mode takes those tagged with a paragraph
        -- or a unit attribute, except lines of insert material (footnote text).
        local is_row = false
        local unit = nil
        -- an empty \hbox{} (page-filling material of \clearpage) is never a row
        local empty = h == 0 and d == 0 and getlist(n) == nil
        if not st.cur and not empty then
          if st.opts.lines_at_top then
            is_row = true
          else
            unit = st.attr_unit and getattribute(n, st.attr_unit)
            if unit and unit < 0 then unit = nil end
            if (par and par >= 0) or unit then
              is_row = true
              -- migrated material (footnote text) stays a tagged line but belongs to no unit
              if par and par >= 0 and st.is_insert and st.is_insert(par, unit) then unit = nil end
            end
          end
        end
        if is_row then
          st:begin_line(n, par or 0, line or (st.nlines or #st.lines) + 1, left + s, cur_v, w, h, d, unit)
          hlist_out(st, n, left + s, cur_v)
          st:end_line()
        else
          hlist_out(st, n, left + s, cur_v)
        end
      else
        vlist_out(st, n, left + s, cur_v - h)
      end
      if pic then st.pic_in = st.pic_in - 1 end
      cur_v = cur_v + d
    elseif id == rule_id then
      local w, h, d = getwhd(n)
      local sub = getsubtype(n)
      if w == RUNNING then w = box_w end
      if h == RUNNING then h = 0 end
      if d == RUNNING then d = 0 end
      if sub == RULE_IMAGE then
        local idx = getfield(n, "index")
        if st.pic_images and st.pic_images[idx] then
          st:emit({ "u", "cached_picture", string.format("%d %d %d %d %d", idx, left, cur_v, w, h + d) }); st:flag("pic_cache")
        else
          st:emit({ "i", idx, left, cur_v, w, h + d }); st.images = st.images + 1
        end
      elseif sub == RULE_EMPTY then
      elseif sub == RULE_USER or sub == RULE_OUTLINE then
        st:emit({ "u", "rule_subtype", sub }); st:flag("rule_subtype", sub)
      elseif w > 0 and (h + d) > 0 then
        st:emit({ "r", left, cur_v, w, h + d })
      end
      cur_v = cur_v + h + d
    elseif id == glue_id then
      local wd, st_, sh, sto, sho = getglue(n)
      local rule_ht = wd - cur_g
      if g_sign ~= 0 then
        if g_sign == 1 then
          if sto == g_order then cur_glue = cur_glue + st_; cur_g = tex_round(vet_glue(g_set * cur_glue)) end
        elseif sho == g_order then
          cur_glue = cur_glue - sh; cur_g = tex_round(vet_glue(g_set * cur_glue))
        end
      end
      rule_ht = rule_ht + cur_g
      local sub = getsubtype(n)
      if sub >= 100 then
        local lb = getleader(n)
        if lb and getid(lb) == rule_id then
          local lw = getwidth(lb)
          if lw == RUNNING then lw = box_w end
          if lw > 0 and rule_ht > 0 then st:emit({ "r", left, cur_v, lw, rule_ht }) end
        elseif lb then
          local lw, lh, ld = getwhd(lb)
          local leader_ht = lh + ld
          if leader_ht > 0 and rule_ht > 0 then
            rule_ht = rule_ht + 10
            local edge = cur_v + rule_ht
            local lx = 0
            local save_v = cur_v
            if sub == 100 then
              cur_v = top + leader_ht * ((cur_v - top) // leader_ht)
              if cur_v < save_v then cur_v = cur_v + leader_ht end
            else
              local lq = rule_ht // leader_ht
              local lr = rule_ht % leader_ht
              if sub == 101 then cur_v = cur_v + lr // 2
              else lx = lr // (lq + 1); cur_v = cur_v + (lr - (lq - 1) * lx) // 2 end
            end
            local lshift = getshift(lb)
            while cur_v + leader_ht <= edge do
              cur_v = cur_v + lh
              if getid(lb) == hlist_id then hlist_out(st, lb, left + lshift, cur_v)
              else vlist_out(st, lb, left + lshift, cur_v - lh) end
              cur_v = cur_v + ld + lx
            end
            cur_v = edge - 10
            goto vcontinue
          end
        end
      end
      cur_v = cur_v + rule_ht
    elseif id == kern_id then
      cur_v = cur_v + getkern(n)
    elseif id == whatsit_id then
      local sub = getsubtype(n)
      if sub == ws_colorstack then
        st:emit({ "c", getfield(n, "stack"), getfield(n, "cmd"), getfield(n, "data") })
      elseif sub == ws_literal then
        st:emit({ "l", getfield(n, "mode"), getfield(n, "data") }); st:flag("literal")
      elseif sub == ws_special then
        local data = getfield(n, "data")
        if drawn_special(data) then st:emit({ "l", -1, data }); st:flag("special") end
      elseif silent_whatsit(sub) then
      elseif sub == ws_save then
        st:emit({ "M", "save", left, cur_v, "" })
      elseif sub == ws_setmatrix then
        st:emit({ "M", "set", left, cur_v, getfield(n, "data") or "" })
      elseif sub == ws_restore then
        st:emit({ "M", "restore", left, cur_v, "" })
      else
        st:emit({ "u", "whatsit", sub }); st:flag("whatsit", sub)
      end
    elseif id == penalty_id or id == mark_id then
    elseif id == ins_id then
      -- insert material (footnotes) is placed by the page builder; in a paragraph box it is
      -- not drawn here (the host keeps the page's version until the next layout)
      st.inserts = st.inserts + 1
      if not st.opts.lines_at_top then st:flag("ins") end
    else
      st:flag("node_" .. (node.type(id) or tostring(id)))
    end
    ::vcontinue::
    n = getnext(n)
  end
  return cur_v
end

-- Expand a virtual-font character into real-font glyphs/rules (LuaTeX font `commands`).
-- Nested virtual fonts are expanded recursively (depth-limited).
function State:emit_virtual(f, c, x, y, ef, depth)
  depth = depth or 0
  local e = font_entry(f)
  local ch = e.chars[c]
  local cmds = ch and ch.commands
  if not cmds or depth > 4 then
    self:flag("virtual_unexpanded"); self:emit({ "u", "virtual", f }); return
  end
  local fonts = e.fonts or {}
  local cur_font = fonts[1] and fonts[1].id or f
  local px, py = x, y
  local stack = {}
  for _, cmd in ipairs(cmds) do
    local op = cmd[1]
    if op == "font" then
      local fe = fonts[cmd[2]]
      cur_font = fe and fe.id or cur_font
    elseif op == "char" or op == "slot" then
      local cc = op == "char" and cmd[2] or cmd[3]
      if op == "slot" then local fe = fonts[cmd[2]]; cur_font = fe and fe.id or cur_font end
      local fe2 = font_entry(cur_font)
      local w = (fe2.chars[cc] and fe2.chars[cc].width) or 0
      w = floor(w + 0.5)
      if ef ~= 0 then w = expand(w, ef) end
      if fe2.virtual and cur_font ~= f then
        self:emit_virtual(cur_font, cc, px, py, ef, depth + 1)
      else
        self:usefont(cur_font)
        if not self.bin then self.glyphs = self.glyphs + 1 end
        self:emit({ "g", cur_font, cc, glyph_index(cur_font, cc), px, py, w, ef })
      end
      px = px + w
    elseif op == "right" then px = px + floor(cmd[2] + 0.5)
    elseif op == "down" then py = py + floor(cmd[2] + 0.5)
    elseif op == "push" then stack[#stack + 1] = { px, py }
    elseif op == "pop" then local t = stack[#stack]; if t then px, py = t[1], t[2]; stack[#stack] = nil end
    elseif op == "rule" then
      local h, w = floor(cmd[2] + 0.5), floor(cmd[3] + 0.5)
      if w > 0 and h > 0 then self:emit({ "r", px, py - h, w, h }) end
      px = px + w
    elseif op == "special" or op == "pdf" or op == "lua" or op == "image" or op == "node" then
      self:flag("virtual_" .. op); self:emit({ "u", "virtual_" .. op, cur_font })
    end
  end
end

return hlist_out, vlist_out
end
