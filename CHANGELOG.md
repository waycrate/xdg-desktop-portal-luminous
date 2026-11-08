# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.22] - 2026-11-08

### Fixed

- fix: pipewire thread loop by [Gigas002](https://github.com/Gigas002) in [#246](https://github.com/waycrate/xdg-desktop-portal-luminous/pull/246)
- Fix Deskflow RemoteDesktop integration on Wayland in by [brendensuarez](https://github.com/brendensuarez) [#237](https://github.com/waycrate/xdg-desktop-portal-luminous/pull/237)

### Changed
- update iced_exwlshell to 0.21.3
- Sync to zbus6
- feat: implement org.freedesktop.portal.Usb by [MuntasirSZN](https://github.com/MuntasirSZN) in  [#226](https://github.com/waycrate/xdg-desktop-portal-luminous/pull/226) 
- Send EIS Keymap by [Nuhiat-Arefin](https://github.com/Nuhiat-Arefin) in [#223](https://github.com/waycrate/xdg-desktop-portal-luminous/pull/223)

[0.1.22]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.22-v0.1.21

## [0.1.21] - 2026-08-01

### Fixed

- fix memory leak from libwayshot
- fix RemoteDesktop scroll events by [@spotdemo4](https://github.com/spotdemo4)
- fix kde connect cannot release button


### Changed

- implement the background portal by [@Nuhiat-Arefin](https://github.com/Nuhiat-Arefin)
- add an simple clipboard implement
- bump deps of pipewire
- allow config under /etc/xdg/xdg-desktop-portal-luminous

## [0.1.20] - 2026-06-06

### Changed
- Fix the problem that kde cannot release button. This will make remove touch device will be usable during remoting. For example, remote control from android device.

[0.1.20]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.20-v0.1.19

## [0.1.19] - 2026-05-28

### Changed
- Bump libwayshot, pipewire and iced_layershell
- Fix RemoteDesktop scroll events by @spotdemo4

[0.1.19]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.19-v0.1.18

## [0.1.18] - 2026-03-20

### Changed
- Fix xdg-desktop-portal-luminous did not serlalize and deserialize the sourceType in the right way

[0.1.18]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.18-v0.1.17

## [0.1.17] - 2026-03-11

### Changed
- Fix xdg-desktop-portal-luminous cannot be stopped after wayland dead

[0.1.17]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.17-v0.1.15

## [0.1.16] - 2026-03-10

### Changed
- Upgrade the deps
- bump reis

[0.1.16]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.16-v0.1.15

## [0.1.15] - 2026-03-10

### Changed
- Fix the problem xdg-desktop-portal-luminous cannot start screencast on niri
- Support org.freedesktop.impl.portal.InputCapture
- bump deps for libwayshot

[0.1.15]: https://github.com/waycrate/xdg-desktop-portal-luminous/compare/v0.1.15-v0.1.14
