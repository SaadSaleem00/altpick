# filepick hotkey for Git Bash.
#
# While typing a command, press Alt+F. The filepick palette opens with the
# files in the current folder. Pick one with the arrow keys and press Enter:
# its name is typed at your cursor. Esc cancels.
#
# Turn it on in this window (or add the line to ~/.bashrc):
#
#   source /c/path/to/filepick/shell/filepick.bash
#
# filepick.exe is looked for in this order: $FILEPICK_EXE, next to this
# script, then on PATH. The key comes from the filepick settings file
# (hotkey = "Alt+f" in %APPDATA%\filepick\config.toml). FILEPICK_KEY, set
# before sourcing, overrides it, in readline's notation: '\eOQ' for F2.

# Only interactive shells with readline have key bindings.
case $- in
*i*) ;;
*) return 0 2>/dev/null || exit 0 ;;
esac

__filepick_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)

__filepick_exe() {
    if [ -n "$FILEPICK_EXE" ] && [ -f "$FILEPICK_EXE" ]; then
        printf '%s' "$FILEPICK_EXE"
    elif [ -n "$__filepick_dir" ] && [ -f "$__filepick_dir/filepick.exe" ]; then
        printf '%s' "$__filepick_dir/filepick.exe"
    else
        printf '%s' filepick.exe
    fi
}

# Runs the palette in the current folder and inserts the pick at the cursor.
# On any error it leaves the command line alone.
__filepick_pick() {
    local tmp pick exe
    # The pick comes back through a temp file (UTF-8), so names in any
    # language survive. The palette draws on the console directly.
    tmp=$(mktemp 2>/dev/null) || return 0
    exe=$(__filepick_exe)

    # filepick.exe is a Windows program, so it gets a Windows path for --out.
    if "$exe" --out "$(cygpath -w "$tmp")" --quote sh 2>/dev/null; then
        pick=$(cat "$tmp" 2>/dev/null)
    fi
    rm -f "$tmp"

    if [ -n "$pick" ]; then
        READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}${pick}${READLINE_LINE:READLINE_POINT}"
        READLINE_POINT=$((READLINE_POINT + ${#pick}))
    fi
    return 0
}

__filepick_key=$FILEPICK_KEY
if [ -z "$__filepick_key" ]; then
    __filepick_key=$("$(__filepick_exe)" key 2>/dev/null | tr -d '\r')
fi
case $__filepick_key in
'\'?*) ;;
*) __filepick_key='\ef' ;;
esac
bind -x "\"$__filepick_key\": __filepick_pick"
unset __filepick_key
