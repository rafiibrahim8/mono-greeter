# mono-greeter

A minimal, keyboard-first login screen for [greetd](https://sr.ht/~kennylevinsen/greetd/).

```
  archbox · tty1 · greetd                                            Sun, 4 Oct 2026

  ╶────╮  ╭────╮     ╶────╮  ╭────╮
       │  │    │  •       │  │    │
  ╭────╯  │    │     ╶────┤  ├────┤
  │       │    │  •       │  │    │
  ╰────╴  ╰────╯     ╶────╯  ╰────╯

  Arch Linux · 7.2.8-arch1-2

  login   alex
› pass    ********
  sess    ‹ Plasma (Wayland) wayland ›

                     F1 help  F2 users  F3 sessions  F4 power  F7 show pass  F9 ipc
```

It runs as a terminal app in [foot](https://codeberg.org/dnkl/foot), shown fullscreen by
[cage](https://github.com/cage-kiosk/cage): a real font, exact colours, full keyboard input, and
nothing else on screen.

- Lists your users and installed sessions: Wayland, X11, and a plain shell (each user's own).
- Remembers the last user, and each user's last session.
- Unlocks KDE Wallet / GNOME Keyring at login, and handles extra PAM prompts such as one-time
  codes or an expired password.
- Shows the password with F7, or only while Alt is held. Warns when Caps Lock is on.
- Suspend, reboot and power off from the login screen.
- Made for the keyboard, and the mouse works too.
- Uses no CPU while idle.

## Try it first

No installing, no root, no greetd. In demo mode any listed user signs in with the password `demo`,
and signing in only prints the command that would start the session. From a source checkout:

```sh
cargo run --release -- --demo
foot --config dist/foot.ini target/release/mono-greeter --demo    # the real look, in foot
```

## Install

Requires `greetd`, `cage`, `foot` and `ttf-hack-nerd`. Optional: `xorg-xinit` for X11 sessions,
`kwallet-pam` or `gnome-keyring` to unlock the wallet/keyring at login.

- **Arch Linux:** `mono-greeter-bin` from the linux-gems repository.
- **Release tarball:** unpack it, then `sudo ./install.sh`. That installs into `/usr/local`;
  `sudo PREFIX=/usr ./install.sh` installs into `/usr`.
- **From source:** `./install.sh` builds with cargo, then installs with sudo.

Installing doesn't change your login screen yet, and greetd's own config files are left alone.

## Make it your login screen

**1. Try it on tty2,** with your current login screen still running:

```sh
sudo greetd --config /usr/share/mono-greeter/greetd-test-vt2.toml   # /usr/local/share/... for tarball and source installs
```

Sign in with the **Shell** session: a second desktop for a user who is already logged in can
clash. Ctrl+Alt+F1…F7 takes you back to your desktop; Ctrl+C where greetd runs ends the trial.

**2. Switch.** Find the display manager that's enabled now:

```sh
readlink /etc/systemd/system/display-manager.service   # no output means none
```

Note its name (for example `sddm.service` or `gdm.service`), then:

```sh
sudo systemctl disable <that display manager>   # skip if there was none
sudo systemctl enable greetd
reboot
```

## Keys

| Key | Does |
|---|---|
| Tab / Shift+Tab, ↑ ↓ | move through login, pass, sess and the key bar |
| ← → | change session; move along the key bar |
| Enter | next field / sign in |
| Esc | cancel signing in, clear the password |
| Ctrl+U, Ctrl+W | clear the field, delete a word |
| F1 | help |
| F2, F3, F4 | users, sessions, power (Tab inside a menu moves to the next one) |
| F7, hold Alt | show the password: toggle, or only while held |
| F9 | the messages exchanged with greetd |
| Mouse | click a field, a key or a menu item; click ‹ › or scroll to change the session; click outside a menu to close it |

## Configuration

| What | Where |
|---|---|
| Font and size | `/etc/mono-greeter/foot.ini` (any monospace font foot can use) |
| Login rules (PAM) | `/etc/pam.d/mono-greeter`, for example to add a security key or fingerprint |
| Sessions | `.desktop` files in `/usr/share/wayland-sessions` and `/usr/share/xsessions` |
| Users | accounts in `/etc/passwd` in the normal-user UID range, with a login shell |
| greetd setup | `share/mono-greeter/greetd.toml`, started through a greetd.service drop-in; put your own drop-in in `/etc/systemd/system/greetd.service.d/` to change it |

Package upgrades keep your edits to `foot.ini` and the PAM file.

## Troubleshooting

**Locked out.** Press Ctrl+Alt+F3 for a text login, log in, then go back to your previous display
manager, or to none:

```sh
sudo systemctl disable greetd
sudo systemctl enable <previous display manager>   # skip if there was none
reboot
```

If no text login appears at all, press `e` in the boot menu, add `systemd.unit=multi-user.target`
to the kernel line and boot. That gives a text login with no display manager.

**Logs.** `journalctl -b -t mono-greeter` (the greeter, foot and cage) and
`journalctl -b -u greetd`. On the login screen, F9 shows each step of signing in.

**Wallet doesn't unlock.** Install `kwallet-pam` or `gnome-keyring`. The wallet must use your login
password; a KDE wallet must also be named `kdewallet`.

## Uninstall

Re-enable another display manager first if greetd is your login screen.

- **Package:** `sudo pacman -R mono-greeter-bin`
- **Tarball or source install:**
  ```sh
  sudo rm /usr/local/bin/mono-greeter /etc/tmpfiles.d/mono-greeter.conf /etc/pam.d/mono-greeter
  sudo rm /etc/systemd/system/greetd.service.d/mono-greeter.conf
  sudo rm -r /usr/local/share/mono-greeter /etc/mono-greeter /var/cache/mono-greeter
  sudo systemctl daemon-reload
  ```

## Known limits

- Dead keys and Compose sequences on non-US keyboard layouts may not work.
- Outside foot, for example on a plain text console: 16 colours, no hold-Alt, no Caps Lock warning.

## License

MIT
