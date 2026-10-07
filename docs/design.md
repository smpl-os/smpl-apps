# Daemon design

```mermaid
flowchart LR
  pad[CH552 evdev nodes<br/>EVIOCGRAB] --> dec[Decoder<br/>chords per SYN frame]
  dec --> hw[HardwareMap<br/>chord to key7 / knob2.cw]
  hw --> eng[Engine]
  hypr[Hyprland socket2<br/>+ j/activewindow] --> eng
  eng -->|profile + layer lookup| bind{Binding}
  bind -->|keys| pace[Tap pacer] --> uinput[/dev/uinput keyboard/]
  bind -->|action / invoke / notify| kd[KdenliveDBusClient]
  bind -->|control| co[DeltaCoalescer] --> kd
  kd <-->|org.kde.kdenlive-PID<br/>/ControlSurface| kden[Kdenlive or mock]
  kden -->|ContextChanged| eng
  kden -->|ControlAck| co
```

## Resolution order

For an event on slot `knob1.turn` (or `key5`, `knob2.press`, …):

1. The focused window picks the first profile whose `match` (class/title
   regex) fits. Otherwise the `global` profile applies.
2. In that profile, the first **layer** whose `when` conditions match the
   Kdenlive context and that binds the slot wins. Then the profile's base
   bindings. If the profile has `fallthrough`, the global profile follows.
   `"none"` ends the search.
3. For turns, a `turn` binding (signed detents, best for continuous controls)
   takes precedence over `ccw`/`cw`.

`when` matches dotted paths in the context (`effect.id`, `param.name`,
`focus`, `tool`). Values are exact strings, `/regex/`, `!negation`, lists
(any of) or booleans (present/absent).

Modes (`"modes": {"liftAxis": ["luma","r","g","b"]}`) are per profile.
`{"cycle":"liftAxis"}` advances them, and `"$liftAxis"` in control options
expands to the current value. Cycling also calls `Notify` so Kdenlive shows
"Lift: r".

## Latency and flooding

* **Kdenlive controls:** the first detent goes out immediately. Afterwards at
  most one message is in flight per control+options. Further detents are summed
  until `ControlAck` arrives, or 60 ms pass, and never more often than every
  8 ms. Opposite detents cancel. Measured session-bus round trip: p50 ~110 µs,
  p99 ~180 µs. Under a 15 ms/apply busy GUI, 200 detents become fewer than 40
  messages with exact totals.
* **Acceleration** (`accelFactor` for detents closer than `accelWindowMs`)
  applies only to continuous controls and their fallbacks. Discrete bindings,
  e.g. volume keys, stay one per detent.
* **Key taps** are paced at `keyRateHz` (first tap immediate), capped at 48
  queued. Reversing a knob drops queued taps in the old direction.
* A focus change to another app drops all pending motion. It is never
  delivered to the next window.

## Safety

* Device matching: sysfs walk to the USB parent (VID, PID, serial), then
  `EVIOCGID` on the opened node before `EVIOCGRAB`. The uinput keyboard has its
  own IDs (`1d6b:0cf1`, `BUS_VIRTUAL`), so the daemon can never grab it.
* The virtual keyboard does not advertise power, sleep, wakeup, suspend,
  rfkill, battery or coffee keys, so the kernel drops those codes even if a
  config names them.
* SIGINT/SIGTERM/SIGHUP quit the event loop, so destructors release the grab
  and destroy the uinput device. The kernel also releases a grab when the
  process dies.
* The systemd unit is deliberately unsandboxed. Commands launched by bindings
  are ordinary desktop programs and would inherit restrictions.

## Window backends

* `HyprlandTracker`: `activewindow`/`activewindowv2`/`closewindow`/
  `windowtitlev2` events. Each focus change is followed by one coalesced
  `j/activewindow` query to get the PID. Provisional events carry no stale PID.
  It reconnects every 2 s and picks the newest instance directory if Hyprland
  restarted with a new signature.
* `StaticWindowTracker`: tests, `simulate`, `--force-window`.
* KWin (planned): a tiny KWin script that reports `workspace.activeWindow`
  (resourceClass, caption, pid) to the daemon over D-Bus. The factory switch is
  `createWindowTracker()`.

## FL Studio (future)

The profile exists with key bindings. The planned improvement is a MIDI output
sink (ALSA sequencer port) so knobs can use FL Studio's "Link to controller"
under Wine instead of keystrokes. It would be a new `Binding` kind beside
`keys`/`control`.
