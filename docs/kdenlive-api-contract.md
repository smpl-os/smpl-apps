# Kdenlive control-surface API contract (proposal)

Status: **proposal for review**. It is not implemented in Kdenlive. A mock
(`src/mock/`) and a client (`src/daemon/kdenlivedbusclient.*`) in this
repository implement it, and their tests pass. Authors: the control-surface
project for the user's CH552 macro pad. Kdenlive source owner: the MAIN session
(`.automation-curves`); nothing here edits Kdenlive.

Interface: `org.kde.kdenlive.ControlSurface1` on object `/ControlSurface` of the
service Kdenlive already owns, `org.kde.kdenlive-<pid>` (session bus).

Suggested ledger row: **K23 — generic control-surface D-Bus interface**. Submit
it as its own KDE merge request. It does not depend on MCP, Python, the plugin
framework (K22) or the automation work (K02/K03). The only exception is the
optional `automation.nudge` control in §5.4.

---

## 1. Why a new interface

The pad has 15 keys and 3 rotary knobs that turn (with detents) and press. A
user-session daemon in this repository grabs only that device. It follows the
focused window over Hyprland IPC and applies per-application profiles. For
plain applications it types keys through uinput. Kdenlive deserves more than
faked keystrokes:

* **Keystrokes are context-blind.** The same key does different things
  depending on the focused widget. Shortcuts can be remapped. A knob mapped to
  Left/Right cannot know whether a colour wheel or the timeline is focused.
* **Continuous controls need rates keystrokes cannot express.** Examples: jog
  by N frames, shuttle speed, zoom, nudging a parameter by a step, and moving a
  lift/gamma/gain wheel along one axis.
* **One undo per gesture.** A knob turn of 40 detents on a parameter should be
  one undo step, not 40.

### 1.1 What exists today (read-only survey of `.automation-curves` and `src`)

| Surface | Where | What it offers a control surface | Why it is not enough |
|---|---|---|---|
| Built-in JogShuttle | `src/jogshuttle/jogshuttle.cpp:37-148`, `jogmanager.cpp:46-57`, `jogaction.cpp:16-52` | A reader thread on a `media_ctrl` device posts events to the GUI thread. Buttons map to **KAction names** via `actionCollection()->action(name)->trigger()`. Jog calls `MonitorManager::slotForwardOneFrame/slotRewindOneFrame`. Shuttle uses the speed table `0,1,2,4,5,8,16,60`. | Only Contour ShuttlePro-class devices through `media_ctrl`. No focus context, no parameter control, no out-of-process use. This is the **prior art to build on**, and its action-by-name model is reused below. |
| Rendering D-Bus | `src/org.kdenlive.MainWindow.xml`, `src/render/renderserver.cpp:19` | Service `org.kde.kdenlive-<pid>` exists in stock builds (`KDBusService` default `Multiple`, `src/main.cpp:564`). | Render progress only. |
| `/Automation` adaptor (ours) | `src/plugins/plugindbusadaptor.h:22-73`, `.cpp:119-132` | Project info, tracks, bin, markers, selection, effect add, guides, plugin jobs | Curated data edits. No transport, focus or UI state. Not upstream. |
| `/Editor` `request(ss)` + MCP (ours) | `data/mcp/kdenlive_mcp.py:603-687`. **107 tools** (`--list-tools`) | Exact, guarded edits: `get_timeline`, `set_timeline_selection`, `add_effect`, `set_effect_parameters`, automation `discover/get/apply`, `create_guide`, `history_undo/redo`, `render_project`, loop/reverse tools, … | Designed for agents. Every mutation takes `expected_project_id`/`expected_undo_index` guards, is synchronous (30 s timeout) and is one undo per call. Nothing covers playback, seek, zoom, focus or the active monitor, nor arbitrary actions. Calling it 100× per second would also break the undo history. |
| KXmlGui actions | `mainwindow.cpp`, `monitor/monitormanager.cpp:664-812`, `timeline2/view/*` | About 250 named actions (`monitor_play`, `mark_in`, `insert_to_in_point`, `cut_timeline_clip`, `ripple_tool`, `keyframe_add`, `zoom_fit`, `edit_undo` (KStandardAction), …) | No D-Bus access, so only keystrokes reach them, and they carry no context. |

