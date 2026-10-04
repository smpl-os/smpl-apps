# Start menu

The launcher uses `smpl_common::theme` to read the desktop palette from
`$XDG_CONFIG_HOME/eww/theme-colors.scss` (default `~/.config/eww/theme-colors.scss`).
The shared watcher applies the initial palette and changed themes every two
seconds, retaining the last good palette on read/parse errors.

`$theme-popup-opacity` controls only the window background and defaults to 1.
The application-background opacity setting does not affect popups. Text and icons remain
opaque; search/sidebar/row backgrounds use low-alpha tints rather than another
layer of full window opacity. The power and pin/unpin menus intentionally keep
95% background opacity for readability over the app list.

Headless source-contract checks for the launcher, notifications and hints:

```sh
python3 -B -m unittest discover -s start-menu/tests -p 'test_popup_theme_contract.py' -v
```

`start-menu --resident [--hidden]` keeps the menu in one long-lived process
controlled by signals; see "Start-menu resident mode" in the repository
README for the pidfile and signal contract.
