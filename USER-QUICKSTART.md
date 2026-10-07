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

If a control prints nothing, the pad is not sending anything. That has been
the case since programming. See `docs/hardware-ch552.md`, "Silent after
programming", for the recovery steps, and tell the coordinator what you saw.

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

Kdenlive currently offers 7 actions: play, pause, loop zone, switch monitor,
zoom fit, and zoom in/out. The default key bindings for marks, edits and
undo/redo start working when Kdenlive offers those actions. Until then,
`list-capabilities` lists them under "not offered".

## 4. Start the service

```sh
systemctl --user enable --now control-surface.service
journalctl --user -u control-surface.service -f      # its log
```

To stop it: `systemctl --user disable --now control-surface.service`.

## 5. What the controls do (default configuration)

Which layer applies depends on what Kdenlive reports as focused. "Hold + turn"
means turning a knob while it is pressed down.

| Where | Knob 1 | Knob 2 | Knob 3 |
|---|---|---|---|
| Timeline | scrub/jog by frames; hold + turn scrolls; press play/pause | zoom; press zoom to fit | move between tracks; press switch monitor |
| Clip or project monitor | jog by frames; press play/pause | shuttle; press pause | jog by 10 frames; press switch monitor |
| Lift/Gamma/Gain wheels | lift | gamma | gain |
| Any other effect parameter | change the value; hold + turn for fine steps; press toggles normal/fine | move the playhead; press play/pause | next/previous parameter |
| Track page (key 13) | pick a track | track mixer gain | selected clip's gain |
| Trim page (key 13 twice) | jog | resize the start of the selected clip(s) | resize the end |
| Anything else in Kdenlive | as on the timeline | | |

More detail on some of these:

* **Lift/Gamma/Gain wheels:** each knob edits its own wheel. Pressing a knob
  cycles that wheel's channel: value, then red, green, blue. Hold + turn
  gives fine steps.
* **Track page:** keys 1–4 toggle mute (hide on a video track), solo, lock and
  target for the selected track.
* **Trim page:** this only resizes clips; it never ripples.

Keys (row 1 = keys 1–5):

| | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| Row 1 | mark in | mark out | insert | overwrite | switch monitor |
| Row 2 | cut | delete | previous snap | next snap | add marker |
| Row 3 | select tool | razor tool | **page**: edit → track → trim | undo | redo |

With the colour wheels focused, keys 6–8 reset lift, gamma and gain. In the
effect parameter layer:

* keys 8–10 go to the previous or next keyframe, and add a keyframe;
* key 11 switches between editing the existing keyframe and creating a new
  one (Kdenlive allows creating only while stopped);
* key 12 resets the parameter.

Each knob gesture is one undo step in Kdenlive.

Other apps have their own profiles. For example, the global profile sets knob 1
to volume (press = mute) and keys 11–13 to previous track, play/pause and next
track.

## 6. Change the configuration

Edit `~/.config/control-surface/config.jsonc`. The service reloads it when you
save. If the file has an error, the service keeps the previous configuration
and tells you why, in a notification and in the log. To check a file before
saving it in place:

```sh
control-surfaced check-config -c ~/.config/control-surface/config.jsonc
```

Errors name the place, for example `profile kdenlive layer trim key1: cycles
undefined mode 'x'`. Warnings point out names that this daemon does not know.

The file's header documents every binding form. In short:

* **Layers.** Each layer has a `"when"` (for example `{"focus":
  "clipMonitor"}`, `{"colorWheels": true}` or `{"$mode.page": "track"}`) and
  its own bindings. The first matching layer that binds a control wins.
* **Bindings.** A binding can be one of these:
  * a Kdenlive control: `{"control": "timeline.zoom"}`;
  * an action: `{"action": "zoom_fit"}`;
  * a command: `{"request": "track.set", "params": {…}}`;
  * a mode switch: `{"cycle": "page"}`;
  * keys, for other apps: `"ctrl+z"`.
* **Knobs.** `"turn"`, `"ccw"`/`"cw"`, `"press"`, and `"shift": {"turn": …}`
  for hold + turn. `"scale"` multiplies detents. `"accel"` sets acceleration
  for one binding; 1 turns it off.

`control-surfaced list-capabilities` prints every context value that `"when"`
can test, with its current value.