**Conclusion:** add one small interface. It exposes UI context, invokes actions
by name and accepts coalesced continuous controls. It reuses what Kdenlive
already has (`KActionCollection`, `MonitorManager`, `TimelineController`,
`AssetParameterModel`) and adds no editing engine.

---

## 2. (a) What editors actually do: inventory

These are the operations a hardware surface should reach, grouped the way
editors think about them. *Input* is the natural pad gesture.

| # | Area | Operation | Input |
|---|---|---|---|
| N1 | Navigation | Play/pause, stop | key / knob press |
| N2 | | J/K/L shuttle: reverse/forward speed steps (1,2,4,5,8,16,60×) | knob turn |
| N3 | | Jog by frame / by second | knob turn |
| N4 | | Scrub (jog with audio) | knob turn |
| N5 | | Go to start/end, clip start/end, zone in/out | key |
| N6 | | Previous/next snap point (edit point) | key / knob |
| N7 | | Previous/next marker or guide | key |
| N8 | | Loop zone / Loop Between Markers (K01) | key |
| M1 | Marking | Set in / out, clear in/out | key |
| M2 | | Add marker/guide quickly, edit, delete, by category | key |
| T1 | Three-point | Insert / overwrite zone to in-point, lift, extract | key |
| T2 | | Source/target track selection, switch active target | key / knob |
| E1 | Cutting | Razor at playhead (cut clip), cut all tracks | key |
| E2 | | Delete selected (lift), ripple delete | key |
| E3 | Trimming | Select tool / razor / spacer / ripple / roll / slip / slide | key |
| E4 | | Trim the selected edit point by ±N frames under the active tool | knob turn |
| E5 | | Resize clip start/end to playhead | key |
| S1 | Selection | Select clip under playhead, next/previous clip, deselect | key / knob |
| K1 | Tracks | Active track up/down | knob turn |
| K2 | | Mute / solo / lock / hide track, target toggle | key |
| F1 | Keyframes | Add/remove keyframe at playhead, previous/next keyframe | key |
| F2 | | Nudge keyframe value / time | knob |
| P1 | Effects | Focus next/previous parameter of the focused effect | knob |
| P2 | | Nudge the focused parameter (coarse/fine), reset parameter | knob / key |
| P3 | | Enable/disable effect, compare (split) | key |
| C1 | Colour | Lift/Gamma/Gain wheels: luma, R, G, B (or hue/sat) per wheel | 3 knobs + press to cycle the axis |
| C2 | | Reset a wheel | key |
| C3 | | Curves / white balance points | knob |
| A1 | Audio | Clip gain / track volume in 0.1 dB steps | knob |
| A2 | | Mixer fader of the active track | knob |
| V1 | View | Timeline zoom in/out/fit, horizontal scroll | knob |
| V2 | | Switch clip/project monitor, monitor zoom | key |
| U1 | Automation (K02/K03) | Select previous/next point, nudge value/time, add point | knob / key |
| L1 | Loop/reverse tools | Loop Between Markers (K01), reverse range (K21), seamless loop (K12) | key |
| R1 | Project | Undo/redo, save, render dialog, render zone | key |

## 3. (b) Coverage matrix

Legend: ✅ available, ◐ partial or unsuitable for interactive use, ❌ missing.
"Action" means a stock KAction id exists. Hardware can reach it only through
faked shortcuts today, or through `TriggerAction` once this proposal lands.

