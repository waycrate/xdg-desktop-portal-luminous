# The Luminous portal:

An alternative to xdg-desktop-portal-wlr for wlroots compositors. This project is a stand alone binary and does not depend on grim.
`libwayshot` is used as the screencopy backend to enable screenshots.

![https://github.com/waycrate/xdg-desktop-portal-luminous/actions](https://github.com/waycrate/xdg-desktop-portal-luminous/actions/workflows/ci.yaml/badge.svg)

# Exposed interfaces:

1. org.freedesktop.impl.portal.RemoteDesktop
1. org.freedesktop.impl.portal.ScreenCast
1. org.freedesktop.impl.portal.ScreenShot
1. org.freedesktop.impl.portal.Settings

# Settings:

Luminous is configured through the following auto hot-reloaded file: `~/.config/xdg-desktop-portal-luminous/config.toml`.

```toml
color_scheme = "dark" # can also be "light"
accent_color = "#880022"
contrast = "higher" # enable higher contrast
reduced_motion = "reduced" # enable reduced motion
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
