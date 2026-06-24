# The Luminous portal:

An alternative to xdg-desktop-portal-wlr for wlroots compositors. This project is a stand alone binary and does not depend on grim.
`libwayshot` is used as the screencopy backend to enable screenshots.

![https://github.com/waycrate/xdg-desktop-portal-luminous/actions](https://github.com/waycrate/xdg-desktop-portal-luminous/actions/workflows/ci.yaml/badge.svg)

# Exposed interfaces:

1. org.freedesktop.impl.portal.RemoteDesktop
1. org.freedesktop.impl.portal.ScreenCast
1. org.freedesktop.impl.portal.ScreenShot
1. org.freedesktop.impl.portal.Settings
1. org.freedesktop.impl.portal.InputCapture
1. org.freedesktop.impl.portal.Background
1. org.freedesktop.impl.portal.Clipboard

# Settings:

Luminous is configured through the following auto hot-reloaded file: `$XDG_CONFIG_FILE/xdg-desktop-portal-luminous/config.toml`.

And under `/etc/xdg/xdg-desktop-portal-luminous/config.toml`

```toml
color_scheme = "dark" # can also be "light"
accent_color = "#880022"
contrast = "higher" # enable higher contrast
reduced_motion = "reduced" # enable reduced motion
screenshot_permission_check = false # disable the permission check dialog
```

# How to set priority of portal backend:

The following file needs to be created `~/.config/xdg-desktop-portal/CURRENT_DESKTOP_NAME-portals.conf`.
(eg: For the `sway` desktop, `sway-portals.conf` must exist.)

Eg:
```
[preferred]
default=luminous
org.freedesktop.impl.portal.Settings=luminous;gtk
```

# Background autostart:

Applications that use `org.freedesktop.portal.Background.RequestBackground` can be autostarted by `xdg-desktop-portal`: when the request is granted, the frontend writes an XDG autostart desktop entry, which `systemd-xdg-autostart-generator` turns into an `app-*@autostart.service` unit on the next login.

Native apps have a bootstrap requirement before that can happen: the first `RequestBackground` call must come from a process whose application ID the portal frontend can identify. Flatpak and snap apps provide this through sandbox metadata, but native apps normally need to be launched by a session or launcher that puts them in an `app-*` systemd scope or service. uwsm or a one-off `systemd-run --user --scope --unit=app-...scope ...` command can provide that scope. A bare native launch cannot be granted background autostart, so no portal-written autostart entry is created.

Generated autostart units only run if the session starts `xdg-desktop-autostart.target`, or a compositor-specific wrapper target that starts it. Some wlroots sessions do not do this by default. For example, a Sway session can start its wrapper target from the compositor config:

```conf
exec systemctl --user start sway-xdg-autostart.target
```

Non-Flatpak apps have one more requirement: their `Exec=` command must be resolvable by the systemd user manager. A bare command such as `Exec=MyApp` is silently skipped by `systemd-xdg-autostart-generator` when the binary lives outside the manager's `PATH` (for example in `~/.local/bin`, which the user manager does not include by default). The app itself can avoid this by passing an absolute command to `RequestBackground`; a user can instead add the directory to the user manager environment (hand-editing the generated entry does not stick, since the portal rewrites it on the app's next request):

```ini
# ~/.config/environment.d/10-local-bin.conf
PATH=${HOME}/.local/bin:${PATH}
```

After changing `environment.d`, log out and back in or reboot so the user manager re-runs its generators and the autostart target starts the generated unit. `systemctl --user daemon-reload` re-runs the generators too, but it does not start a newly generated autostart unit; start that unit or restart the autostart target if you want to test it in the current session.

# Future goals:

* Do not rely on slurp binary. We feel calling binaries is a hack to achieve some end goal, it is almost always better to programmatically invoke the given API.

# Building:

```sh
meson build
ninja -C build install
```

# Requirements:

Build time requirements are marked with `*`.

1. cargo *
1. libclang *
1. meson *
1. ninja *
1. pipewire *
1. pkg-config *
1. rustc *
1. xkbcommon *
1. slurp
1. wayland
1. wayland-protocols *
1. glib-2.0 *