| # | Stock KAction (id) | Our `/Editor`/MCP | Continuous and low latency today | Covered by this proposal |
|---|---|---|---|---|
| N1 | ✅ `monitor_play`, `monitor_pause` | ❌ | ❌ | `TriggerAction` |
| N2 | ✅ `monitor_seek_backward`/`forward` (J/L), JogShuttle internals | ❌ | ❌ | `playhead.shuttle` |
| N3 | ✅ `monitor_seek_*-one-frame`/`-one-second` | ❌ (`get_timeline_frame` renders, does not seek) | ❌ | `playhead.jog` |
| N4 | ◐ `mlt_scrub` toggles audio scrubbing | ❌ | ❌ | `playhead.jog` + `{"scrub":true}` |
| N5 | ✅ `seek_start`, `seek_end`, `seek_clip_start/end`, `seek_zone_start/end` | ❌ | – | `TriggerAction` |
| N6 | ✅ `monitor_seek_snap_backward/forward` | ❌ | – | `TriggerAction` |
| N7 | ✅ `monitor_seek_guide_backward/forward` | ◐ `get_guides`/`list_markers` read only | – | `TriggerAction` |
| N8 | ✅ `monitor_loop_zone`; K01 action (local) | ❌ | – | `TriggerAction` |
| M1 | ✅ `mark_in`, `mark_out` | ❌ | – | `TriggerAction` |
| M2 | ✅ `add_marker_guide_quickly`, `edit_*_marker`, `delete_*_marker` | ◐ `create_guide`, `add_guide` (guarded, no category UI) | – | `TriggerAction` |
| T1 | ✅ `insert_to_in_point`, `overwrite_to_in_point`, `extract_zone`, `remove_lift`, `remove_extract` | ◐ `insert_clip_from_bin` (legacy) | – | `TriggerAction` |
| T2 | ✅ `switch_track_target`, `switch_active_target`, `activate_all_targets` | ❌ | – | `TriggerAction`, `timeline.track` |
| E1 | ✅ `cut_timeline_clip`, `cut_timeline_all_clips` | ◐ `split_linked_clip` (explicit clip/frame) | – | `TriggerAction` |
| E2 | ✅ `delete_timeline_clip` | ◐ `delete_clips` (lift, explicit ids) | – | `TriggerAction` |
| E3 | ✅ `select_tool`, `razor_tool`, `spacer_tool`, `ripple_tool`, `roll_tool`, `slip_tool`, `slide_tool` | ❌ | – | `TriggerAction`; `tool` in context |
| E4 | ❌ (mouse-only drag in QML) | ❌ | ❌ | `edit.trim` |
| E5 | ✅ `resize_timeline_clip_start/end` | ❌ | – | `TriggerAction` |
| S1 | ✅ `select_timeline_clip`, `deselect_timeline_clip` | ◐ `set_timeline_selection` (explicit ids) | – | `TriggerAction`; selection in context |
| K1 | ◐ no action; QML only | ❌ | ❌ | `timeline.track` |
| K2 | ◐ `switch_track_lock`, `switch_track_disabled`; mute/solo only as track-header buttons (*) | ◐ `set_clip_audio_muted` (clips, not tracks) | – | `TriggerAction`, `Invoke("track.toggle", …)` |
| F1 | ✅ `keyframe_add`, `keyframe_next`, `keyframe_previous` | ◐ automation `apply_automation` | – | `TriggerAction` |
| F2 | ❌ | ◐ `apply_automation` (one guarded undo per call) | ❌ | `param.nudge`, `automation.nudge` |
| P1 | ❌ (Tab only, widget-dependent) | ❌ | ❌ | `param.focus` |
| P2 | ❌ | ◐ `set_effect_parameters` (absolute, one undo per call, needs UUIDs) | ❌ | `param.nudge`, `Invoke("param.reset")` |
| P3 | ✅ `disable_timeline_effects`, effect header toggles | ❌ | – | `Invoke("effect.toggle")` |
| C1 | ❌ (mouse on `ColorWheel`, `assets/view/widgets/lumaliftgainparam.cpp:26-90`) | ◐ `set_effect_parameters` on `lift_r…gain_b` | ❌ | `colorwheel.nudge` |
| C2 | ❌ | ◐ same | – | `Invoke("colorwheel.reset")` |
| C3 | ❌ | ◐ same | ❌ | `param.nudge` (focused point) – follow-up |
| A1 | ❌ | ◐ effect parameters | ❌ | `audio.gain` |
| A2 | ❌ (mixer widget) | ❌ | ❌ | `audio.gain` with `{"target":"track"}` |
| V1 | ✅ `view_zoom_in/out` (KStandardAction), `zoom_fit` | ❌ | ◐ discrete only | `timeline.zoom`, `timeline.scroll` |
| V2 | ✅ `switch_monitor`, `monitor_zoomin/out` | ❌ | – | `TriggerAction`; `activeMonitor` in context |
| U1 | ✅ `automation_editor` (local) | ✅ `discover/get/apply_automation` (data level) | ❌ | `automation.nudge` (+ selection in context) |
| L1 | ◐ K01 action (local), K21/K12 as plugins | ✅ `reverse_video_range`, `create_seamless_loop`, `smooth_loop_boundary` | – | `TriggerAction` (K01). Long jobs stay with MCP. |
| R1 | ✅ `edit_undo`, `edit_redo`, `file_save`, `project_render` | ✅ `history_undo/redo`, `save_project`, `render_project` | – | `TriggerAction` |
| – | **UI context** (focus, monitor, tool, focused effect/param) | ❌ anywhere | ❌ | `GetContext`, `Subscribe`, `ContextChanged` |

