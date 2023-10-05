# xdg-desktop-portal-luminous

An alternative to xdg-desktop-portal-wlr for wlroots compositors. This project is a stand alone binary and does not depend on grim.
`libwayshot` is used as the screencopy backend to enable screenshots.

![https://github.com/waycrate/xdg-desktop-portal-luminous/actions](https://github.com/waycrate/xdg-desktop-portal-luminous/actions/workflows/ci.yaml/badge.svg)

# Exposed interfaces:

1. org.freedesktop.impl.portal.ScreenCast
1. org.freedesktop.impl.portal.ScreenShot
1. org.freedesktop.impl.portal.RemoteDesktop
1. org.freedesktop.impl.portal.Settings

# NOTE :How to set in in the newest xdg-desktop-portal

to use Settings, you need to create `~/.config/xdg-desktop-portal/CURRENT_DESKTOP_NAME-portals.conf`, for example, if you name is setted as `sway`, you need to create `sway.conf`

And write into it like

`gtk` is preferred to use when use Settings backend

```
[preferred]
default=luminous
org.freedesktop.impl.portal.Settings=luminous;gtk
```

# About settings

You need to create `~/.config/xdg-desktop-portal-luminous/config.toml`

write config like

```toml
color_scheme = "dark"
accent_color = "#880022"
```
`color_scheme` can be `dark` or `light`. When the file changed, the settings will be applied immediately. You will see the changes in chromium and firefox.

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
