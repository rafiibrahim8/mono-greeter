#!/bin/sh
# Installs mono-greeter system-wide and points greetd at it. Run as root.
#
#   sudo ./install.sh                  from the release tarball, into /usr/local
#   sudo PREFIX=/usr ./install.sh      into /usr
#   --from DIR                         take the binary from DIR (default: next to this script)
#   DESTDIR=/tmp/root ./install.sh     stage into a directory, touching nothing live
#
# greetd itself must be installed (it creates the greeter user). This does not enable greetd or
# disable the current display manager; see the README for switching over.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
from="$here"
if [ "${1:-}" = "--from" ]; then from="$(cd "$2" && pwd)"; fi
prefix="${PREFIX:-/usr/local}"
dest="${DESTDIR:-}"
share="$prefix/share/mono-greeter"
# greetd runs the binary; it is not a command for users, so it lives outside bin/
libexec="$prefix/lib/mono-greeter"

[ -x "$from/mono-greeter" ] || { echo "missing $from/mono-greeter" >&2; exit 1; }
if [ -z "$dest" ]; then
  [ "$(id -u)" -eq 0 ] || { echo "run as root (sudo)" >&2; exit 1; }
  id greeter >/dev/null 2>&1 || { echo "no 'greeter' user: install greetd first (pacman -S greetd)" >&2; exit 1; }
fi

# The shipped files name /usr/lib and /usr/share; point them at this prefix.
rewrite() {
  sed -e "s|/usr/lib/mono-greeter/|$libexec/|" -e "s|/usr/share/mono-greeter/|$share/|" "$1" > "$2"
  chmod 0644 "$2"
}

# Config under /etc: keep a copy the admin changed and put the shipped one next to it, like pacman.
install_conf() {
  if [ -e "$2" ] && [ "$(sha256sum < "$1")" != "$(sha256sum < "$2")" ]; then
    install -Dm644 "$1" "$2.new"
    echo "kept your $2; the shipped version is $2.new"
  else
    install -Dm644 "$1" "$2"
  fi
}

install -Dm755 "$from/mono-greeter" "$dest$libexec/mono-greeter"
# versions up to 0.1.2 installed it into bin/
rm -f "$dest$prefix/bin/mono-greeter"
install -dm755 "$dest$share" "$dest/etc/systemd/system/greetd.service.d"
rewrite "$here/greetd.toml" "$dest$share/greetd.toml"
rewrite "$here/greetd-test-vt2.toml" "$dest$share/greetd-test-vt2.toml"
install_conf "$here/foot.ini" "$dest/etc/mono-greeter/foot.ini"
install_conf "$here/pam.d/mono-greeter" "$dest/etc/pam.d/mono-greeter"
install -Dm644 "$here/tmpfiles.conf" "$dest/etc/tmpfiles.d/mono-greeter.conf"
# Last: from here on greetd uses the config above, which needs the PAM file (a missing PAM service
# denies every login).
rewrite "$here/greetd.service.d/mono-greeter.conf" "$dest/etc/systemd/system/greetd.service.d/mono-greeter.conf"

if [ -z "$dest" ]; then
  systemd-tmpfiles --create /etc/tmpfiles.d/mono-greeter.conf
  systemctl daemon-reload
fi

cat <<EOF
Installed mono-greeter to $libexec; greetd now starts it (drop-in in /etc/systemd/system/greetd.service.d).
To make greetd the login screen, disable the current display manager (if any) and enable greetd:
  readlink /etc/systemd/system/display-manager.service   # the current one; no output means none
  systemctl disable <that display manager> && systemctl enable greetd
Trial on tty2 first, with the current display manager still running:
  greetd --config $share/greetd-test-vt2.toml
EOF
