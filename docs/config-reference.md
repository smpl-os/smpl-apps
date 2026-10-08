# control-surface configuration reference

The daemon reads `~/.config/control-surface/config.jsonc`: JSON that may carry
`//` and `/* */` comments and trailing commas. Saved changes apply at once;
a file that does not parse or validate is reported (log, `status`, the
Settings app) and the last good configuration stays in use.

You rarely need to edit it by hand:

| Want to | Command |
|---|---|
| change a simple option | `control-surfaced set KEY VALUE` (for example `set input raw`) |
| see the simple options and their values | `control-surfaced get` (one: `get input`; `--json` for scripts) |
| check a file | `control-surfaced check-config [-c FILE]` |
| see what each input does in an app | `control-surfaced cheatsheet --window brave-browser` |
| see what Kdenlive offers | `control-surfaced list-capabilities` |
| watch what the pad sends | `control-surfaced monitor` |

The Settings app (smplOS Keypad tab) uses the same operations over D-Bus
(`SetOption`, `GetOption`, `ListOptions`, `GetConfig`/`SetConfig`; see
[dbus-settings-api.md](dbus-settings-api.md)).

## Simple options (`set` / `get`)

`set` validates the value and the whole resulting config, keeps a backup of
the previous file (`config.jsonc.bak`), writes atomically, and changes only
that value in the file: comments and layout stay as they are. A missing
option is added in its section. If the file does not exist it starts from
the shipped example. When the daemon is running, `set` waits until the daemon
has loaded the change and says so.

| Key | Values | Default | What |
|---|---|---|---|
| `input` | `auto`, `evdev` (alias `keymap`), `raw` | `auto` | How the daemon reads the pad. `auto`: raw on the control-surface firmware 2.0.2+, evdev otherwise. `evdev` (Settings: "Keymap (compatible)"): the keys the pad types from its keymap; real press and release. `raw` (Settings: "Raw (fastest)"): the firmware's own numbered events with snapshots; needs firmware 2.0.2+ (older firmware falls back to evdev). |
| `serial` | text | `""` | Drive only the pad with this USB serial; empty drives the first 1189:8890 pad found. |
| `cheatsheet.opacity` | 0.05 – 1 | 0.35 | The overlay's opacity. |
| `cheatsheet.autoHideMs` | 0 – 600000 | 8000 | Hide the overlay after this long without pad input; 0 = until hidden. A `hold` key keeps it up while held; clicking it hides it. |
| `cheatsheet.position` | `center`, `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left`, `bottom-right` | `center` | Where the overlay sits. |
| `cheatsheet.eww` | `true`, `false` | unset | Push the overlay into eww; unset: `run --eww-window/--eww-config` decide. |
| `settings.accelFactor` | 1 – 10 | 1 | How much further fast knob detents move (1 = never accelerate). |
| `settings.accelWindowMs` | 5 – 200 | 40 | Detents closer than this count as fast. |
| `settings.keyRateHz` | 10 – 1000 | 120 | Pace of key taps produced by knob detents. |

Exit codes: 0 done (or already so), 2 unknown option, bad value or a config
that would not validate (nothing written), 3 the file could not be written.
`control-surfaced features --json` lists the same table under `options`.

## Layout of the file

```jsonc
{
    "profiles": [ ... ],          // what the keys and knobs do, per app

    // ---- Advanced ----
    "device":     { "serial": "", "input": "auto" },
    "cheatsheet": { "opacity": 0.35, "position": "center" },
    "settings":   { ... },
    "hardware":   "~/.config/control-surface/hardware-map.json",
    "layout":     "sy181-15k3e"     // optional, see "Layout" below
}
```

## Inputs

