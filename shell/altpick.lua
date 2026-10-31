-- altpick hotkey for CMD, through Clink (https://chrisant996.github.io/clink/).
--
-- While typing a command, press Alt+F. The altpick palette opens with the
-- files in the current folder. Pick one with the arrow keys and press Enter:
-- its name is typed at your cursor. Esc cancels.
--
-- Clink loads every .lua file in its script folders. To try this one in a
-- single CMD window without changing any settings:
--
--   set ALTPICK_EXE=C:\path\to\altpick.exe
--   clink inject --scripts C:\path\to\altpick\shell
--
-- altpick.exe is looked for in this order: %ALTPICK_EXE%, next to this
-- script, then on PATH.
--
-- The key comes from the altpick settings file (hotkey = "Alt+f" in
-- %APPDATA%\altpick\config.toml), via `altpick.exe key`. You can also bind
-- one in Clink's .inputrc, e.g.
--   "\e[12~": "luafunc:altpick_pick"     (F2)

-- Two copies can be loaded at once (say, one registered by `altpick setup`
-- and one from `clink inject --scripts`, which Clink loads first). The first
-- one wins; later ones step aside.
if ALTPICK_LOADED then
    return
end
ALTPICK_LOADED = true

local function script_dir()
    local source = debug.getinfo(1, "S").source
    return source:match("^@(.*)[\\/][^\\/]*$")
end

local function find_exe()
    local exe = os.getenv("ALTPICK_EXE")
    if exe and os.isfile(exe) then
        return exe
    end
    local dir = script_dir()
    if dir then
        local nearby = path.join(dir, "altpick.exe")
        if os.isfile(nearby) then
            return nearby
        end
    end
    return "altpick.exe"
end

-- Runs the palette in the current folder and returns the picked name,
-- already quoted for CMD, or nil.
local function run_palette()
    -- The pick comes back through a temp file (UTF-8), so names in any
    -- language survive. The palette draws on the console directly.
    local file, tmp = os.createtmpfile("altpick", ".txt")
    if not file then
        return nil
    end
    file:close()

    -- os.execute goes through "cmd /c", which strips the first and last
    -- quote of the line, hence the extra pair around everything. The child
    -- starts in CMD's current folder, so no --dir is needed. 2>nul keeps a
    -- "not recognized" message off the screen if the exe can't be found.
    local command = '""' .. find_exe() .. '" --out "' .. tmp .. '" --quote cmd 2>nul"'
    local ok, _, code = os.execute(command)

    local pick
    if ok and code == 0 then
        local r = io.open(tmp, "rb")
        if r then
            pick = r:read("*a")
            r:close()
        end
    end
    os.remove(tmp)
    return pick
end

-- Global, so Clink can bind it as "luafunc:altpick_pick".
function altpick_pick(rl_buffer)
    -- Never break the user's command line because of the palette.
    local ok, pick = pcall(run_palette)
    if ok and pick and pick ~= "" then
        rl_buffer:insert(pick)
    end
    rl_buffer:refreshline()
end

if rl.describemacro then
    rl.describemacro([["luafunc:altpick_pick"]], "Pick a file from the current folder and type its name at the cursor")
end
-- The hotkey in readline notation; Alt+F (\ef) unless the settings say otherwise.
local function hotkey()
    local ok, key = pcall(function()
        local r = io.popen('2>nul ""' .. find_exe() .. '" key"')
        if not r then
            return nil
        end
        local out = r:read("*l")
        r:close()
        return out
    end)
    if ok and key and key:match("^\\[%w%[%]~%-\\]+$") then
        return key
    end
    return [[\ef]]
end

rl.setbinding('"' .. hotkey() .. '"', [["luafunc:altpick_pick"]])
