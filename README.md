# xdg-desktop-portal-luminous

An alternative to xdg-desktop-portal-wlr for wlroots compositors. This project is a stand alone binary and does not depend on grim.
`libwayshot` is used as the screencopy backend to enable screenshots.

![https://github.com/waycrate/xdg-desktop-portal-luminous/actions](https://github.com/waycrate/xdg-desktop-portal-luminous/actions/workflows/ci.yaml/badge.svg)

# Exposed interfaces:

1. org.freedesktop.impl.portal.ScreenCast
1. org.freedesktop.impl.portal.ScreenShot

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
1. slurp
1. wayland
1. wayland-protocols *
