# Macro pad quick start

This is for the CH552 pad (15 keys, 3 knobs that turn and press) and the
`control-surfaced` daemon. Nothing below changes Kdenlive's source code.

## 1. Install (once)

```sh
cd ~/Documents/source/control-surface
scripts/install-user.sh
```

The script:

* builds the project on the AI drive and runs the tests;
* installs `~/.local/bin/control-surfaced` and the user service;
* creates `~/.config/control-surface/config.jsonc` from the default
  configuration, or upgrades an earlier unmodified default and keeps a backup.

It does **not** enable or start the service.

## 2. Check the pad (about 1 minute)

```sh
scripts/verify-pad.sh
```

The script asks you to press each of the 24 inputs in turn: keys 1–15 (row by
row, left to right), then each knob turned left, pressed and turned right. It
shows what each input sends and saves the layout in
`~/.config/control-surface/hardware-map.json`. While it runs, only this pad is
grabbed, so its keys reach nothing else.

If a control prints nothing, it may use a key ID that is not programmed yet
(see `docs/hardware-ch552.md`, "Key-ID-first format"). To reprogram the pad,
run `ch552-padprog flash`: add `--yes` to write, and use `blank` to clear.

To see live what each control would do, without sending anything:

```sh
control-surfaced run --dry-run
```

## 3. Turn on Kdenlive's control interface

Kdenlive's control-surface interface is **off by default**. Turn it on in
Kdenlive's settings with the "Control surface interface" option
(`enableControlSurfaceInterface`).

While it is off, the pad does **nothing** in Kdenlive. The daemon types no
shortcuts. It shows one notification instead: "Kdenlive control interface not
enabled". Every Kdenlive binding is an API call.

If you want stock keyboard shortcuts while the interface is off, add
`"keyFallback": true` to the `kdenlive` profile.

To see what the running Kdenlive offers, and which of your bindings it does
not:

```sh
control-surfaced list-capabilities          # add --json for scripts
```

Kdenlive's newer build (K23-MR1a) offers about 70 actions, and every key in
the default configuration uses one of them. An older build offers only 7: play,
pause, loop zone, switch monitor, zoom fit, and zoom in/out. On an older build,
the keys for marks, edits and undo/redo do nothing, and `list-capabilities`
lists them under "not offered".

Some actions only work in the right place. If one does nothing, the log
says why:

* **Delete:** the timeline must have focus.
* **Insert and overwrite:** a clip must be open in the clip monitor, and a
  track must be targeted.
* **Slip tool:** while it is active, Kdenlive blocks play and shuttle. Press
  key 11 (the Selection tool) first. Pause always works.

## 4. Start the service

```sh
systemctl --user enable --now control-surface.service
journalctl --user -u control-surface.service -f      # its log
```

To stop it: `systemctl --user disable --now control-surface.service`.

## 5. What the controls do (default configuration)

Hold key 1 (top left) in any app to see what every key and knob does there
right now; release it to hide the overlay.

In Kdenlive, which layer applies depends on what Kdenlive reports as focused:

| Where | Knob 1 | Knob 2 | Knob 3 |
|---|---|---|---|
| Timeline | scrub/jog by frames; press play/pause | zoom; press zoom to fit | move between tracks; press switch monitor |
| Clip or project monitor | jog by frames; press play/pause | shuttle; press pause | jog by 10 frames; press switch monitor |
| Lift/Gamma/Gain wheels | lift | gamma | gain |
| Any other effect parameter | change the value; press toggles normal/fine steps | move the playhead; press play/pause | next/previous parameter |
| Track page (key 13) | pick a track | track mixer gain | selected clip's gain |
| Trim page (key 13 twice) | jog | resize the start of the selected clip(s) | resize the end |
| Anything else in Kdenlive | as on the timeline | | |

More detail on some of these:

* **Lift/Gamma/Gain wheels:** each knob edits its own wheel. Pressing a knob
  cycles that wheel's channel: value, then red, green, blue.