(*) `mlt_mute` (`monitor/monitormanager.cpp:796`) mutes the **monitor**, not a track. No track mute/solo action id was found; the header buttons are QML-only.

---

## 4. (c) The interface

### 4.1 Principles

1. **Small and generic.** About 10 methods and 3 signals. Kdenlive concepts
   only; no AI, MCP, plugin or vendor specifics.
2. **Free when unused.** Context tracking and signals run only while at least
   one client is subscribed. Subscribers are tracked by unique bus name and
   dropped when they leave the bus.
3. **Never blocks the GUI.** Methods run on the GUI thread, where QtDBus
   delivers them, and never start nested event loops. Continuous controls are
   fire-and-forget (`NoReply`) and are applied once per event-loop pass.
4. **Existing code paths only.** Actions go through `KActionCollection`
   (exactly like keyboard shortcuts and JogShuttle). Transport goes through
   `MonitorManager`/`Monitor`. Zoom uses `TimelineController::setScaleFactor`.
   Parameters use the same `AssetParameterModel` paths and undo commands as the
   widgets.
5. **Versioned and discoverable.** The `1` suffix in the interface name.
   Additions are additive. `Capabilities()` lists controls, commands and context
   keys. Clients must ignore unknown keys.
6. **Safe by default.** Same-user session bus only. No file paths, scripts or
   project I/O. The power is equivalent to the keyboard. While a modal dialog is
   open, `context.dialog` is `true` and controls/actions are ignored, except
   `Notify` and `GetContext`.

### 4.2 Transport and latency

* Session bus, existing well-known name `org.kde.kdenlive-<pid>`. The client
  picks the instance from the focused window's PID (Hyprland and KWin both
  report it), so several running instances work.
* **Measured** with this repository's mock on the user's machine (i9-10900,
  session `dbus-broker`): `Control` → `ControlAck` round trip **p50 104–121 µs,
  p99 177 µs, max 230 µs** over 2×2000 calls (`control-surfaced bench-dbus 2000
  --kdenlive-service …`). This is far below one frame at 60 fps, so **no private
  socket is needed**. A peer-to-peer address could be added later as a
  `PeerAddress` property without changing the methods.
* With a simulated 16 ms GUI cost per apply, acknowledgements arrive after about
  17.4 ms. Client and server coalescing (§4.5) keep the queue from growing.

### 4.3 Methods and signals

```xml
<interface name="org.kde.kdenlive.ControlSurface1">
  <method name="Capabilities"><arg direction="out" type="a{sv}"/></method>
  <method name="Subscribe"><arg direction="out" type="a{sv}" name="context"/></method>
  <method name="Unsubscribe"/>
  <method name="GetContext"><arg direction="out" type="a{sv}"/></method>

  <method name="ListActions"><arg direction="out" type="aa{sv}"/></method>
  <method name="TriggerAction">
    <arg direction="in" type="s" name="id"/><arg direction="out" type="b"/>
  </method>

  <method name="Control">
    <annotation name="org.freedesktop.DBus.Method.NoReply" value="true"/>
    <arg direction="in" type="s" name="control"/>
    <arg direction="in" type="d" name="delta"/>
    <arg direction="in" type="a{sv}" name="options"/>
    <arg direction="in" type="u" name="seq"/>
  </method>
  <method name="SetControlValue">
    <arg direction="in" type="s" name="control"/><arg direction="in" type="d" name="value"/>
    <arg direction="in" type="a{sv}" name="options"/><arg direction="out" type="b"/>
  </method>
  <method name="Invoke">
    <arg direction="in" type="s" name="command"/><arg direction="in" type="a{sv}" name="args"/>
    <arg direction="out" type="a{sv}" name="result"/>
  </method>
  <method name="Notify">
    <annotation name="org.freedesktop.DBus.Method.NoReply" value="true"/>
    <arg direction="in" type="s" name="text"/><arg direction="in" type="i" name="timeoutMs"/>
  </method>

  <signal name="ContextChanged"><arg type="a{sv}" name="context"/></signal>
  <signal name="ControlAck">
    <arg type="u" name="seq"/><arg type="s" name="control"/><arg type="a{sv}" name="state"/>
  </signal>
  <signal name="ActionsChanged"/>
</interface>
```

