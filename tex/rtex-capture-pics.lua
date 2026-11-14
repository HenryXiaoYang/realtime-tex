-- rtex-capture-pics.lua: the picture cache hooks of rtex-capture.lua (the mechanics shared with
-- the live server are in rtex-pic.lua).
--
-- Returns a function that takes the capture table C, rtex-pic.lua (P), the JSON module and the
-- debug switch, and defines C.pic_arm, C.pic_begin, C.pic_end and C.pic_write.
return function(C, P, json, DEBUG_ENV)
-- Picture cache ---------------------------------------------------------------------------------
-- A pass records where every picture environment (tikzpicture, circuitikz, pgfpicture) landed
-- on its page. The next pass gets a manifest ($RTEX_CAPTURE_DIR/pic-manifest.json, written by
-- the session: "file:line" -> {pdf, page, bbox, w, h, d, state, end_line}) for pictures whose
-- source and surroundings are unchanged, and replaces each of them with that region of the
-- earlier pass's PDF: an image of exactly the picture's size, lowered by its depth. Everything
-- else on the page is typeset as usual, so placements are identical and the pass skips the
-- picture's TikZ work.
--
-- A drawn picture is tagged through attribute inheritance: the attribute is set while the
-- environment runs, so every node it makes (its box, pgfplots' extra boxes) carries it, and
-- nothing made before it (an \item label the paragraph's \everypar places) does; rtex-dl.lua
-- records the union of the outermost tagged boxes.
C.attr_pic = luatexbase.new_attribute("rtex_pic")
C.pic_id = 0
C.pic_keys = {}       -- id -> { key, env, state }
C.pics = {}           -- key -> { page, x, y, w, h, d, page_height, state } (drawn this pass)
C.pic_bad = {}        -- keys whose output spans lines or pages
C.pic_mismatch = {}   -- keys whose skipped body did not end on the predicted line
C.cache_images = {}   -- image resource index -> true (cached pictures)
C.manifest = nil
C.manifest_used = {}
C.src_lines = {}      -- file name -> its lines (for the key check)
local function pic_key()
  local f = status.filename or ""
  f = f:gsub("^%./", "")
  return f .. ":" .. tex.inputlineno
end
local function load_manifest()
  if C.manifest ~= nil then return C.manifest end
  C.manifest = false
  local dir = os.getenv("RTEX_CAPTURE_DIR")
  if not dir then return false end
  local f = io.open(dir .. "/pic-manifest.json", "r")
  if not f then return false end
  local text = f:read("*a")
  f:close()
  local ok, m = pcall(json.decode, text)
  if ok and type(m) == "table" then C.manifest = m end
  return C.manifest
end
-- The source line the begin macro executes on must start with \begin{env}: that is the line
-- the session keyed the picture by. A picture inside a macro argument executes on the
-- argument's last line, whose text says so (and it is drawn).
local function line_begins_env(env)
  local f = status.filename
  if not f then return false end
  local lines = C.src_lines[f]
  if lines == nil then
    lines = false
    local fh = io.open(f, "r")
    if fh then
      lines = {}
      for l in fh:lines() do lines[#lines + 1] = l end
      fh:close()
    end
    C.src_lines[f] = lines
  end
  if not lines then return false end
  local l = lines[tex.inputlineno]
  if not l then return false end
  l = l:gsub("^%s+", "")
  local marker = "\\begin{" .. env .. "}"
  return l:sub(1, #marker) == marker
end
-- env/<env>/begin hook and the wrapped begin macro (rtex-pic.tex). A manifest entry is used
-- when its key, environment and state match and the executing line starts with the \begin;
-- a drawn picture gets an id every node it makes inherits.
function C.pic_arm(env) P.arm(C.picctl, env) end
function C.pic_begin(env)
  C.pic_id = C.pic_id + 1
  local id = C.pic_id
  local key = pic_key()
  local lookup = function(env, state)
    C.pic_keys[id] = { key = key, env = env, state = state }
    local m = load_manifest()
    local e = m and m[key]
    if DEBUG_ENV then texio.write_nl("RTEXPIC begin " .. key .. " state " .. state .. " manifest " .. tostring(e and e.state)) end
    if e and not C.manifest_used[key] and e.env == env and (e.state or "") == state and line_begins_env(env) then
      C.manifest_used[key] = true
      C.pic_hit = { key = key, end_line = e.end_line }
      return e
    end
    return nil
  end
  local on_draw = function()
    -- every node the environment makes carries the picture's id (restored with the group)
    tex.setattribute(C.attr_pic, id)
  end
  if not P.begin(C.picctl, env, lookup, on_draw) then
    C.pic_hit = nil
  end
end
-- After the body of a cached picture was skipped (rtex-capture.sty): it must have ended on
-- the line the session's scan found its \end on; otherwise the skipped text was not the
-- picture the cache holds, and the session forgets the entry (the next pass draws it).
function C.pic_end()
  local h = C.pic_hit
  C.pic_hit = nil
  if h and h.end_line and tex.inputlineno ~= h.end_line then
    C.pic_mismatch[#C.pic_mismatch + 1] = h.key
    texio.write_nl("rtex-capture: cached picture " .. h.key .. " ended on line " .. tex.inputlineno ..
                   ", expected " .. tostring(h.end_line) .. " (forgotten)")
  end
end
-- Writes the cached picture's image node (inside \hbox{...} of \rtex@picreplace).
-- not listed in images_info: hosts never see a cached picture as an image
function C.pic_write() P.write(C.picctl, C.cache_images) end
end
