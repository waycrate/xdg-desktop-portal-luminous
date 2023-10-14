project(
  'xdg-desktop-portal-luminous',
  'rust',
  version: '0.1.2',
  meson_version : '>= 1.1.0'
)

cargo = find_program('cargo', version: '>= 1.70')

rustc = find_program('rustc', version: '>= 1.70')

if get_option('debug')
  command = [
    cargo, 'build',
    '&&',
    'cp', meson.global_source_root() / 'target' / 'debug' / meson.project_name(), '@OUTPUT@'
  ]
else
  command = [
    cargo, 'build',
    '--release',
    '&&',
    'cp', meson.global_source_root() / 'target' / 'release' / meson.project_name(), '@OUTPUT@'
  ]
endif

prefix = get_option('prefix')

xdg_install_dir = prefix / get_option('libexecdir')

portal_dir = prefix / get_option('datadir') / 'xdg-desktop-portal' / 'portals'

dbus1_dir = prefix / get_option('datadir') / 'dbus-1' / 'services'

system_dir = prefix / get_option('libdir') / 'systemd' / 'user'
global_conf = configuration_data()

global_conf.set('xdg_install_dir', xdg_install_dir)

systemd_service_file = configure_file(
  input: 'misc/xdg-desktop-portal-luminous.service.in',
  output: 'xdg-desktop-portal-luminous.service',
  configuration: global_conf
)

dbus_service_file = configure_file(
  input: 'misc/org.freedesktop.impl.portal.desktop.luminous.service.in',
  output: 'org.freedesktop.impl.portal.desktop.luminous.service',
  configuration: global_conf
)

xdg_target = custom_target('xdg-deskop-portal-luminous',
  output: 'xdg-desktop-portal-luminous',
  build_by_default: true,
  install: true,
  install_dir: xdg_install_dir,
  console: true,
  command: command
)

install_data('misc/luminous.portal',
  install_dir: portal_dir
)

install_data(meson.project_build_root() / 'org.freedesktop.impl.portal.desktop.luminous.service',
  install_dir: dbus1_dir
)

install_data(meson.project_build_root() / 'xdg-desktop-portal-luminous.service',
  install_dir: system_dir
)
