--!strict
-- Small persistent state in `tern.kv` (`<plugin.data>/kv.json`), shared by the host block and every window.
-- Keys:
--   sidebar.width      number   last expanded sidebar width in cells (recorded by the panel, applied by the window)
--   sidebar.collapsed  boolean  whether the sidebar was last collapsed (hidden, narrowed to its rail, or a tile)
--   sidebar.collapse   string   how Collapse works: "hide" (park the pane; default), "rail" (narrow icon rail) or
--                               "tile" (a floating card over the main chat's top-left corner)
--   sidebar.tab_cols   number   cols of a tab's tiled area, measured by a sidebar; lets the window size a sidebar
--                               to the remembered width in the same step that docks it
--   sidebar.expanded   object   project row key ("project:<id>" / "discovered:<root>") -> true when unfolded
--   chats.open         object   chat uid -> {pane = number, cwd = string} of the pane running it (window only)
--   sidebar.auto_tabs  boolean  open a Projects sidebar in every new tab (absent = on; Settings can turn it off)
--   sidebar.card       boolean  show the selected chat's or folder's card at the bottom of the sidebar (absent = on)
--   sidebar.parked_folded boolean whether the sidebar's Parked section is folded (absent = unfolded)
--   sidebar.auth       string   per-session capability token shared by the panel VM and window routes
local store = {}

store.WIDTH_KEY = "sidebar.width"
store.COLLAPSED_KEY = "sidebar.collapsed"
store.STYLE_KEY = "sidebar.collapse"
store.TAB_COLS_KEY = "sidebar.tab_cols"
store.EXPANDED_KEY = "sidebar.expanded"
store.OPEN_KEY = "chats.open"
store.AUTO_KEY = "sidebar.auto_tabs"
store.CARD_KEY = "sidebar.card"
store.PARKED_FOLD_KEY = "sidebar.parked_folded"
store.AUTH_KEY = "sidebar.auth"

-- Expanded width when none is stored, the icon rail's target width (Tern keeps a split at least a tenth of
-- its parent, so the rail usually ends wider), and the narrowest width remembered as an expanded width
-- (anything narrower is the rail or a transient layout).
store.WIDTH = 44
store.COLLAPSED = 6
store.MIN_WIDTH = 20

-- The stored value, or nil when it is missing or the store can't be read.
function store.get(key: string): any
	local ok, value = pcall(tern.kv.get, key)
	return if ok then value else nil
end

function store.set(key: string, value: any)
	local ok, err = pcall(tern.kv.set :: any, key, value)
	if not ok then tern.log.warn("tern-projects: couldn't store " .. key, err) end
end

-- The remembered expanded sidebar width.
function store.width(): number
	local w = store.get(store.WIDTH_KEY)
	return if type(w) == "number" and w >= store.MIN_WIDTH and w <= 400 then math.floor(w) else store.WIDTH
end

-- The last measured cols of a tab's tiled area, or nil before any sidebar measured one.
function store.tab_cols(): number?
	local w = store.get(store.TAB_COLS_KEY)
	return if type(w) == "number" and w >= 20 and w <= 2000 then w else nil
end

-- Whether new tabs get a Projects sidebar automatically; on until the user turns it off in Settings.
function store.auto_tabs(): boolean
	return store.get(store.AUTO_KEY) ~= false
end

-- Whether selecting a chat or folder shows its card at the sidebar's foot; on until turned off in Settings.
-- Off, a click only selects; the row's right-click menu holds the same actions.
function store.card(): boolean
	return store.get(store.CARD_KEY) ~= false
end

-- Whether the Parked section at the top of Projects is folded; unfolded by default so parked tiles are easy to close.
function store.parked_folded(): boolean
	return store.get(store.PARKED_FOLD_KEY) == true
end

-- How Collapse works: "hide" parks the sidebar until it is reopened, "rail" narrows it to an icon rail, "tile"
-- floats it as a card over the main chat.
function store.collapse_style(): string
	local style = store.get(store.STYLE_KEY)
	return if style == "rail" or style == "tile" then style else "hide"
end

return store