* Keys: `key1`..`key15`, row by row from the top left (row 1 = key1..key5).
* Knobs: `knob1`..`knob3`, from the top. Each knob has:
  * `"turn"`: signed detents; preferred for continuous controls;
  * or `"ccw"` / `"cw"`: one binding per direction;
  * `"press"`;
  * `"shift"` with its own `"turn"`/`"ccw"`/`"cw"`: turning while the knob is
    held down. A knob with shift bindings fires its `"press"` on release, and
    only if it did not turn. **Not on this pad:** pressing the top or middle
    knob holds one of its encoder lines low, so a turn while it is pressed
    has no direction on any firmware, and the control-surface firmware ignores
    turns while any knob is pressed ([hardware-ch552.md](hardware-ch552.md)).
    `check-config` warns about shift bindings here, and `features --json` says
    so (`slots.shiftSupported: false`, the board's `turnsWhilePressed` and
    `pressPinsEncoder`). Use a [held-key layer](#held-key-layers) instead.

## Profiles

```jsonc
{
    "name": "brave",
    "match": { "class": "^brave-browser$", "title": "optional regex" },
    "bindings": { "key2": "alt+left", "knob2": { "ccw": ..., "cw": ..., "press": ... } }
}
```

* `match`: regular expressions on the focused window's class (Hyprland
  `activewindow`) and, optionally, its title. The first matching profile is
  used.
* A profile without `match` is the global fallback (`"name": "global"`):
  slots a matching profile leaves unbound fall through to it, unless that
  profile has `"fallthrough": false`.
* `"kdenlive": true`: the profile talks to Kdenlive (see below);
  `"keyFallback"`, `"modes"` and `"layers"` are mostly used there.
* `"modes"`: named values, e.g. `"page": ["edit", "track", "trim"]`. Each
  starts at its first value. `{"cycle": "page"}` advances one value,
  `{"cycle": "page", "step": -1}` goes back, and
  `{"mode": "page", "set": "trim"}` sets one directly. `"$page"` in options
  expands to the current value, and `"when": {"$mode.page": "trim"}` tests
  it. Changing a mode tells Kdenlive the new value (`{label}: {value}`):
  * the object form gives a mode its own text,
    `"workspace": {"values": ["", "Edit"], "notify": "Workspace: {value|Main}"}`
    (`{value|Main}` reads `Main` while the value is empty);
  * a binding's `"notify"` overrides it, and `"notify": false` says
    nothing. Placeholders: `{label}`, `{mode}`, `{value}`, `{value|text}`.
* `"autoModes"`: modes that follow Kdenlive's context (see below).

## Binding forms

| Form | What |
|---|---|
| `"ctrl+z"` | tap keys through the virtual keyboard |
| `{"keys": ["ctrl+k", "x"]}` | a key sequence |
| `{"action": "mark_in", "fallback": "i"}` | a Kdenlive action by its KActionCollection id (see `list-capabilities`); `fallback` keys are typed only in a profile with `"keyFallback": true` and only while Kdenlive's interface is absent or off |
| `{"control": "playhead.jog", "scale": 1, "accel": 3, "options": {...}, "fallback": ["left", "right"]}` | a continuous control (coalesced); `scale` multiplies detents and may be negative (the other way: on keys, a `+R`/`-R` pair is `"scale": 1` and `"scale": -1`), `accel` overrides `settings.accelFactor` for this binding (1 = never accelerate), `fallback` = [negative key, positive key] |
| `{"cycle": "liftAxis", "label": "Lift"}` | advance a mode (`"step": -1` goes back); `"$liftAxis"` in options expands, `"$ctx:tool"` expands to a value from Kdenlive's context and `"$!ctx:path"` to its negation (toggles); a missing context value sends nothing; `"control": "$mode"` lets a mode choose the control a knob drives |
| `{"mode": "workspace", "set": "Edit"}` | set a mode to one of its values (`"value"` works too); the overlay shows the key as `on` while the mode has that value |
| `{"do": [{"mode": "workspace", "set": "Color"}, {"action": "load_layout5"}], "delayMs": 50}` | several bindings in order (a "multi-action"): each step acts as if it were bound itself. `delayMs` (0–5000) waits before each later step; steps still waiting are dropped when the focused window changes. No nested `"do"`, `"none"` or cheatsheet `"hold"` steps. The overlay greys it out when any step cannot run |
| `{"request": "colorwheel.reset", "params": {"wheel": "lift"}}` | `Invoke()` on Kdenlive |
| `{"mouse": "left"}` | `left`, `right`, `middle`, `back`, `forward`, or one wheel detent per knob detent: `wheel-up`, `wheel-down`, `wheel-left`, `wheel-right` |
| `"volumeup"`, `"playpause"`, `"mute"` ... | media keys (any name from `features --json` `keyNames`, in any case) |
| `{"command": ["gtk-launch", "org.kde.kdenlive"]}` | start a program (argv, no shell; `~` and `~/...` are expanded) |
| `{"cheatsheet": "hold"}` | show an overlay of what every key and knob does right now (profile, Kdenlive layer, modes) while held; `"toggle"` shows or hides it. Keys and knob presses only |
| `"none"` | explicitly unbound (stops the fall-through) |