| Member | Semantics |
|---|---|
| `Capabilities()` | `{"version":1, "controls":as, "commands":as, "contextKeys":as, "implementation":s}`. Clients use it, not the Kdenlive version, to enable features. |
| `Subscribe()` | Registers the caller and returns the full context (§4.4). `ContextChanged` then follows changes. Idempotent per caller. |
| `Unsubscribe()` | Explicit release. The server also drops callers that leave the bus (`QDBusServiceWatcher`). |
| `GetContext()` | Snapshot without subscribing. |
| `ListActions()` | One entry per `KActionCollection` action: `{"id","text","shortcut","enabled","checkable","checked","category"}`. `ActionsChanged` fires when actions are added or removed (plugins, layouts). It is not emitted for enabled-state churn. |
| `TriggerAction(id)` | `QAction::trigger()` on the GUI thread. Returns `false` if the action is unknown, disabled, or blocked by a modal dialog. The client then uses its own fallback. |
| `Control(control, delta, options, seq)` | Continuous, relative and fire-and-forget (§4.5, §5). `delta` is in the control's unit (usually detents × user scale). |
| `SetControlValue(control, value, options)` | Absolute variant where it makes sense (`playhead.shuttle` index, `timeline.zoom` level). Returns whether it was applied. |
| `Invoke(command, args)` | Small, typed, discrete commands that are not `QAction`s or need a target (§5.5). Result is `{"ok":b, "error":s, …}`. |
| `Notify(text, timeoutMs)` | Shows a transient status-bar message (`pCore->displayMessage(…, InformationMessage)`), e.g. "Lift: R". |
| `ContextChanged(context)` | Full context, coalesced to at most one signal per 33 ms (≤ 30 Hz). Each carries an increasing `serial`. Emitted only while subscribers exist. |
| `ControlAck(seq, control, state)` | One per applied batch of a control. It carries the highest `seq` merged and the resulting state, e.g. `{"position":1234}`, `{"wheel":"lift","values":{"r":…}}`, or `{"error":"…"}` if nothing could be applied. |

### 4.4 Context schema

All keys are optional and clients must tolerate absent keys. Values come from
existing state; the hook column shows where.

| Key | Type / values | Kdenlive source (hook) |
|---|---|---|
| `serial` | int64, increasing | interface |
| `focus` | `timeline` · `clipMonitor` · `projectMonitor` · `effectStack` · `bin` · `automationEditor` · `mixer` · `subtitles` · `titler` · `other` | `QApplication::focusChanged` → owning dock (`KDDockWidgets` dock id) |
| `activeMonitor` | `clip` · `project` | `MonitorManager::activeMonitor()` (`monitor/monitormanager.h:42`) |
| `playing` / `speed` | bool / double | `Monitor` play state, shuttle speed |
| `position` / `fps` | frames (active monitor) / double | `Monitor::position()`, profile |
| `tool` | `select` · `razor` · `spacer` · `ripple` · `roll` · `slip` · `slide` · `multicam` | `ProjectTool` (`definitions.h:244`), timeline tool change |
| `timeline` | `{"zoom":d, "track":{"id","name","type","locked","muted","target"}, "selection":{"count","clips":ai}}` | `TimelineController` (active track, selection) |
| `effect` | `{"owner":{"type":"clip\|track\|master\|bin","id"}, "index", "id", "name", "enabled"}` | `AssetPanel`/`EffectStackView` active item |
| `param` | `{"name", "type", "min", "max", "step", "value", "keyframed"}` | focused `AbstractParamWidget` → `AssetParameterModel` index |
| `colorWheel` | `{"wheel":"lift\|gamma\|gain"}` when a wheel of `lumaliftgainparam` has focus or hover | `LumaLiftGainParam` (`assets/view/widgets/lumaliftgainparam.cpp`) |
| `automation` | `{"open":b, "channel", "selectedPoints":i}` | `AutomationPanel` (`assets/automation/automationpanel.hpp`). K03 only; omitted upstream |
| `dialog` | bool: a modal dialog is open | `QApplication::activeModalWidget()` |

