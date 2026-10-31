# altpick hotkey for Git Bash.
#
# While typing a command, press Alt+F. The altpick palette opens with the
# files in the current folder. Pick one with the arrow keys and press Enter:
# its name is typed at your cursor. Esc cancels.
#
# Turn it on in this window (or add the line to ~/.bashrc):
#
#   source /c/path/to/altpick/shell/altpick.bash
#
# altpick.exe is looked for in this order: $ALTPICK_EXE, next to this
# script, then on PATH. The key comes from the altpick settings file
# (hotkey = "Alt+f" in %APPDATA%\altpick\config.toml). ALTPICK_KEY, set
# before sourcing, overrides it, in readline's notation: '\eOQ' for F2.

# Only interactive shells with readline have key bindings.
case $- in
*i*) ;;
*) return 0 2>/dev/null || exit 0 ;;
esac

__altpick_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)

__altpick_exe() {
    if [ -n "$ALTPICK_EXE" ] && [ -f "$ALTPICK_EXE" ]; then
        printf '%s' "$ALTPICK_EXE"
    elif [ -n "$__altpick_dir" ] && [ -f "$__altpick_dir/altpick.exe" ]; then
        printf '%s' "$__altpick_dir/altpick.exe"
    else
        printf '%s' altpick.exe
    fi
}

# Runs the palette in the current folder and inserts the pick at the cursor.
# On any error it leaves the command line alone.
__altpick_pick() {
    local tmp pick exe
    # The pick comes back through a temp file (UTF-8), so names in any
    # language survive. The palette draws on the console directly.
    tmp=$(mktemp 2>/dev/null) || return 0
    exe=$(__altpick_exe)

    # In mintty (Git Bash's own window) the terminal is a /dev/pty, not a
    # Windows console. Windows programs then run behind a hidden console that
    # keeps the palette's alternate-screen switch to itself and blanks
    # mintty's real screen instead. So switch mintty to its alternate screen
    # here: the blanking lands there, and mintty restores the real screen.
    local own_screen=
    case $(tty 2>/dev/null) in
    /dev/pty*) own_screen=1 ;;
    esac
    [ -n "$own_screen" ] && printf '\e[?1049h' >/dev/tty

    # altpick.exe is a Windows program, so it gets a Windows path for --out.
    if "$exe" --out "$(cygpath -w "$tmp")" --quote sh 2>/dev/null; then
        pick=$(cat "$tmp" 2>/dev/null)
    fi
    rm -f "$tmp"

    [ -n "$own_screen" ] && printf '\e[?1049l' >/dev/tty

    if [ -n "$pick" ]; then
        READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}${pick}${READLINE_LINE:READLINE_POINT}"
        READLINE_POINT=$((READLINE_POINT + ${#pick}))
    fi
    return 0
}

__altpick_key=$ALTPICK_KEY
if [ -z "$__altpick_key" ]; then
    __altpick_key=$("$(__altpick_exe)" key 2>/dev/null | tr -d '\r')
fi
case $__altpick_key in
'\'?*) ;;
*) __altpick_key='\ef' ;;
esac
bind -x "\"$__altpick_key\": __altpick_pick"
unset __altpick_key
