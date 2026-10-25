# 安装前后均校验完整策略，只替换带标记的专用文件
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
LC_ALL=C
export LC_ALL
umask 077
fail() { printf '%s\n' "ShellRS: $1" >&2; exit 1; }
test "$(id -u)" = 0 || fail 'Root permission required'
uid=${SUDO_UID:-}
case "$uid" in ''|*[!0-9]*) fail 'Invalid invoking user';; esac
test "$uid" != 0 || fail 'Invalid invoking user'
command -v visudo >/dev/null 2>&1 || fail 'visudo unavailable'
test -x /usr/bin/head || fail 'Reader unavailable'
dir=/etc/sudoers.d
test -d "$dir" && test ! -L "$dir" || fail 'Unsafe sudoers directory'
test "$(stat -c %u "$dir")" = 0 || fail 'Unsafe sudoers owner'
mode=$(stat -c %a "$dir")
test "$((0$mode & 0022))" = 0 || fail 'Unsafe sudoers permissions'
visudo -c >/dev/null 2>&1 || fail 'Existing sudoers policy is invalid'
target="$dir/shellrs-monitor-$uid"
lock="$dir/.shellrs-monitor-lock"
mkdir "$lock" 2>/dev/null || fail 'Another setup is running'
work=
changed=no
saved=no
cleanup() {
    code=$?
    trap - EXIT HUP INT TERM
    if test "$changed" = yes; then
        if test "$saved" = yes; then
            mv -f "$work/previous" "$target" || { printf '%s\n' 'ShellRS: Rollback failed; backup retained' >&2; exit 1; }
        else
            rm -f "$target" || code=1
        fi
    fi
    test -z "$work" || rm -rf "$work"
    rmdir "$lock"
    exit "$code"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
work=$(mktemp -d "$dir/.shellrs-monitor-XXXXXX")
if test -e "$target" || test -L "$target"; then
    test -f "$target" && test ! -L "$target" || fail 'Unsafe destination'
    test "$(/usr/bin/head -n 1 "$target")" = '# ShellRS managed monitor access v1' || fail 'Destination is not managed by ShellRS'
    cp -p "$target" "$work/previous"
    saved=yes
fi
{
    printf '%s\n' '# ShellRS managed monitor access v1'
    printf '#%s ALL=(root) NOPASSWD: %s\n' "$uid" '@COMMANDS@'
} > "$work/rule"
chown 0:0 "$work/rule"
chmod 0440 "$work/rule"
visudo -cf "$work/rule" >/dev/null 2>&1 || fail 'Generated rule is invalid'
changed=yes
mv -f "$work/rule" "$target"
checked=$(visudo -c 2>&1) || fail 'Installed policy is invalid'
case "$checked" in *"$target:"*) ;; *) fail 'sudoers.d is not included';; esac
changed=no
rm -f "$work/previous"
printf '%s\n' '@@shellrs-monitor-access ready'