Only **coarse focus** is published. Tracking focus at widget level beyond the
table is out of scope.

### 4.5 Coalescing and acknowledgement (normative)

Client (implemented in `src/daemon/coalescer.*` and `engine.cpp`):

1. The first detent of an idle control is sent **immediately**.
2. While a control (`control` + `options`) has an unacknowledged message,
   further deltas are **summed locally**. Opposite detents cancel, and a net
   zero sends nothing.
3. A `ControlAck` releases the next merged message. The client stops waiting
   after `ackTimeoutMs` (default 60 ms) so a lost ack never freezes a knob. It
   also never sends one control more often than every `coalesceMs` (default 8 ms).
4. On focus change to another application, pending deltas are **dropped**, never
   delivered elsewhere.

Server (implemented in the mock, required for Kdenlive):

1. `Control` only accumulates into `pending[control+options]` and schedules one
   `QTimer::singleShot(0)` apply. Everything that arrives before the apply runs
   is merged.
2. The apply runs on the GUI thread, calls the existing model/monitor APIs once
   per key, and emits one `ControlAck(lastSeq, control, state)` per key.
3. Seeks reuse the monitor's existing seek-request path, which already drops
   superseded frames, so jog never queues renders.

Result in the test suite: 200 jog detents with a 15 ms-per-apply "busy GUI"
arrive as **fewer than 40 messages**, the final position is exact (200), and no
detent is lost (`tests/tst_kdenlive_dbus.cpp`).

### 4.6 Undo semantics for continuous controls

* Transport, zoom, scroll and track focus create **no undo entries**. They are
  view state.
* `param.nudge`, `colorwheel.nudge`, `audio.gain`, `automation.nudge` and
  `edit.trim` form **one undo entry per gesture**. A gesture is consecutive
  deltas on the same control and target, ending after 600 ms without input, a
  change of target or focus, or `options.gesture == "end"`.
  The parameter widgets already support this: `LumaLiftGainParam` emits
  `valuesChanged(…, createUndo)` (`lumaliftgainparam.hpp:42-47`). Apply
  intermediate values with `createUndo=false` and finish with one undo command
  from the gesture's start value.
* Undo/redo themselves are just `TriggerAction("edit_undo")`/`("edit_redo")`.

---

## 5. Controls and commands (version 1)

Every control accepts `options`. Unknown options are ignored. `"target"` defaults
to "whatever is focused/selected", matching what the mouse or keyboard would
change.

### 5.1 Transport and view

| Control | Unit of `delta` | Options | Implementation hook |
|---|---|---|---|
| `playhead.jog` | frames | `scrub` (bool, audio while jogging), `monitor` (`active`\|`clip`\|`project`) | `Monitor::slotForwardOneFrame(int diff)` / `slotRewindOneFrame(int diff)` (`monitor/monitor.h:368-369`), via `MonitorManager` |
| `playhead.shuttle` | shuttle steps (index into `0,1,2,4,5,8,16,60`, signed) | – | Same table and calls as `JogShuttleAction` (`jogshuttle/jogaction.cpp:16-34`): `slotForward(speed)`/`slotRewind(speed)`, index 0 = pause. Absolute form through `SetControlValue`. |
| `timeline.zoom` | zoom steps (+ = in) | `anchor`: `playhead`\|`mouse` | `TimelineController::setScaleFactor[OnMouse]` (`timeline2/view/timelinecontroller.h:168-169`) / `MainWindow::slotZoomIn/Out` |
| `timeline.scroll` | tenths of the visible width | – | timeline QML flickable `contentX` through `TimelineController` |
| `timeline.track` | tracks (+ = up) | – | active track change, as the Up/Down track actions do |

