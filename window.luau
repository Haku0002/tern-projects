--!strict
-- tern-projects window half: commands, the per-tab Projects sidebar (toggle, collapse/expand sizing) and
-- guarded routes that open chats beside it: in the main area, in a split, or in an explicit new tab.
-- iOS has no tern.process; skip setup on that platform.
if tern.runtime.os == "ios" then
	return
end

local fmt = require("./lib/fmt")
local store = require("./lib/store")
local unpct = fmt.unpct

local KIND = "tern-projects.panel"
-- Where a chat opens relative to the sidebar; any other `place` in a URL is rejected.
local PLACES: { [string]: boolean } = { main = true, right = true, down = true, tab = true }

-- `sessions`: every session id an OMP chat is stored under, checked against running omp processes.
type OpenRequest = { cwd: string, cmd: string, place: string, uid: string?, title: string?, sessions: { string }? }

-- Per-pane state for toggle tracking.
local last_mode: { [number]: string } = {}
local prev_focus: { [number]: number } = {}
-- Sidebars currently narrowed to the icon rail (tiled, but collapsed).
local rail_panes: { [number]: boolean } = {}
-- Last serialized `live-panes` payload sent to each sidebar pane, to skip redundant events.
local last_live_sig: { [number]: string } = {}
-- Per tab: the chat pane focused last, which Open in Main replaces and the splits divide.
local main_of: { [number]: number } = {}
-- Tabs created in this window whose first pane hasn't arrived yet; that pane brings the automatic sidebar.
local new_tabs: { [number]: boolean } = {}
-- Hidden (parked) sidebars -> the tab they were hidden from, which Projects deals them back into.
local parked_from: { [number]: number } = {}
-- Sidebars seen parked (hidden, or parked through Tern); one that comes back tiled is docked at the left.
local was_parked: { [number]: boolean } = {}
-- Sidebars whose last blind nudge (width unknown) grew them; the next one shrinks them back.
local nudged: { [number]: boolean } = {}
-- Tabs holding a tiled sidebar; the status line offers Projects in the others.
local sidebar_tabs: { [number]: boolean } = {}
-- Non-sidebar panes currently parked in the bottom deck, for the status bar segment.
local parked_count = 0
-- Whether this window already explained how to bring a hidden sidebar back.
local hint_shown = false
local sync_panels: (WindowCx, number?) -> ()

-- Query parameter `name` of `q`, percent-decoded; nil when absent.
local function param(q: string, name: string): string?
	local raw = string.match("&" .. q, "&" .. name .. "=([^&]*)")
	return if raw ~= nil then unpct(raw) else nil
end

-- Sends an event to a panel block, ignoring panes whose block hasn't finished starting or just exited.
local function send_panel(cx: WindowCx, pane: number, ev: { [string]: any }): boolean
	local ok = pcall(function()
		cx.session:event(pane, ev)
	end)
	return ok
end

-- Whether `p` is a chat (agent or terminal) tiled in `tab`, neither parked nor floating.
local function tiled_chat(p: PaneInfo?, tab: number): boolean
	return p ~= nil and (p.kind == "agent" or p.kind == "terminal") and p.tab == tab and not p.parked and not p.floating
end

-- The tiled panes of `tab`, left/top first.
local function leaves(cx: WindowCx, tab: number): { number }
	local out: { number } = {}
	local function walk(node: TabTree?)
		if node == nil then
			return
		end
		if node.pane ~= nil then
			table.insert(out, node.pane)
			return
		end
		walk(node[1])
		walk(node[2])
	end
	local layout = cx.session:layout(tab)
	if layout ~= nil then
		walk(layout.root)
	end
	return out
end

-- The main-area pane of `tab`: its chat focused last, else its first tiled chat; nil when it has none.
local function main_pane(cx: WindowCx, tab: number): number?
	local last = main_of[tab]
	if last ~= nil and tiled_chat(cx.session:pane(last), tab) then
		return last
	end
	main_of[tab] = nil
	for _, pane in leaves(cx, tab) do
		if tiled_chat(cx.session:pane(pane), tab) then
			return pane
		end
	end
	return nil
end

