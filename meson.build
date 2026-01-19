project(
  'xdg-desktop-portal-luminous',
  'rust',
  version: '0.1.14',
  meson_version: '>= 1.1.0',
)

cargo = find_program('cargo', version: '>= 1.85')

helper_name = 'lumious_headless_helper'

find_program('rustc', version: '>= 1.85')

if get_option('debug')
  command_main = [
    cargo,
    'build',
    '&&',
    'cp',
    meson.global_source_root() / 'target' / 'debug' / meson.project_name(),
    '@OUTPUT@',
  ]
  command_helper = [
    cargo,
    'build',
    '--package', 'lumious_headless_helper',
    '&&',
    'cp',
    meson.global_source_root() / 'target' / 'debug' / helper_name,
    '@OUTPUT@',
  ]
else
  command_main = [
    cargo,
    'build',
    '--release', '&&',
    'cp',
    meson.global_source_root() / 'target' / 'debug' / meson.project_name(),
    '@OUTPUT@',
  ]
  command_helper = [
    cargo,
    'build',
    '--package', 'lumious_headless_helper',
    '--release', '&&',
    'cp',
    meson.global_source_root() / 'target' / 'release' / helper_name,
    '@OUTPUT@',
  ]
endif

prefix = get_option('prefix')

bindir = prefix / get_option('bindir')

xdg_install_dir = prefix / get_option('libexecdir')

portal_dir = prefix / get_option('datadir') / 'xdg-desktop-portal' / 'portals'

dbus1_dir = prefix / get_option('datadir') / 'dbus-1' / 'services'

system_dir = prefix / get_option('libdir') / 'systemd' / 'user'
global_conf = configuration_data()

global_conf.set('xdg_install_dir', xdg_install_dir)

configure_file(
  input: 'misc/xdg-desktop-portal-luminous.service.in',
  output: 'xdg-desktop-portal-luminous.service',
  configuration: global_conf,
)

configure_file(
  input: 'misc/org.freedesktop.impl.portal.desktop.luminous.service.in',
  output: 'org.freedesktop.impl.portal.desktop.luminous.service',
  configuration: global_conf,
)

custom_target(
  'xdg-desktop-portal-luminous',
  output: 'xdg-desktop-portal-luminous',
  build_by_default: true,
  install: true,
  install_dir: xdg_install_dir,
  console: true,
  command: command_main,
)
custom_target(
  'lumious_headless_helper',
  output: helper_name,
  build_by_default: true,
  install: true,
  install_dir: bindir,
  console: true,
  command: command_helper,
)

install_data('misc/luminous.portal', install_dir: portal_dir)

install_data(
  meson.project_build_root() / 'org.freedesktop.impl.portal.desktop.luminous.service',
  install_dir: dbus1_dir,
)

install_data(
  meson.project_build_root() / 'xdg-desktop-portal-luminous.service',
  install_dir: system_dir,
)