### 5.2 Parameters and colour

| Control | Unit | Options | Hook |
|---|---|---|---|
| `param.focus` | parameters (+ = next) | – | move focus within the focused `AssetParameterView` |
| `param.nudge` | parameter steps | `param` (name, default focused), `step`: `normal`\|`fine` (fine = 1/10), `keyframe`: `auto`\|`create`\|`static` | `AssetParameterModel::setParameter` / `KeyframeModelList::updateKeyframe` at the playhead when the parameter is keyframed, with the same rules as editing the widget. Clamped to min/max; step = parameter `step` or (max−min)/100. |
| `colorwheel.nudge` | 0.01 per step (lift range −1…1, gamma/gain 0…5, i.e. the XML `factor` scale) | `wheel`: `lift`\|`gamma`\|`gain` (default the focused wheel); `axis`: `luma` (r,g,b together) \| `r` \| `g` \| `b` \| `hue` \| `saturation` | Write `lift_r…gain_b` (`data/effects/lift_gamma_gain.xml`) through the same path as `LumaLiftGainParam::liftChanged/gammaChanged/gainChanged` (`lumaliftgainparam.cpp:49-72`). The effect is the focused effect, or the first `lift_gamma_gain` on the selected clip. |
| `audio.gain` | 0.1 dB | `target`: `clip`\|`track` | the clip's `volume` effect or the track's mixer gain |
| `edit.trim` | frames | – | trim the selected edit point under the active `tool` (ripple/roll/slip/slide), using the same model operations as the QML drag handlers |

### 5.3 Ack state per control

`playhead.jog` → `{"position"}` · `playhead.shuttle` → `{"shuttle","speed"}` ·
`timeline.zoom` → `{"zoom"}` · `param.nudge` → `{"param","value"}` ·
`colorwheel.nudge` → `{"wheel","axis","values":{"r","g","b"}}` · `audio.gain` →
`{"gainDb"}` · `edit.trim` → `{"trim","tool"}` · errors → `{"error"}`.

### 5.4 Optional: automation curves (only in builds with K02/K03)

| Control | Unit | Options | Hook |
|---|---|---|---|
| `automation.nudge` | value: 1/100 of the channel range per step; time: frames | `axis`: `value`\|`time` | Move the selected points of the open `AutomationPanel` through `AutomationService`, one undo per gesture (`assets/automation/automationservice.cpp`). Advertised in `Capabilities` only when available, so upstream can omit it. |

### 5.5 `Invoke` commands (version 1)

| Command | Args | Effect |
|---|---|---|
| `colorwheel.reset` | `wheel` | Reset one wheel to its defaults (lift 0, gamma/gain 1), one undo |
| `param.reset` | `param` (default focused) | Reset to the XML default, one undo |
| `effect.toggle` | `index` (default focused) | Enable/disable the focused effect |
| `track.toggle` | `what`: `mute`\|`lock`\|`hide`\|`solo`\|`target`; `track` (default active) | Track header toggles that have no `QAction` |
| `marker.add` | `category` (int), `comment` | Add a marker/guide at the playhead in a category without a dialog |

### 5.6 Errors

* Methods with replies return D-Bus errors only for malformed calls
  (`org.freedesktop.DBus.Error.InvalidArgs`).
* Domain failures are values: `TriggerAction` returns `false`, `Invoke` returns
  `{"ok":false,"error":…}`, and `ControlAck.state` carries `{"error":…}`. A
  client must therefore receive an ack even when nothing was applied.

---

## 6. Implementing it in Kdenlive (sketch for MAIN)