-- The share of `tab`'s width that tiled `pane` takes: the product of the ratios on its path through the
-- right-splits above it (down-splits don't change width). Nil when the pane isn't tiled there.
local function width_share(cx: WindowCx, tab: number, pane: number): number?
	local function walk(node: TabTree?): number?
		if node == nil then
			return nil
		end
		if node.pane ~= nil then
			return if node.pane == pane then 1 else nil
		end
		for side = 1, 2 do
			local share = walk(node[side])
			if share ~= nil then
				if node.split ~= "right" then
					return share
				end
				local ratio = node.ratio or 0.5
				return share * (if side == 1 then ratio else 1 - ratio)
			end
		end
		return nil
	end
	local layout = cx.session:layout(tab)
	return if layout ~= nil then walk(layout.root) else nil
end

-- Sizes sidebar `pane` to `cols` within the handler that just docked it, before Tern draws a frame, so it
-- appears at that width instead of the half Tern gives a new split. Needs the tab width a sidebar measured
-- (`sidebar.tab_cols`); returns false without one, and the panel's own fit sizes it after it lands.
local function snap_width(cx: WindowCx, tab: number, pane: number, cols: number): boolean
	local total = store.tab_cols()
	local share = width_share(cx, tab, pane)
	if total == nil or share == nil then
		return false
	end
	local delta = math.floor(cols - share * total + 0.5)
	return delta == 0 or cx.layout:resize(pane, "right", delta)
end

-- Open chats ---------------------------------------------------------------------------------
-- `chats.open` maps chat uid -> {pane, cwd}, so a chat that already runs is shown instead of started twice,
-- across plugin reloads and window restarts.

local function open_chats(): { [string]: any }
	local map = store.get(store.OPEN_KEY)
	return if type(map) == "table" then map else {}
end

-- Directory equality; Windows paths compare case- and separator-insensitively, verbatim prefix ignored.
local function same_dir(a: string, b: string): boolean
	local function norm(p: string): string
		if tern.runtime.os == "windows" then
			p = string.lower((string.gsub(p, "\\", "/")))
			p = string.gsub(p, "^//%?/unc/", "//")
			p = string.gsub(p, "^//%?/", "")
		end
		return (string.gsub(p, "(.)/+$", "%1"))
	end
	return norm(a) == norm(b)
end

-- Whether `info` still runs the chat recorded as `entry` (started in `cwd`, when given).
local function live_chat(info: PaneInfo, entry: any, cwd: string?): boolean
	if (info.kind ~= "agent" and info.kind ~= "terminal") or info.exited ~= nil then
		return false
	end
	if type(entry) ~= "table" or type(entry.cwd) ~= "string" then
		return false
	end
	if cwd ~= nil and not same_dir(entry.cwd, cwd) then
		return false
	end
	local now = info.cwd
	return now == nil or now == "" or same_dir(now, entry.cwd)
end

-- The pane already running chat `uid` in `cwd`, or nil; forgets a record whose pane is gone or changed.
local function open_pane(cx: WindowCx, uid: string, cwd: string): PaneInfo?
	local map = open_chats()
	local entry = map[uid]
	if entry == nil then
		return nil
	end
	local info = if type(entry) == "table" and type(entry.pane) == "number" then cx.session:pane(entry.pane) else nil
	if info ~= nil and live_chat(info, entry, cwd) then
		return info
	end
	map[uid] = nil
	store.set(store.OPEN_KEY, map)
	return nil
end

local function remember(uid: string, pane: number, cwd: string, title: string?)
	local map = open_chats()
	map[uid] = { pane = pane, cwd = cwd, title = if title ~= nil and title ~= "" then title else nil }
	store.set(store.OPEN_KEY, map)
end

local function forget(pane: number)
	local map = open_chats()
	local changed = false
	for uid, entry in map do
		if type(entry) ~= "table" or entry.pane == pane then
			map[uid] = nil
			changed = true
		end
	end
	if changed then
		store.set(store.OPEN_KEY, map)
	end
end

-- Drops records whose pane exited, changed directory, or (when only one local session exists) no longer exists.
local function reconcile(cx: WindowCx)
	local map = open_chats()
	local single = false
	local ok_s, sess = pcall(function() return cx.sessions:list() end)
	local ok_h, hosts = pcall(function() return cx.hosts:list() end)
	if ok_s and ok_h and #sess <= 1 and #hosts <= 1 then
		single = true
	end
	local changed = false
	for uid, entry in map do
		local pane = if type(entry) == "table" and type(entry.pane) == "number" then entry.pane else nil
		local info = if pane ~= nil then cx.session:pane(pane) else nil
		if pane == nil or (info == nil and single) or (info ~= nil and not live_chat(info, entry, nil)) then
			map[uid] = nil
			changed = true
		end
	end
	if changed then
		store.set(store.OPEN_KEY, map)
	end
end

-- Live panes & parked deck -------------------------------------------------------------------

local function agent_map(cx: WindowCx): { [number]: AgentBlock }
	local out: { [number]: AgentBlock } = {}
	local ok, list = pcall(function() return cx.agents:list() end)
	if ok and type(list) == "table" then
		for _, a in list do
			if type(a.pane) == "number" then
				out[a.pane] = a
			end
		end
	end
	return out
end

local function pane_state(p: PaneInfo, ag: AgentBlock?): string
	if ag ~= nil and type(ag.state) == "string" then
		if ag.state == "working" then return "working" end
		if ag.state == "waiting_input" then return "waiting" end
		if ag.state == "exited" then return "exited" end
		return "idle"
	end
	if p.exited ~= nil then return "exited" end
	if p.busy == true then return "working" end
	return "idle"
end

local function is_fork_command(cmd: string): boolean
	return string.find(cmd, "%-%-fork") ~= nil or string.match(cmd, "codex%s+fork%s") ~= nil
end

local function extract_session_id(cmd: string): string?
	if is_fork_command(cmd) then
		return nil
	end
	local jl = string.match(cmd, "([%w_%-]+)%.jsonl")
	if jl ~= nil then
		return string.match(jl, "Z_([%w_%-]+)$") or jl
	end
	return string.match(cmd, "%-%-resume%s+['\"]?([%w_%-]+)['\"]?")
		or string.match(cmd, "%sresume%s+['\"]?([%w_%-]+)['\"]?")
end

local function infer_agent(cmd: string?, prog: string?): string?
	local s = string.lower((cmd or "") .. " " .. (prog or ""))
	if string.find(s, "claude", 1, true) then return "claude" end
	if string.find(s, "codex", 1, true) then return "codex" end
	if string.find(s, "opencode", 1, true) then return "opencode" end
	if string.find(s, "omp", 1, true) then return "omp" end
	if string.match(s, "%f[%w]pi%f[%W]") then return "pi" end
	return nil
end

-- Whether `info` is an already-exited pane or a brand-new untouched shell prompt that never ran a command.
-- Replacing it in Main closes it instead of dumping a dead or blank shell tile into the parked deck. Live
-- agents, tracked chats and shells that ran anything are never disposable.
local function disposable_pane(info: PaneInfo, agents: { [number]: AgentBlock }?): boolean
	if info.block == KIND then return false end
	if info.exited ~= nil then return true end
	if info.kind ~= "terminal" then return false end
	if agents ~= nil and agents[info.pane] ~= nil then return false end
	for _, entry in open_chats() do
		if type(entry) == "table" and entry.pane == info.pane then
			return false
		end
	end
	return info.at_prompt == true and info.running == nil and info.last == nil and not info.busy
end

-- Placement ----------------------------------------------------------------------------------

-- Shows `info` at `place` without restarting its program: a visible pane is focused; a parked one is dealt
-- back in, swapping places with the main pane for `main` (or replacing a disposable blank/exited shell).
local function show_open(cx: WindowCx, info: PaneInfo, sidebar: number, main: number?, place: string): boolean
	local pane = info.pane
	if not info.parked then
		cx.layout:focus(pane)
		return true
	end
	if place == "tab" then
		return cx.layout:move_to_new_tab(pane, nil, { focus = true }) ~= nil
	elseif place == "main" and main ~= nil then
		local minfo = cx.session:pane(main)
		if minfo ~= nil and disposable_pane(minfo, agent_map(cx)) then
			if not cx.layout:move(pane, main, "right", { focus = true }) then
				return false
			end
			cx.layout:close(main)
			return true
		end
		return cx.layout:move(pane, main, "swap", { focus = true })
	elseif place == "down" and main ~= nil then
		return cx.layout:move(pane, main, "down", { focus = true })
	end
	-- Right of the main pane, or of the sidebar when the tab has no chat (never below the sidebar).
	return cx.layout:move(pane, main or sidebar, "right", { focus = true })
end

-- Starts the chat at `req.place`; returns its pane.
local function launch(cx: WindowCx, req: OpenRequest, sidebar: number, main: number?): number?
	local spec: Launch = { cwd = req.cwd, command = req.cmd }
	if req.place == "tab" then
		return cx.layout:new_tab(spec)
	end
	if main == nil then
		-- The tab's first chat opens right of the sidebar, whatever the placement.
		return cx.layout:split(sidebar, "right", spec)
	end
	if req.place ~= "main" then
		return cx.layout:split(main, if req.place == "down" then "down" else "right", spec)
	end
	-- Main replaces the visible chat in place. A blank untouched shell or exited pane closes cleanly; any
	-- live chat or used shell is parked so its program keeps running in the deck.
	local minfo = cx.session:pane(main)
	local drop_main = minfo ~= nil and disposable_pane(minfo, agent_map(cx))
	local pane = cx.layout:split(main, "right", spec, { focus = false })
	if pane == nil then
		return nil
	end
	if drop_main then
		cx.layout:close(main)
	else
		local parked, err = cx.layout:park(main)
		if not parked then
			tern.log.warn("tern-projects: couldn't park the previous chat", err)
		end
	end
	cx.layout:focus(pane)
	return pane
end

-- Places `sidebar`'s tab after showing or launching `pane` at `place`.
local function finish_open(cx: WindowCx, sidebar: PaneInfo, main_before: number?, pane: number, place: string)
	local tab = sidebar.tab :: number
	if tiled_chat(cx.session:pane(pane), tab) then
		main_of[tab] = pane
	end
	if main_before == nil and place ~= "tab" then
		-- The sidebar filled the tab until this first chat split it; bring it straight back to the remembered
		-- width (the panel refits it if the tab's width isn't known yet).
		local target = if rail_panes[sidebar.pane] then store.COLLAPSED else store.width()
		snap_width(cx, tab, sidebar.pane, target)
		cx.session:event(sidebar.pane, { ev = "action", id = "main", act = "sidebar-fit" })
	end
	sync_panels(cx)
end

-- Opens a chat for the panel `sidebar` at `req.place`. A chat that already runs is shown, never started
-- again; otherwise it launches beside the tab's main pane (or the sidebar while the tab has no chat).
local function open_session_in_layout(cx: WindowCx, sidebar: PaneInfo, req: OpenRequest)
	local tab = sidebar.tab :: number
	local main = main_pane(cx, tab)
	local uid = req.uid
	local existing = if uid ~= nil then open_pane(cx, uid, req.cwd) else nil
	local ok, result = pcall(function(): number?
		if existing ~= nil then
			return if show_open(cx, existing, sidebar.pane, main, req.place) then existing.pane else nil
		end
		return launch(cx, req, sidebar.pane, main)
	end)
	if not ok or result == nil then
		local title = if existing ~= nil then "Could not show the open chat" else "Could not open chat"
		cx:toast("error", title, if ok then "Tern did not place the pane." else tostring(result))
		return
	end
	local pane = result :: number
	if uid ~= nil and existing == nil then
		remember(uid, pane, req.cwd, req.title)
	end
	finish_open(cx, sidebar, main, pane, req.place)
end

-- The running omp process whose command line names one of `sessions` (`omp --resume <file>`; omp's session
-- file names carry the session id), or nil.
local function omp_holder(list: ProcessList?, sessions: { string }): ProcessInfo?
	if list == nil then
		return nil
	end
	for _, proc in list.processes do
		local is_fork = false
		for _, arg in proc.command do
			if arg == "--fork" or string.sub(arg, 1, 7) == "--fork=" then
				is_fork = true
				break
			end
		end
		if not is_fork then
			for _, arg in proc.command do
				for _, id in sessions do
					if string.find(arg, id, 1, true) then
						return proc
					end
				end
			end
		end
	end
	return nil
end

-- Opens a chat like `open_session_in_layout`, except an OMP chat that another omp process already holds:
-- a second omp on a held session doesn't share it, it copies the conversation to a new session file and
-- carries on there, so the two diverge and the chat shows up twice. That chat is left where it runs.
local function open_chat(cx: WindowCx, sidebar: PaneInfo, req: OpenRequest)
	local sessions = req.sessions
	if sessions == nil or (req.uid ~= nil and open_pane(cx, req.uid, req.cwd) ~= nil) then
		open_session_in_layout(cx, sidebar, req)
		return
	end
	local sidebar_pane = sidebar.pane
	cx.procs:list({ name = "omp", limit = 1000 }):next(function(list, err, later)
		if later == nil then
			return
		end
		if err ~= nil then
			tern.log.warn("tern-projects: couldn't list omp processes; opening the chat unchecked", err)
		end
		local holder = omp_holder(list, sessions)
		if holder ~= nil then
			later:toast(
				"info",
				"This chat is already running",
				"omp (PID " .. tostring(holder.pid) .. ") has it open. Switch to it there: a second omp would copy the chat to a new one."
			)
			return
		end
		local side = later.session:pane(sidebar_pane)
		if side ~= nil and side.tab ~= nil then
			open_session_in_layout(later, side, req)
		end
	end)
end

-- Sidebar ------------------------------------------------------------------------------------

-- Brings sidebar `pane` (hidden, a tile, or dealt back by Tern beside some other pane) to the left edge of
-- `tab` at the remembered width, in one step so no frame shows it elsewhere or half the tab wide. The panel
-- still gets `sidebar-fit` to correct a width that couldn't be snapped (no tab width measured yet).
local function dock_sidebar(cx: WindowCx, pane: number, tab: number): boolean
	parked_from[pane] = nil
	was_parked[pane] = nil
	rail_panes[pane] = nil
	local info = cx.session:pane(pane)
	if info == nil then
		return false
	end
	local edge: number? = nil
	for _, leaf in leaves(cx, tab) do
		if leaf ~= pane then
			edge = leaf
			break
		end
	end
	local placed = true
	if info.floating then
		placed = edge ~= nil and cx.layout:dock(pane, edge, "left", { focus = false })
	elseif info.parked or leaves(cx, tab)[1] ~= pane then
		placed = edge ~= nil and cx.layout:move(pane, edge, "left", { focus = false })
	end
	if not placed then
		return false
	end
	snap_width(cx, tab, pane, store.width())
	store.set(store.COLLAPSED_KEY, false)
	cx.session:event(pane, { ev = "action", id = "main", act = "sidebar-state", value = "expanded", share = width_share(cx, tab, pane) })
	cx.session:event(pane, { ev = "action", id = "main", act = "sidebar-fit" })
	return true
end

-- Tracks tiled sidebars, docks sidebars Tern dealt back from the deck, and pushes live open/parked pane state
-- plus each tab's active chat directory to this window's sidebars whenever anything changed.
sync_panels = function(cx: WindowCx, force_pane: number?)
	local all_panes = cx.session:panes()
	local tabs: { [number]: boolean } = {}
	local panels: { PaneInfo } = {}
	local pane_by_id: { [number]: PaneInfo } = {}
	for _, p in all_panes do
		pane_by_id[p.pane] = p
		if p.block == KIND then
			if p.parked then
				was_parked[p.pane] = true
			elseif was_parked[p.pane] and p.tab ~= nil and not p.floating then
				dock_sidebar(cx, p.pane, p.tab)
			end
			table.insert(panels, p)
			if p.tab ~= nil and not p.parked and not p.floating then
				tabs[p.tab] = true
			end
		end
	end

	local agents = agent_map(cx)
	local map = open_chats()
	local uid_by_pane: { [number]: string } = {}
	local saved_title: { [number]: string } = {}
	local open_out: { [string]: any } = {}
	local sess_out: { [string]: any } = {}

	for uid, entry in map do
		if type(entry) == "table" and type(entry.pane) == "number" then
			local p = pane_by_id[entry.pane]
			if p ~= nil and live_chat(p, entry, nil) then
				uid_by_pane[p.pane] = uid
				if type(entry.title) == "string" and entry.title ~= "" then
					saved_title[p.pane] = entry.title
				end
				open_out[uid] = {
					pane = p.pane,
					parked = p.parked == true,
					focused = p.focused == true,
					state = pane_state(p, agents[p.pane]),
				}
			end
		end
	end

	for _, p in all_panes do
		if p.block ~= KIND and (p.kind == "agent" or p.kind == "terminal") and p.exited == nil then
			local ag = agents[p.pane]
			local cmd = if ag ~= nil and type(ag.command) == "string" then ag.command elseif p.running ~= nil then p.running.line else nil
			local sid = if cmd ~= nil then extract_session_id(cmd) else nil
			if sid ~= nil then
				sess_out[sid] = {
					pane = p.pane,
					parked = p.parked == true,
					focused = p.focused == true,
					state = pane_state(p, ag),
				}
			end
		end
	end

	local parked_out: { any } = {}
	for _, p in all_panes do
		if p.parked and p.block ~= KIND then
			local ag = agents[p.pane]
			local uid = uid_by_pane[p.pane]
			local cmd = if ag ~= nil and type(ag.command) == "string" then ag.command elseif p.running ~= nil then p.running.line else nil
			local agent = if uid ~= nil then string.match(uid, "^([^:]+):") else infer_agent(cmd, p.program)
			local sid = if cmd ~= nil then extract_session_id(cmd) else nil
			local cwd = if type(p.cwd) == "string" then p.cwd else ""
			local title = saved_title[p.pane]
			if title == nil or title == "" then
				title = if p.title ~= "" then p.title elseif cwd ~= "" then fmt.basename(cwd) else (p.program or "Pane")
			end
			table.insert(parked_out, {
				pane = p.pane,
				uid = uid,
				sid = sid,
				agent = agent,
				kind = p.kind,
				title = title,
				cwd = cwd,
				state = pane_state(p, ag),
			})
		end
	end

	local chrome_dirty = #parked_out ~= parked_count
	parked_count = #parked_out
	if not chrome_dirty then
		for tab in sidebar_tabs do
			if not tabs[tab] then chrome_dirty = true; break end
		end
	end
	if not chrome_dirty then
		for tab in tabs do
			if not sidebar_tabs[tab] then chrome_dirty = true; break end
		end
	end
	sidebar_tabs = tabs
	if chrome_dirty then
		tern.chrome.refresh()
	end

	local parked_arr = tern.json.array(parked_out)
	for _, panel in panels do
		local tab_cwd = ""
		if panel.tab ~= nil then
			local m = main_pane(cx, panel.tab)
			local mi = if m ~= nil then pane_by_id[m] else nil
			if mi ~= nil and type(mi.cwd) == "string" and mi.cwd ~= "" then
				tab_cwd = mi.cwd
			end
		end
		local sig = tern.json.encode({ o = open_out, s = sess_out, p = parked_arr, c = tab_cwd })
		if force_pane == panel.pane or last_live_sig[panel.pane] ~= sig then
			if send_panel(cx, panel.pane, {
				ev = "action",
				id = "main",
				act = "live-panes",
				open = open_out,
				sessions = sess_out,
				parked = parked_arr,
				cwd = tab_cwd,
			}) then
				last_live_sig[panel.pane] = sig
			end
		end
	end
end

-- Collapses the sidebar `pane` by Settings: "hide" parks it (keeping at most one parked Projects sidebar in
-- the deck so hidden sidebars never pile up), "tile" floats it as a card over the main chat's top-left corner.
-- Either way the tab's chats take its width. A tab never empties: a sidebar alone in its tab stays.
local function collapse_sidebar(cx: WindowCx, pane: number, style: string)
	local info = cx.session:pane(pane)
	local tab = if info ~= nil then info.tab else nil
	local main = if tab ~= nil then main_pane(cx, tab) else nil
	if tab == nil or #leaves(cx, tab) < 2 or (style == "tile" and main == nil) then
		cx:toast("error", "Could not collapse Projects", "The sidebar needs a chat beside it.")
		return
	end
	rail_panes[pane] = nil
	if style == "tile" then
		if not cx.layout:float(pane, main :: number, "tl", { focus = false }) then
			cx:toast("error", "Could not collapse Projects", "Tern did not float the sidebar.")
			return
		end
		cx.session:event(pane, { ev = "action", id = "main", act = "sidebar-state", value = "collapsed", tile = true })
	else
		local existing_parked: number? = nil
		for _, p in cx.session:panes() do
			if p.block == KIND and p.parked and p.pane ~= pane then
				existing_parked = p.pane
				break
			end
		end
		if existing_parked ~= nil then
			-- One hidden sidebar in the deck is enough; close the duplicate instead of stacking folder cards.
			parked_from[existing_parked] = tab
			cx.layout:close(pane)
		else
			local parked, err = cx.layout:park(pane)
			if not parked then
				cx:toast("error", "Could not hide Projects", tostring(err or "Tern did not park the sidebar."))
				return
			end
			parked_from[pane] = tab
			was_parked[pane] = true
			cx.session:event(pane, { ev = "action", id = "main", act = "sidebar-state", value = "collapsed", tile = false })
		end
	end
	store.set(store.COLLAPSED_KEY, true)
	if main ~= nil then
		cx.layout:focus(main)
	end
	sync_panels(cx)
	if style == "hide" and not hint_shown and cx.settings:get("status_bar") ~= true then
		hint_shown = true
		local keys = if tern.runtime.os == "macos" then "Cmd+Alt+S" else "Ctrl+Alt+S"
		cx:toast("info", "Projects hidden", "Press " .. keys .. " to show it again, or turn on the status bar for a Projects button.")
	end
end

-- Sizes the sidebar `pane`, now `cols` wide, for `state`: collapse it (hide, tile, or narrow to the icon rail,
-- by Settings), expand it to the remembered width (docking a tile back first), or quietly fit a new or
-- returning sidebar to its width: the rail's when `rail`, else the remembered one.
local function size_sidebar(cx: WindowCx, pane: number, state: string, cols: number?, rail: boolean)
	local style = store.collapse_style()
	local info = cx.session:pane(pane)
	if info ~= nil and info.floating then
		if state == "expand" and info.tab ~= nil then
			dock_sidebar(cx, pane, info.tab)
			sync_panels(cx, pane)
		elseif state == "fit" then
			cx.session:event(pane, { ev = "action", id = "main", act = "sidebar-state", value = "collapsed", tile = true })
		end
		return
	end
	if state == "collapse" and style ~= "rail" then
		collapse_sidebar(cx, pane, style)
		return
	end
	local narrow = state == "collapse" or (state == "fit" and rail)
	local target = if narrow then store.COLLAPSED else store.width()
	local moved
	if cols == nil then
		local tab = if info ~= nil then info.tab else nil
		if tab ~= nil and snap_width(cx, tab, pane, target) then
			moved = true
		elseif narrow then
			moved = cx.layout:resize(pane, "right", -1000)
		else
			nudged[pane] = not nudged[pane]
			moved = cx.layout:resize(pane, "right", if nudged[pane] then 4 else -4)
		end
	else
		moved = cols == target or cx.layout:resize(pane, "right", target - cols)
	end
	local has_neighbor = info ~= nil and info.tab ~= nil and #leaves(cx, info.tab) >= 2
	if not moved and state == "collapse" and not has_neighbor then
		cx:toast("error", "Could not collapse Projects", "The sidebar needs a pane beside it.")
		return
	end
	rail_panes[pane] = if narrow then true else nil
	if state ~= "fit" then
		store.set(store.COLLAPSED_KEY, narrow)
	end
	cx.session:event(pane, {
		ev = "action",
		id = "main",
		act = "sidebar-state",
		value = if narrow then "collapsed" else "expanded",
		share = if info ~= nil and info.tab ~= nil then width_share(cx, info.tab, pane) else nil,
	})
	if state == "collapse" and narrow and info ~= nil and info.tab ~= nil then
		local main = main_pane(cx, info.tab)
		if main ~= nil then
			cx.layout:focus(main)
		end
	end
end

-- Toggle or open the projects panel in `mode` for the current tab; a tab holds at most one. Pressing the
-- shortcut while the expanded sidebar is focused in `mode` collapses it by Settings instead of destroying it.
local function toggle(cx: WindowCx, mode: string)
	local f = cx.session:focused()
	local fi = if f ~= nil then cx.session:pane(f) else nil
	local tab = if fi ~= nil then fi.tab else nil
	if f == nil or fi == nil or tab == nil then
		return
	end

	local panel: number? = nil
	local parked: number? = nil
	for _, p in cx.session:panes() do
		if p.block == KIND then
			if p.tab == tab then
				panel = p.pane
				break
			elseif p.parked and (parked == nil or parked_from[p.pane] == tab) then
				parked = p.pane
			end
		end
	end

	local edge = leaves(cx, tab)[1] or f
	local pinfo = if panel ~= nil then cx.session:pane(panel) else nil
	local was_collapsed = false
	if panel ~= nil and pinfo ~= nil and pinfo.floating then
		was_collapsed = true
		dock_sidebar(cx, panel, tab)
	elseif panel == nil and parked ~= nil and dock_sidebar(cx, parked, tab) then
		was_collapsed = true
		panel = parked
	elseif panel ~= nil and rail_panes[panel] then
		was_collapsed = true
		size_sidebar(cx, panel, "expand", nil, false)
	end

	if panel ~= nil then
		if not was_collapsed and f == panel and last_mode[panel] == mode then
			if #leaves(cx, tab) >= 2 then
				size_sidebar(cx, panel, "collapse", nil, false)
				local pf = prev_focus[panel]
				if pf ~= nil and pf ~= panel and cx.session:pane(pf) ~= nil then
					cx.layout:focus(pf)
				end
			end
			return
		end
		local new_cwd = if f ~= panel then fi.cwd or "" else ""
		cx.session:event(panel, {
			ev = "action",
			id = "main",
			act = "open=" .. mode .. ":" .. new_cwd,
			value = mode .. ":" .. new_cwd,
		})
		cx.layout:focus(panel)
		last_mode[panel] = mode
		if f ~= panel then
			prev_focus[panel] = f
		end
		sync_panels(cx, panel)
		return
	end

	local new_pane = cx.layout:split(edge, "left", { block = KIND, args = { mode, fi.cwd or "" } })
	if new_pane == nil then
		cx:toast("error", "Projects block unavailable", "tern plugin list")
		return
	end
	snap_width(cx, tab, new_pane, store.width())
	store.set(store.COLLAPSED_KEY, false)
	last_mode[new_pane] = mode
	prev_focus[new_pane] = f
	sync_panels(cx, new_pane)
end

-- Opens a Projects sidebar at the left edge of the new `tab`, without taking focus, unless Settings turned
-- that off, the sidebar was last hidden/tiled, or the tab already holds one.
local function auto_sidebar(cx: WindowCx, tab: number)
	local rail = store.collapse_style() == "rail" and store.get(store.COLLAPSED_KEY) == true
	if not store.auto_tabs() or (not rail and store.get(store.COLLAPSED_KEY) == true) then
		return
	end
	for _, p in cx.session:panes() do
		if p.block == KIND and p.tab == tab then
			return
		end
	end
	local edge = leaves(cx, tab)[1]
	local info = if edge ~= nil then cx.session:pane(edge) else nil
	if edge == nil or info == nil then
		return
	end
	local pane = cx.layout:split(edge, "left", { block = KIND, args = { "projects", info.cwd or "" } }, { focus = false })
	if pane ~= nil then
		last_mode[pane] = "projects"
		if rail then
			rail_panes[pane] = true
			if not snap_width(cx, tab, pane, store.COLLAPSED) then
				cx.layout:resize(pane, "right", -1000)
			end
		else
			snap_width(cx, tab, pane, store.width())
		end
		sync_panels(cx, pane)
	end
end

-- Parked deck management ---------------------------------------------------------------------

local function close_parked(cx: WindowCx, include_working: boolean): (number, number)
	local agents = agent_map(cx)
	local closed, kept = 0, 0
	for _, p in cx.session:panes() do
		if p.parked and p.block ~= KIND then
			local st = pane_state(p, agents[p.pane])
			if not include_working and st == "working" then
				kept += 1
			else
				cx.layout:close(p.pane)
				closed += 1
			end
		end
	end
	sync_panels(cx)
	return closed, kept
end

local function handle_pane_route(cx: WindowCx, sidebar: PaneInfo, query: string)
	local op = param(query, "op") or ""
	if op == "close-idle-parked" or op == "close-all-parked" then
		local closed, kept = close_parked(cx, op == "close-all-parked")
		if closed > 0 then
			cx:toast(
				"success",
				"Closed " .. fmt.plural(closed, "parked pane"),
				if kept > 0 then "Kept " .. fmt.plural(kept, "working chat") .. " alive." else nil
			)
		elseif kept > 0 then
			cx:toast("info", "All parked panes are working", "Right-click a specific tile to close it anyway.")
		end
		return
	end
	local raw_pane = param(query, "pane") or ""
	local pane_id = if string.match(raw_pane, "^%d+$") then tonumber(raw_pane) else nil
	local target = if pane_id ~= nil then cx.session:pane(pane_id) else nil
	if target == nil or target.block == KIND then
		return
	end
	if op == "close" then
		cx.layout:close(target.pane)
		sync_panels(cx)
	elseif op == "park" then
		local tab = target.tab
		if not target.parked and tab ~= nil and #leaves(cx, tab) >= 2 then
			local parked, err = cx.layout:park(target.pane)
			if not parked then
				cx:toast("error", "Could not park pane", tostring(err or "Tern did not park the pane."))
				return
			end
			sync_panels(cx)
		else
			cx:toast("info", "Cannot park the only pane in a tab", "Open another chat or split beside it first.")
		end
	elseif op == "show" then
		local place = param(query, "place") or "main"
		if not PLACES[place] then return end
		local tab = sidebar.tab :: number
		local main = main_pane(cx, tab)
		if show_open(cx, target, sidebar.pane, main, place) then
			finish_open(cx, sidebar, main, target.pane, place)
		else
			cx:toast("error", "Could not show pane", "Tern did not place the pane.")
		end
	end
end

-- Opens a quick picker to close unwanted parked tiles without unparking them, even when the sidebar is hidden.
local function pick_parked_to_close(cx: WindowCx)
	local agents = agent_map(cx)
	local map = open_chats()
	local saved_title: { [number]: string } = {}
	for _, entry in map do
		if type(entry) == "table" and type(entry.pane) == "number" and type(entry.title) == "string" then
			saved_title[entry.pane] = entry.title
		end
	end
	local rows: { PickItem } = {}
	local idle_n, working_n = 0, 0
	for _, p in cx.session:panes() do
		if p.parked and p.block ~= KIND then
			local ag = agents[p.pane]
			local st = pane_state(p, ag)
			if st == "working" then working_n += 1 else idle_n += 1 end
			local cmd = if ag ~= nil and type(ag.command) == "string" then ag.command elseif p.running ~= nil then p.running.line else nil
			local agent = infer_agent(cmd, p.program)
			local cwd = if type(p.cwd) == "string" then p.cwd else ""
			local label = saved_title[p.pane]
			if label == nil or label == "" then
				label = if p.title ~= "" then p.title elseif cwd ~= "" then fmt.basename(cwd) else (p.program or "Pane")
			end
			local status = if st == "working" then "Working" elseif st == "waiting" then "Waiting" elseif st == "exited" then "Exited" else "Idle"
			table.insert(rows, {
				label = label,
				detail = status .. (if cwd ~= "" then " · " .. fmt.path(cwd) else ""),
				group = "Parked tiles — Enter closes without opening",
				icon = if agent ~= nil then fmt.agent_icon(agent) elseif p.kind == "file" then "file" else "terminal",
				op = "one",
				pane = p.pane,
			})
		end
	end
	if #rows == 0 then
		cx:toast("info", "No parked panes", "Nothing is parked in the deck right now.")
		return
	end
	local items: { PickItem } = {}
	if idle_n > 1 then
		table.insert(items, {
			label = "Close all idle parked panes (" .. tostring(idle_n) .. ")",
			detail = if working_n > 0 then "Keeps " .. fmt.plural(working_n, "working chat") else "Closes every parked tile",
			group = "Batch cleanup",
			icon = "trash",
			op = "idle",
		})
	end
	for _, r in rows do
		table.insert(items, r)
	end
	cx:pick({
		items = items,
		placeholder = "Filter parked tiles to close…",
		icon = "layers",
		action = "close",
	}):next(function(choice, _err, later)
		if choice == nil or later == nil then return end
		if choice.op == "idle" then
			local closed, kept = close_parked(later, false)
			if closed > 0 then
				later:toast("success", "Closed " .. fmt.plural(closed, "parked pane"), if kept > 0 then "Kept " .. fmt.plural(kept, "working chat") .. " alive." else nil)
			end
		elseif choice.op == "one" and type(choice.pane) == "number" then
			local info = later.session:pane(choice.pane)
			if info ~= nil and info.block ~= KIND then
				later.layout:close(choice.pane)
				sync_panels(later)
			end
		end
	end)
end

-- User overrides take precedence over plugin defaults. Migrate only exact old actions.
local function migrate_keybinds(cx: WindowCx)
	local replacements = {
		["plugin.tern-sessions.sessions"] = "plugin.tern-projects.projects",
		["plugin.tern-sessions.search"] = "plugin.tern-projects.search",
	}
	for chord, value in cx.settings:keybinds() do
		if type(value) == "string" then
			local replacement = replacements[value]
			if replacement ~= nil then cx.settings:bind(chord, replacement) end
		else
			local changed = false
			local actions: { string } = {}
			for _, action in value do
				local replacement = replacements[action]
				table.insert(actions, replacement or action)
				if replacement ~= nil then changed = true end
			end
			if changed then cx.settings:bind(chord, actions) end
		end
	end
end

tern.on("window_start", function(cx: WindowCx)
	migrate_keybinds(cx)
	reconcile(cx)
	if store.collapse_style() == "rail" and store.get(store.COLLAPSED_KEY) == true then
		for _, p in cx.session:panes() do
			if p.block == KIND and not p.parked and not p.floating then
				rail_panes[p.pane] = true
			end
		end
	end
	sync_panels(cx)
end)

tern.command({
	id = "projects",
	title = "Projects",
	icon = "folder",
	keys = { if tern.runtime.os == "macos" then "cmd+alt+s" else "ctrl+alt+s" },
	run = function(cx)
		migrate_keybinds(cx)
		toggle(cx, "projects")
	end,
})

tern.command({
	id = "search",
	title = "Projects: Search",
	icon = "search",
	keys = { if tern.runtime.os == "macos" then "ctrl+alt+cmd+s" else "ctrl+alt+shift+s" },
	run = function(cx)
		migrate_keybinds(cx)
		toggle(cx, "search")
	end,
})

tern.command({
	id = "settings",
	title = "Projects: Settings",
	icon = "settings",
	run = function(cx) toggle(cx, "settings") end,
})

tern.command({
	id = "parked",
	title = "Projects: Close Parked Panes…",
	icon = "layers",
	available = function(cx)
		for _, p in cx.session:panes() do
			if p.parked and p.block ~= KIND then return true end
		end
		return false
	end,
	run = function(cx) pick_parked_to_close(cx) end,
})

-- The chat focused last in a tab is where Open in Main and the splits go.
tern.on("focus", function(ev: FocusEvent, cx: WindowCx)
	if tiled_chat(cx.session:pane(ev.pane), ev.tab) then
		main_of[ev.tab] = ev.pane
	end
	sync_panels(cx)
end)

-- Dragging a floating Tile sidebar into a split docks it back at the left edge at its remembered width.
tern.on("float_moved", function(ev: FloatMovedEvent, cx: WindowCx)
	if ev.docked ~= nil then
		local p = cx.session:pane(ev.pane)
		if p ~= nil and p.block == KIND then
			dock_sidebar(cx, ev.pane, ev.tab)
		end
	end
	sync_panels(cx)
end)

-- Clean up tracking state when a pane or tab closes.
tern.on("pane_closed", function(ev: PaneEvent, cx: WindowCx)
	last_mode[ev.pane] = nil
	prev_focus[ev.pane] = nil
	parked_from[ev.pane] = nil
	was_parked[ev.pane] = nil
	nudged[ev.pane] = nil
	rail_panes[ev.pane] = nil
	last_live_sig[ev.pane] = nil
	for tab, pane in main_of do
		if pane == ev.pane then
			main_of[tab] = nil
		end
	end
	forget(ev.pane)
	-- A sidebar left alone in its tab fills it; that width is no choice of the user's.
	for _, p in cx.session:panes() do
		if p.block == KIND and p.tab ~= nil and #leaves(cx, p.tab) == 1 then
			cx.session:event(p.pane, { ev = "action", id = "main", act = "sidebar-hold" })
		end
	end
	sync_panels(cx)
end)

-- A sidebar hidden from a closing tab has nowhere to return; close it rather than keep it parked.
tern.on("tab_closed", function(ev: TabEvent, cx: WindowCx)
	main_of[ev.tab] = nil
	new_tabs[ev.tab] = nil
	sidebar_tabs[ev.tab] = nil
	for pane, tab in parked_from do
		if tab == ev.tab then
			parked_from[pane] = nil
			local info = cx.session:pane(pane)
			if info ~= nil and info.block == KIND and info.parked then
				cx.layout:close(pane)
			end
		end
	end
	sync_panels(cx)
end)

-- New tabs get the sidebar at their left edge. A tab's first pane may arrive before or after the tab's own
-- event (and `cx` is gone once a handler returns), so whichever comes second docks it.
local function dock_new(cx: WindowCx, tab: number)
	local ok, err = pcall(auto_sidebar, cx, tab)
	if not ok then
		tern.log.warn("tern-projects: couldn't open the sidebar in a new tab", err)
	end
end

tern.on("tab_created", function(ev: TabEvent, cx: WindowCx)
	if leaves(cx, ev.tab)[1] ~= nil then
		dock_new(cx, ev.tab)
	else
		new_tabs[ev.tab] = true
	end
end)

tern.on("pane_created", function(ev: PaneEvent, cx: WindowCx)
	local info = cx.session:pane(ev.pane)
	local tab = if info ~= nil then info.tab else nil
	if tab ~= nil and new_tabs[tab] then
		new_tabs[tab] = nil
		dock_new(cx, tab)
	end
	sync_panels(cx)
end)

for _, ev_name in { "cwd", "title", "command_started", "command_finished", "agent_state", "agent_finished" } do
	tern.on(ev_name, function(_ev: any, cx: WindowCx)
		reconcile(cx)
		sync_panels(cx)
	end)
end

-- The status line (when the status bar is on) offers Projects in tabs without a sidebar, and a Parked
-- counter when parked tiles exist so they can be closed in one picker without unparking.
tern.chrome.status(function(pane: StatusPaneInfo): { StatusSegment }?
	local segs: { StatusSegment } = {}
	if pane.tab ~= nil and not sidebar_tabs[pane.tab] then
		table.insert(segs, { text = "Projects", icon = "folder", tone = "muted", command = "plugin.tern-projects.projects" })
	end
	if parked_count > 0 then
		table.insert(segs, { text = "Parked · " .. tostring(parked_count), icon = "layers", tone = "muted", command = "plugin.tern-projects.parked" })
	end
	return if #segs > 0 then segs else nil
end)


-- Panel-generated requests carry a shared capability token (`sidebar.auth`) and name their `src` panel pane,
-- so right-clicking or clicking buttons in an unfocused sidebar works while links in terminal/chat output can
-- never forge a launch, close or resize.
tern.route.link(function(link: LinkRequest, cx: WindowCx): RouteDecision?
	local route, q = string.match(link.url, "^tern%-projects://([a-z]+)%?(.*)$")
	if route ~= "resume" and route ~= "launch" and route ~= "sidebar" and route ~= "pane" then return nil end
	local query = q :: string
	local expected = store.get(store.AUTH_KEY)
	if type(expected) ~= "string" or expected == "" or param(query, "tok") ~= expected then
		return { handled = true }
	end
	local src = tonumber(param(query, "src") or "")
	if src == nil and route == "sidebar" then
		src = tonumber(param(query, "pane") or "")
	end
	local from = src or link.pane
	local origin = if from ~= nil then cx.session:pane(from) else nil
	if origin == nil or origin.block ~= KIND or origin.tab == nil then
		return { handled = true }
	end
	if route == "sidebar" then
		local state, cols = param(query, "state"), param(query, "cols")
		if state == "sync" then
			sync_panels(cx, origin.pane)
		elseif (state == "collapse" or state == "expand" or state == "fit") and (cols == nil or string.match(cols, "^[1-9]%d?%d?$")) then
			size_sidebar(cx, origin.pane, state, if cols ~= nil then tonumber(cols) else nil, param(query, "rail") == "1")
		end
		return { handled = true }
	end
	if route == "pane" then
		handle_pane_route(cx, origin, query)
		return { handled = true }
	end
	if cx.hosts:current().kind ~= "local" then
		cx:toast("error", "Cannot launch a local chat on a remote host", "Switch to the local host first.")
		return { handled = true }
	end
	local cwd = param(query, "cwd") or ""
	local cmd = param(query, "cmd") or ""
	if cmd == "" or cwd == "" or string.find(cwd, "[%z\r\n]") or string.find(cmd, "[%z\r\n]") then
		cx:toast("error", "Invalid chat launch", "The working directory or command is missing or invalid.")
		return { handled = true }
	end
	local place = param(query, "place") or "main"
	local uid = if route == "resume" then param(query, "uid") else nil
	local title = param(query, "title")
	if not PLACES[place] or (uid ~= nil and (uid == "" or #uid > 256 or string.find(uid, "[%c&]") ~= nil)) then
		cx:toast("error", "Invalid chat launch", "The placement or chat ID is invalid.")
		return { handled = true }
	end
	-- Session ids: up to 64 plain ids (letters, digits, `-`, `_`), comma separated.
	local sessions: { string }? = nil
	local raw = if route == "resume" then param(query, "sessions") else nil
	if raw ~= nil then
		local list: { string } = {}
		for id in string.gmatch(raw, "[^,]+") do
			if #id > 128 or string.find(id, "^[%w_%-]+$") == nil or #list >= 64 then
				cx:toast("error", "Invalid chat launch", "The chat's session ids are invalid.")
				return { handled = true }
			end
			table.insert(list, id)
		end
		sessions = if #list > 0 then list else nil
	end
	open_chat(cx, origin, { cwd = cwd, cmd = cmd, place = place, uid = uid, title = title, sessions = sessions })
	return { handled = true }
end)