* **Track page:** keys 2–5 toggle mute (hide on a video track), solo, lock and
  target for the selected track.
* **Trim page:** the knobs only resize clips; they never ripple. Keys 2–5:
  cut the clip's start to the playhead, cut its end to the playhead, remove
  the gap at the playhead, and extract the selection (delete and close the
  gap).
* **Clip and project monitors:** keys 6–9 play the zone, loop it (in the clip
  monitor, loop the clip), and jump to the zone's start or end.

Keys (row 1 = keys 1–5):

| | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| Row 1 | cheatsheet (hold) | mark in | mark out | insert | overwrite |
| Row 2 | cut | delete | previous snap | next snap | add marker |
| Row 3 | select tool | razor tool | **page**: edit → track → trim | undo | redo |

With the colour wheels focused, keys 6–8 reset lift, gamma and gain. In the
effect parameter layer:

* keys 8–10 go to the previous or next keyframe, and add a keyframe;
* key 11 switches between editing the existing keyframe and creating a new
  one (Kdenlive allows creating only while stopped);
* key 12 resets the parameter.

Each knob gesture is one undo step in Kdenlive.

Other apps:

* **Brave:** keys 6–10 back, forward, reload, new tab, close tab; knob 2
  scrolls (press reopens a closed tab); knob 3 switches tabs (press: address
  bar).
* **Everything else (global profile):** keys 2–5 start Grafium, Brave,
  Copilot and Kdenlive, keys 6–8 a terminal, your files and Settings (an app
  that is not installed is skipped); keys 11–13 previous track, play/pause,
  next track; knob 1 is the volume (press mutes), knob 2 scrolls (press:
  middle click).

Turning a knob while it is pressed does nothing: the pad's firmware ignores
it, because pressing a knob moves its contacts.

## 6. Change the configuration

Simple options have a command, so you never have to edit the file for them:

```sh
control-surfaced get                      # every simple option and its value
control-surfaced set input raw            # how the pad is read: auto, evdev or raw
control-surfaced set cheatsheet.opacity 0.5
```

`set` checks the value, keeps the previous file as `config.jsonc.bak`, changes
only that value (your comments stay) and tells you when the running service
has applied it. The input modes: `auto` (the default) reads the pad raw on
firmware 2.0.2 or newer, else through its keymap; `evdev` always uses the
keymap ("compatible"); `raw` always the firmware's own events ("fastest").

What each key and knob does lives in `~/.config/control-surface/config.jsonc`:
the per-app profiles come first, the advanced settings at the end. The
service reloads the file when you save. If it has an error, the service keeps
the previous configuration and tells you why, in a notification and in the
log. To check a file:

```sh
control-surfaced check-config -c ~/.config/control-surface/config.jsonc
```

Errors name the place, for example `profile kdenlive layer trim key2: cycles
undefined mode 'x'`. Warnings point out names that this daemon does not know.

[docs/config-reference.md](docs/config-reference.md) documents every option
and binding form. In short:

* **Layers.** Each layer has a `"when"` (for example `{"focus":
  "clipMonitor"}`, `{"colorWheels": true}` or `{"$mode.page": "track"}`) and
  its own bindings. The first matching layer that binds a control wins.
* **Bindings.** A binding can be one of these:
  * a Kdenlive control: `{"control": "timeline.zoom"}`;
  * an action: `{"action": "zoom_fit"}`;
  * a command: `{"request": "track.set", "params": {…}}`;
  * a mode switch: `{"cycle": "page"}`;
  * keys, for other apps: `"ctrl+z"`;
  * a program: `{"command": ["gtk-launch", "brave-browser"]}`;
  * the overlay: `{"cheatsheet": "hold"}`.
* **Knobs.** `"turn"`, `"ccw"`/`"cw"` and `"press"`. `"scale"` multiplies
  detents. `"accel"` sets acceleration for one binding; 1 turns it off.

`control-surfaced list-capabilities` prints every context value that `"when"`
can test, with its current value.