* New directory `src/controlsurface/`:
  * `controlsurfaceinterface.{h,cpp}`: a `QObject` with
    `Q_CLASSINFO("D-Bus Interface","org.kde.kdenlive.ControlSurface1")`, plus
    `QDBusContext` for the caller name. Register it in `MainWindow` after GUI
    setup with
    `QDBusConnection::sessionBus().registerObject("/ControlSurface", …, ExportAllSlots|ExportAllSignals)`.
    The service name is already owned by `KDBusService` (`main.cpp:564`).
  * `contextwatcher.{h,cpp}`: connects only while subscribers exist. It
    watches `QApplication::focusChanged`, `MonitorManager` activation,
    `TimelineController` tool/track/selection signals and `AssetPanel`
    item/parameter focus, then rate-limits `ContextChanged`.
  * `controldispatcher.{h,cpp}`: pending map, apply-once-per-loop, gesture
    undo grouping.
* **Share with JogShuttle.** Move the shuttle table and the jog/shuttle calls
  out of `JogShuttleAction` into a small `TransportControl` used by both, so the
  two device paths behave identically.
* A setting `KdenliveSettings::enableControlSurfaceInterface` (default on;
  nothing runs unless someone subscribes or calls) lets distributions disable it.
* Tests: a `QDBusServer` peer-to-peer test like `tests/tst_kdenlive_dbus.cpp`
  in this repository. Cover controls on a loaded test project: jog/shuttle/zoom
  state, parameter nudge clamping, one undo entry per gesture, and no signals
  without subscribers.
* Rough size: 700–1000 lines plus tests.

**Merge-request split.**
1. Interface skeleton: `Capabilities`, `Subscribe`/`GetContext` (focus,
   monitor, tool, playing, position), `ListActions`/`TriggerAction`, transport
   and zoom controls, `Notify`, and the JogShuttle refactor.
2. Effect and parameter context, `param.focus`, `param.nudge`,
   `colorwheel.nudge`, gesture undo grouping, `Invoke` commands.
3. `edit.trim`, `audio.gain`, `timeline.scroll`, `timeline.track`.
4. (Downstream, our tree only) `automation.nudge` for K02/K03.

## 7. Conformance kit in this repository

* `mock-kdenlive`: a reference server for this contract with a scripted console
  (`focus effectStack`, `effect lift_gamma_gain`, `param level`, `tool ripple`, `state`).
* `control-surfaced bench-dbus N --kdenlive-service org.kde.kdenlive-<pid>`:
  measures round-trip latency against any implementation, including a real
  Kdenlive build.
* `control-surfaced run --kdenlive-service org.kde.kdenlive-<pid>`: drive a real
  build with the pad. `--force-window org.kde.kdenlive` skips window tracking.
* `tests/tst_kdenlive_dbus.cpp`: wire-level expectations (nested `a{sv}`,
  `aa{sv}`, NoReply plus ack, rate-limited context, unsubscribe silence,
  fallback when the interface is absent).

## 8. Open questions for MAIN / upstream

1. **Naming.** Is `org.kde.kdenlive.ControlSurface1` on `/ControlSurface`
   acceptable? An alternative is folding it into a future generic
   `org.kde.kdenlive.Scripting` (K04/K05). Keeping it separate is recommended:
   it is a UI-control channel, not a data API, and is reviewable on its own.
2. **Default on or opt-in?** The proposal is default on, idle unless used, and
   keyboard-equivalent power. Upstream may prefer an explicit setting.
3. **Focus granularity.** Is dock-level focus plus "focused parameter" enough,
   or should hover count (wheels are often adjusted while hovered, not focused)?
   The proposal counts a hovered `ColorWheel` as `colorWheel.wheel`.
4. **Keyframed parameters.** Should `param.nudge` default to creating a keyframe
   at the playhead (`keyframe:"auto"` mirrors the widget) or refuse when not on
   a keyframe?
5. **Track mute/solo ids.** Confirm the action ids, or keep them as
   `Invoke("track.toggle")` only.
6. **Hue/saturation axes** for wheels: compute in the wheel's own colour model
   (`NegQColor`) so the knob matches mouse dragging.
7. **MCP.** Should a few of these (`GetContext`, `TriggerAction`) also be
   exposed as MCP tools for agents? This is optional and does not block K23.
8. **KWin backend for the daemon.** On Plasma the focused window's PID comes
   from a KWin script over D-Bus. Nothing is needed from Kdenlive for that.