Any binding object may also carry:

* `"label": "Back"`: the cheatsheet's text (default: made from what it does);
* `"icon": "player-play"`: the cheatsheet's icon, a Tabler outline icon name
  (default: chosen from what it does; `"none"` for no icon);
* `"ifInstalled": "grafium"`: a program or desktop id (or a list); while one
  is missing the binding is skipped and the slot falls through.

## Kdenlive

Kdenlive is driven only through its control-surface interface (D-Bus
`ControlSurface1`), which is off by default in Kdenlive: enable it in
Kdenlive's settings (`enableControlSurfaceInterface`). While it is off the
daemon types nothing into Kdenlive and shows one notice instead. Set
`"keyFallback": true` on the profile to type the stock shortcuts instead.
Only what the running Kdenlive advertises is used; a refusal never types keys.

Actions are Kdenlive's curated list (see `list-capabilities`; offline:
`list-actions`). K23 MR1b-A adds clip, effect, multicam, sequence, view and
audio actions and four families: `activate_video_1`..`9` (camera N, only in
the Multicam tool, and only cameras that exist), `load_layout1`..`9`
(registered layout slots), `tag_<n>` (the project's bin tags, exactly one bin
clip selected) and `effect_<id>` (add an installed effect, e.g.
`effect_avfilter.gblur`, to the selected clip). Some depend on
context: delete needs timeline focus, insert/overwrite need a clip in the
clip monitor and a target track, and the Slip tool's preview blocks playback
and shuttle until you switch back to the Selection tool.

### Modes that follow Kdenlive (autoModes)

```jsonc
"autoModes": [
    { "name": "wheels", "when": { "colorWheels": true }, "set": { "workspace": "Color" }, "restore": true },
    { "name": "bin", "when": { "focus": "bin" }, "set": { "workspace": "Media" }, "notify": false }
]
```

A rule sets its modes when its `"when"` (Kdenlive's context and modes, as
for layers; not `"held"`) starts to match, once. It is edge triggered:
changing the mode by hand afterwards sticks. With `"restore": true` the
modes go back to what they were when the condition stops matching, unless you
changed them meanwhile. `"notify": false` sets them silently; by default the
mode's text is sent. Rules apply in the focused profile only, and only while
Kdenlive answers. Make a rule optional with a mode condition, e.g.
`"$mode.auto": "on"` plus a key that cycles `auto`.

### Held-key layers

Hold a key and the other keys and knobs do something else while it is down,
in any app:

```jsonc
"layers": [
    {
        "name": "Workspaces",
        "when": { "held": "key1" },
        "bindings": {
            "knob1": { "ccw": { "command": ["hyprctl", "dispatch", "workspace", "e-1"], "label": "Previous workspace" },
                       "cw":  { "command": ["hyprctl", "dispatch", "workspace", "e+1"], "label": "Next workspace" } },
            "key2": { "command": ["hyprctl", "dispatch", "workspace", "1"], "label": "Workspace 1" }
        }
    }
]
```

* `"held"` takes one control (`"key1"`), a list of alternatives
  (`["key13", "key14"]`: either one held), or controls held together
  (`"key1+knob3"`). Controls are `key1`..`key16`, or `knob1`..`knob3`
  for a knob press. Other `"when"` conditions can be added; all must hold.
* **Precedence:** `"held"` is a condition like any other. The first matching
  layer in list order wins, so list a held layer before the layers it should
  override. A held layer listed after layers that bind the same input applies
  only where they do not, for example only in a "hub" mode. A held layer in
  the global profile covers what the app profile leaves unbound.
* **The held key's own binding:**
  * `{"cheatsheet": "hold"}` shows the overlay at once, and the overlay shows
    the held layer, so holding key 1 shows what the knobs do while it is
    held.
  * Any other binding is a tap. It fires when the key is released, and only
    if no other key or knob was used while it was down. A key that is
    only a modifier needs no binding of its own.
* Releasing the key ends the layer. A pad that disappears while a key is
  held (unplugged, raw session lost) ends it too, and the deferred tap does
  not fire.
* In the cheatsheet (`GetCheatsheet`), `"held"` lists the held-layer keys
  that are down. Previews take `"$held"` in their context JSON
  (`GetCheatsheetFor`), and the CLI takes
  `control-surfaced cheatsheet --window brave-browser --held key1`.
* **What this pad can hold together:** key 1 has its own pin. Keys 2–15
  and the knob presses are read one at a time: pressing key 5 while key 13
  is held reports key 13 released, and it reappears when key 5 comes up. So
  a held layer works with:
  * key 1 held plus any other key, knob press or knob turn;
  * any key or knob press held plus the knob turns (and key 1).

  A layer held on key 13 that binds key 5 never fires, and
  `"key13+knob3"` can never be held. `check-config` warns about both. The
  daemon recognises the pad's re-report of the held key (within 40 ms), so
  that roll never fires the held key's own tap.
* **Input modes:** raw mode (the default `"auto"` on firmware 2.0.2+)
  handles every combination above. With `"input": "evdev"`, the held key's
  keymap chord stays in the pad's report:
  * an input whose chord shares the held key's F-key is lost, and the held
    key reads as released (key 1 = F14 and knob2 ccw = Alt+F14);
  * an input pressed while a modified chord is held can read as another
    control (knob 3's press is Alt+F18, so key 1 pressed meanwhile reads as
    Alt+F14 = knob2 ccw).

  `check-config` lists both kinds.

### Layers

```jsonc
"layers": [
    { "name": "color-wheels", "when": { "colorWheels": true }, "bindings": { ... } },
    { "name": "timeline", "when": { "focus": "timeline" }, "bindings": { ... } }
]
```

The first layer whose `"when"` matches (held keys included) and that binds a
slot wins, then the profile's own bindings, then the global profile. `"when"` tests Kdenlive's
context (`"focus"`, `"colorWheels"`, `"param.target"`,
`"timeline.track.audio"` ...) or a mode (`"$mode.page"`). Values may be
`"/regex/"`, `"!value"`, a list of alternatives, or a boolean. `"focus"` is
`timeline`, `clipMonitor`, `projectMonitor`, `effectStack`, `bin` (K23
MR1b-A) or `other`. Editing
controls target the focused item from Kdenlive's context;
`"targetFrom": "hoveredColorWheel.target"` aims at the wheel under the mouse
instead.

## Advanced

### device

`{"serial": "", "input": "auto"}`: see `serial` and `input` above. The
daemon only ever grabs a 1189:8890 pad: `"vendor"`/`"product"` may be
stated, but any other id is refused (it would grab another device).

### cheatsheet

`opacity`, `autoHideMs`, `position` as above, and `eww`: `true`, `false` or
`{"window": "pad-cheatsheet", "config": "~/.config/eww", "variable":
"pad_sheet"}`. With it the daemon pushes the overlay into eww itself
(`eww update pad_sheet=<json>`, `eww open WINDOW --anchor ...`,
`eww close WINDOW`). Unset, `run --eww-window NAME --eww-config DIR`
(smplOS's unit) decides; set here, it overrides those flags field by field.

### settings

| Key | Default | What |
|---|---|---|
| `coalesceMs` | 8 | at most one continuous update per control this often |
| `ackTimeoutMs` | 60 | and only one in flight until Kdenlive acknowledges it, or this long |
| `accelWindowMs` | 40 | detents closer than this count as fast |
| `accelFactor` | 1 | fast detents move this many times as far (per binding: `"accel"`) |
| `keyRateHz` | 120 | pacing of key taps produced by knobs |
| `gestureIdleMs` | 500 | an editing gesture (one undo step in Kdenlive) ends after this idle time |

### hardware and layout

`"hardware"` names the hardware map written by `control-surfaced verify`;
until it exists the firmware's (or the default) slot numbering applies.
`"layout"` overrides the board: a built-in id (`features --json`
`layouts.builtin`) or `{"keys": 1..16, "knobs": 0..3, "columns": n}`.
Precedence: the config's `layout`, then the firmware's board, then the
hardware map, then the default. A layout that does not match the firmware
is reported as a `layout:` warning.
