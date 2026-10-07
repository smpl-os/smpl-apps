# Kdenlive control-surface API contract (K23, revision 2)

Status: **fixed wire contract, revision 2 (2026-10-07)**. The normative text is
MAIN's `k23-contract-revised.md`, including its clarification that ControlAck
outcomes carry the session
(SHA-256 `5da6dcee7e8cc0c56ded09654a49d6113ba076439cdf590c12aa14e1904cc5cf`).
That file supersedes the first proposal in this file
(SHA-256 `1829fbc4…fc2a`). §4 below mirrors it; where this file and MAIN's
differ, MAIN's wins.

This repository implements the **client** (`src/daemon/kdenlivedbusclient.*`,
`engine.*`) and a **reference mock** (`src/mock/`). The mock is checked over
real D-Bus marshalling: on a peer connection, and on a private session bus with
several subscribers. Kdenlive itself is not implemented here. Mock results are
transport/behaviour evidence only. They say nothing about native editing
correctness or real-editor latency.

Interface: `org.kde.kdenlive.ControlSurface1`, object `/ControlSurface`, on the
service Kdenlive already owns, `org.kde.kdenlive-<pid>` (session bus). It is
compiled only with `USE_DBUS` and controlled by a user-visible setting that is
**off by default**. While off, the object is absent.

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

**Conclusion:** add one small, optional interface (default off). It exposes UI
context, invokes a curated set of actions by name and accepts coalesced
continuous controls. It reuses what Kdenlive already has and adds no editing
engine. MAIN's review (`k23-control-surface-review.json`) accepted this
direction with the revisions captured in §4.

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
"Stage" is where revision 2 places the operation (MR1, MR2, MR3, or
"action": a curated `TriggerAction` id in the allowlist).

| # | Stock KAction (id) | Our `/Editor`/MCP | Revision-2 coverage | Stage |
|---|---|---|---|---|
| N1 | ✅ `monitor_play`, `monitor_pause` | ❌ | `TriggerAction` | action |
| N2 | ✅ J/L (`monitor_seek_backward/forward`), JogShuttle internals | ❌ | `playhead.shuttle` (index −7…7, 0 pauses; `SetControlValue`) | MR1 |
| N3 | ✅ `monitor_seek_*-one-frame/-one-second` | ❌ | `playhead.jog`: transport only, never the tool-aware JogShuttle slots | MR1 |
| N4 | ◐ `mlt_scrub` (global preference) | ❌ | `playhead.jog` + `scrub` option; the global preference is never changed | MR1 |
| N5–N8 | ✅ `seek_*`, `monitor_seek_snap_*`, `monitor_seek_guide_*`, `monitor_loop_zone` | ◐ read-only guides/markers | `TriggerAction` (curated) | action |
| M1–M2 | ✅ `mark_in/out`, `add_marker_guide_quickly` | ◐ `create_guide` | `TriggerAction` (curated; no interactive dialogs) | action |
| T1–T2 | ✅ `insert_to_in_point`, `overwrite_to_in_point`, `switch_track_target` | ◐ legacy insert | `TriggerAction`; track target via `track.set` | action / MR3 |
| E1–E2 | ✅ `cut_timeline_clip`, `delete_timeline_clip` | ◐ explicit-id edits | `TriggerAction` | action |
| E3 | ✅ tool actions | ❌ | `TriggerAction`; `tool` in context | action |
| E4 | ❌ (QML drag only) | ❌ | `edit.trim` with explicit `edge`, `mode` and a host-issued edit target; qualified modes only | MR3 |
| K1 | ◐ QML only | ❌ | `timeline.track` (visual steps; native id in context) | MR3 |
| K2 | ◐ `switch_track_lock`, `switch_track_disabled` (hide video / mute audio); solo is in `MixerManager` | ◐ clip mute only | `Invoke("track.set")` with type applicability and an explicit solo mode | MR3 |
| F1 | ✅ `keyframe_add/next/previous` | ◐ automation apply | `TriggerAction` | action |
| P1 | ❌ | ❌ | `param.focus` (no data change) | MR2 |
| P2 | ❌ | ◐ `set_effect_parameters` | `param.nudge` (existing-keyframe default, explicit `create`), `Invoke("param.reset")` | MR2 |
| C1–C2 | ❌ (mouse on `ColorWheel`) | ◐ raw params | `colorwheel.nudge` (value/r/g/b; hue/saturation only if advertised), `Invoke("colorwheel.reset")` | MR2 |
| A1–A2 | ❌ (mixer writes bypass undo today) | ◐ effect params | `audio.gain` with an explicit clip/track target | MR3 |
| V1 | ✅ `view_zoom_in/out`, `zoom_fit` | ❌ | `timeline.zoom` (native zoom-level mapping); `timeline.scroll` | MR1 / MR3 |
| V2 | ✅ `switch_monitor` | ❌ | `TriggerAction`; `activeMonitor` in context | action |
| U1 | ✅ `automation_editor` (local) | ✅ automation APIs | **Deferred**: an optional downstream bridge, not part of revision 2 | – |
| L1 | ◐ K01 (local), K21/K12 plugins | ✅ guarded MCP jobs | Long jobs stay on their guarded APIs; not exposed | – |
| R1 | ✅ `edit_undo/redo`, `file_save` | ✅ guarded history/save | `TriggerAction` (curated) | action |
| – | **UI context** | ❌ | `Subscribe`, `GetContext`, directed `ContextChanged` | MR1 |

`mlt_mute` (`monitor/monitormanager.cpp:796`) is the **monitor** mute, not a
track mute.

---

## 4. Revision 2 wire contract (mirror of MAIN's file)

### 4.1 Availability and authorization

* Session bus only. Service `org.kde.kdenlive-<pid>`, object `/ControlSurface`,
  interface `org.kde.kdenlive.ControlSurface1`. Built only with `USE_DBUS`. The
  setting defaults **off**, and while off the object is absent. No daemon call
  can enable it.
* An explicit `QDBusAbstractAdaptor` exports only the members below.
* `Subscribe` grants a lease bound to the caller's unique bus name. Every
  mutation requires that lease, and a lease is not transferable. Losing the bus
  name or unsubscribing drops queued work and ends the gesture already applied.
* Mutations are refused in these states: application inactive, modal, recovery,
  closing, conflicting edit. A call must pass admission and dispatch checks.
* Falling back to the keyboard when the interface is unavailable is the
  client's policy. **Domain refusals and timeouts never authorize keyboard
  fallback.**

### 4.2 Members (`t` = uint64)

| Member | Inputs | Output |
|---|---|---|
| `Capabilities` | none | `a{sv}` |
| `Subscribe` | none | `a{sv}` |
| `Unsubscribe` | `s session` | `a{sv}` |
| `GetContext` | none | `a{sv}` |
| `ListActions` | none | `a{sv}` |
| `TriggerAction` | `s id, a{sv} options` | `a{sv}` |
| `Control` | `s control, d delta, a{sv} options, t seq` | NoReply |
| `SetControlValue` | `s control, d value, a{sv} options` | `a{sv}` |
| `Invoke` | `s command, a{sv} args` | `a{sv}` |
| `Notify` | `s text, i timeoutMs, a{sv} options` | `a{sv}` |

Signals are **destination-addressed** to the relevant subscriber, never
broadcast:

| Signal | Arguments |
|---|---|
| `ContextChanged` | `a{sv} context` |
| `ControlAck` | `t seq, s control, a{sv} outcome` |
| `ActionFinished` | `t requestId, a{sv} outcome` |
| `ActionsChanged` | none |

### 4.3 Envelopes

* Success: `{ok: true, result: a{sv}}`.
* Failure: `{ok: false, error: {code: s, message: s, field: s}}`.
* There is no "false means fallback" convention. Invalid D-Bus signatures or
  types remain transport errors. Semantic and type validation inside variant
  maps is reported as a structured failure.

Results:

* `Capabilities.result`: `{version: u(1), revision: u(2), implementation: s,
  controls: as, commands: as, contextKeys: as, limits: a{sv}}`. The optional
  `controlDescriptors: aa{sv}` describes units, axes and absolute support.
  Clients must not send controls absent from `controls`.
* `Subscribe.result`: `{session: s, context: a{sv}}`. Repeating it from the same
  sender is idempotent.
* `GetContext.result` is the context itself.
* `Unsubscribe.result`: `{unsubscribed: true}`.
* `ListActions.result`: `{actions: aa{sv}}`, each entry
  `{id, text, enabled, checkable, checked, shortcut}`. This is a curated
  allowlist, not every action the application registers.
* `TriggerAction.result`: `{state: "accepted", requestId: t, id: s}`. The action
  is revalidated before invocation. The directed `ActionFinished` reports
  `{ok:true, result:{state:"invoked", id}}` or an error. "Invoked" does not mean
  the work the action started has completed.
* `Notify`: needs the common options, text ≤ 256 characters and a timeout of
  500–5000 ms. Returns `{shown: true}` and never changes the project.

Unknown read-only result keys may be ignored. Unknown behaviour-changing input
options are rejected.

### 4.4 Context

Required keys:

| Key | Meaning |
|---|---|
| `serial: t` | Observational snapshot sequence; may change with playhead state |
| `epoch: t` | Target/context generation, independent of the playhead |
| `ready`, `active`, `dialog`: `b` | |
| `focus`, `project`, `sequence`: `s` | `project`/`sequence` are empty when unavailable |
| `activeMonitor: s` | `clip`, `project` or `none` |
| `position: i` | |
| `fps: {num: i, den: i}` | |
| `playing: b`, `speed: d`, `tool: s` | |

Additive keys: `timeline`, `effect`, `param`, `colorWheel`,
`hoveredColorWheel`. `timeline.track` is an object holding the native track id
and its sequence identity (plus the opaque `target`); its label (A1/V1) is
display metadata only.

* A mutable target is an opaque, host-issued `target: s`. Labels, visual
  indices and asset names are not identities.
* Parameters describe their displayed units, range, step and editing policy.
* Keyboard focus is distinct from hover, and hover never retargets an existing
  gesture.
* Context signals are capped at **30 per second** with monotonic timing (a
  34 ms minimum spacing is acceptable). With no subscribers there is no
  polling or context timer.
* Signals go only to current subscribers. An unsubscribed caller receives none,
  even while another caller stays subscribed.

### 4.5 Mutation options, controls and acknowledgements

* Common mutation options are `session: s` and `epoch: t`. Editing controls also
  carry `target: s` and a nonempty, caller-chosen `gesture: s`.
* `phase` is `update` (default), `end` or `cancel`. An explicit end with zero
  delta is still a barrier and must be delivered.
* Within a gesture, the captured target and edit frame do not move.
  Context-changing discrete operations end editing gestures before acting.
* Sequence numbers are positive and strictly increasing per lease, never reset
  within a lease. A new lease starts a new sequence space.
* Pending keys include sender, lease, epoch, gesture, target, control and the
  validated semantic options. Different callers or targets are never merged.
* Each applied coalesced batch acknowledges its highest sequence. The outcome
  envelope also carries a top-level `session: s` for lease correlation,
  **including errors**. Its result includes `state: "applied"`, `firstSeq`,
  `lastSeq`, `epoch`, `gesture`, `target`, `changed` and control-specific state.
  Refusals are acknowledged too.
* Duplicate or old sequences are not applied again; they get
  `stale_sequence`.
* Relative deltas are summed, then clamped once (**net batch**, not per detent).
* NaN/infinity, oversized messages, unknown controls/options and out-of-range
  numbers are rejected before queuing.
* Default limits: 8 leases, 64 pending keys, 32 queued actions, 32 options, and
  4096 characters of aggregate string input per call. Bounded deltas are
  advertised. Excess queued work is refused with `resource_limit`.
* Applied view state is acknowledged without waiting for decoded frames. There
  are no waits, joins or nested event loops in the adaptor.
* Focus, project, sequence or owner changes invalidate pending work. Dropped
  batches get `stale_context` while their sender remains subscribed.

### 4.6 Staged controls

| Stage | ID | Relative unit | Options beyond the common ones |
|---|---|---|---|
| MR1 | `playhead.jog` | integral frames | `monitor: active/clip/project`, `scrub: b` |
| MR1 | `playhead.shuttle` | integral index steps (abs −7…7 via `SetControlValue`) | `monitor` |
| MR1 | `timeline.zoom` | integral steps, + zooms in | `anchor: playhead/mouse` |
| MR2 | `param.focus` | integral parameters, no data change | – |
| MR2 | `param.nudge` | steps in advertised display units | `step: normal/fine`, `keyframe: existing/create` (default `existing`) |
| MR2 | `colorwheel.nudge` | wheel steps | `wheel: lift/gamma/gain`, `axis: value/r/g/b` (hue/saturation only if advertised) |
| MR3 | `timeline.track` | integral visual steps | – |
| MR3 | `timeline.scroll` | tenths of visible width | – |
| MR3 | `audio.gain` | 0.1 dB | explicit target, no implicit effect creation |
| MR3 | `edit.trim` | integral frames | `edge: start/end`, `mode: resize/ripple/roll/slip/slide` (qualified only) |

Commands:
* `Invoke("param.reset")` and `Invoke("colorwheel.reset")` (MR2).
* `Invoke("track.set")` (MR3), with an explicit target, `what:
  mute/hide/lock/solo/target`, `value: b` and `soloMode: exclusive/additive`.

Editing rules (MR2/MR3):
* Never create effects implicitly, fall back to the first matching effect, or
  insert keyframes implicitly.
* Static and single-key values update in place. Multi-key animation needs a key
  at the captured frame, or an explicit `create`.
* Managed automation and unsupported types are refused.
* There is one owned undo entry, or one closed command group, per gesture.
  Merge only the same gesture, document, target, component set and edit frame.
* A gesture ends after 600 ms idle, an explicit end, a target or focus change,
  unrelated history, save, or undo/redo. No-op gestures make no history.
* Cancel restores only while the gesture is still owned; otherwise it returns
  `history_conflict`.
* Unsupported or unqualified controls and modes return `unsupported_control` or
  `unsupported_mode`. They are never mapped to another operation.

The optional automation bridge and all MCP additions are deferred.

### 4.7 Error codes

`invalid_arguments`, `not_subscribed`, `stale_context`, `stale_sequence`,
`inactive`, `modal`, `not_ready`, `closing`, `busy`, `resource_limit`,
`unknown_action`, `action_disabled`, `unsupported_control`, `unsupported_mode`,
`target_not_found`, `unsupported_parameter`, `keyframe_required`,
`managed_parameter`, `track_locked`, `history_conflict`, `edit_failed`.

---

## 5. How the daemon uses it (client policy)

### 5.1 States

The client walks `Capabilities` → `Subscribe` → `ListActions` and lands in one
of four states:

| State | When | Pad behaviour in a Kdenlive window |
|---|---|---|
| Available | Revision ≥ 2 subscribed | Advertised controls, commands and allowlisted actions only. Anything else is reported once and **not** typed. |
| Absent | `ServiceUnknown`, `NameHasNoOwner`, `UnknownObject`, `UnknownInterface` or `UnknownMethod`; or an incompatible version/revision | Plain stock Kdenlive (including interface off, the default): configured stock shortcut keys. |
| Pending | Timeout, other transport error, or a refused `Subscribe` | Nothing is sent and nothing is typed. Retried on input or refocus (Pending after 2 s, Absent after 5 s). |
| Detached | No Kdenlive window focused | Other profiles apply. |

A domain refusal (`ok:false`, in `TriggerAction` or `ActionFinished`,
`Invoke`, or `ControlAck`) is reported, never typed. If an action call proves
the interface vanished, the stock key goes only to the same Kdenlive window
(same pid and address) that was asked.

Further client rules:

* Interface and stock decisions apply only when the focused window's pid equals
  the attached instance's pid. During a provisional focus event (pid not known
  yet) or before re-attaching, input is reported and dropped: it is neither
  sent to the previous instance nor typed.
* If a `Subscribe` reply arrives after the daemon has already detached, the
  granted lease is returned with `Unsubscribe`. The exception is when the
  daemon is attached to that same instance again, because `Subscribe` is
  idempotent per sender.
* If `ListActions` fails, the client stays Pending (absent-class errors
  become Absent) and retries. It never becomes Available with an empty
  allowlist. A failed refresh keeps the previous allowlist.

### 5.2 Sequences, correlation and coalescing

* Acks are matched on **(session, seq)**. An outcome whose `session` is not the
  current lease is ignored, on success or error. As defence in depth, `seq` is
  a uint64 that is never reset within the daemon process, so a new lease never
  reuses a sequence number.
* The client keeps `seq → {key, control, target, gesture}`. An ack is accepted
  only if all of these hold:
  * it comes from the service owner (QtDBus sender match);
  * it carries the current session;
  * its `seq` is known and its `control` matches;
  * on success, it echoes the `target` that was sent. It settles every message ≤ `seq` under that key. It releases the key
  only if `seq` is the key's newest message, so a late ack cannot release a
  newer batch. Unknown, duplicate, foreign and wrong-target acks release
  nothing.
* Coalescing keys:
  * transport: `control|options`;
  * editing: `slot|control|options|gesture|target`.
  At most one message is in flight per key, sent no more often than every 8 ms,
  with a 60 ms ack timeout and at most 64 live keys.

### 5.3 Gestures and targets

* Editing controls take their target from the context:
  * `param.nudge`/`param.reset` → `param.target`
  * `colorwheel.*` → `colorWheel.target`
  * `audio.gain` → `audio.target`
  * `edit.trim` → `edit.target`
  * `track.set` → `timeline.track.target`

  A binding may opt into hover with `"targetFrom": "hoveredColorWheel.target"`.
  With no target, nothing is sent.
* A gesture (`cs-<pid>-<n>`) is scoped to one binding (slot, control and
  options), one target and one epoch. An open gesture keeps the target it
  captured; only a new gesture reads the target from the context, so a moving
  hover never retargets a turn in progress. It ends with an explicit `phase:"end"`
  barrier in four cases:
  * 500 ms idle (`gestureIdleMs`; the host ends at 600 ms);
  * a mode cycle;
  * before any discrete action or `Invoke`;
  * a change of target or binding options.
* The end barrier is sent immediately, even with zero delta. Ordering after the
  in-flight update is guaranteed by D-Bus per-sender ordering.
* On an `epoch` change, or a `stale_context`/`target_not_found` refusal, queued
  motion is dropped without being sent. The host has already ended those
  gestures, and queued deltas are never re-targeted.
* A window change drops all pending work and unsubscribes.

### 5.4 Example bindings (data/config.example.jsonc)

* **MR1:**
  * knob 1 jogs; its press is `monitor_play`;
  * knob 2 shuttles; its press is `monitor_pause`;
  * knob 3 zooms; its press is `zoom_fit`;
  * the keys are curated actions.
* **MR2, when `colorWheel.target` is present:** knobs 1–3 are the
  lift/gamma/gain wheels. A press cycles value → r → g → b, and keys 6–8 reset
  a wheel.
* **MR2, effect stack with a `param.target`:** knob 1 nudges and its press
  toggles normal/fine; knob 3 runs `param.focus`.
* **MR3, `ripple`/`roll` tool with an `edit.target`:** knob 2 trims, and its
  press toggles the edge. `mode` comes from `$ctx:tool`.

---

## 6. Mock conformance (src/mock)

`MockKdenlive` exports only the contract members through an explicit
`QDBusAbstractAdaptor` (`ExportAdaptors`); test helpers are not exported. It
implements:

* sender-bound leases (owner loss via `QDBusServiceWatcher`), with the lease
  echoed in every ControlAck outcome; a `Control` from a caller with no lease
  is dropped silently, with no signal of any kind;
* admission order: options/size → lease → unknown options → epoch → ready,
  closing, active, modal. Ready, closing, active and modal are checked again
  at dispatch, for both control batches and queued actions;
* per-control option allowlists;
* integral and finite deltas within `maxDelta`;
* `stale_sequence`;
* pending keys built from sender, session, epoch, gesture, target, control and
  options;
* one apply per event-loop pass, plus a configurable busy-GUI delay;
* directed `ControlAck`, `ContextChanged` and `ActionFinished` (targeted signals
  on the bus, connection-scoped on peers);
* a 34 ms monotonic context limiter that is idle without subscribers;
* staged capabilities (`--stage 1|2|3`) and `--off`, which owns the service
  but not the object;
* one history entry per changed gesture, none for a no-op, judged on the
  gesture's own captured parameter;
* cancel and `history_conflict`. Ended gestures are remembered: a late cancel
  gets `history_conflict`, a late update `stale_context`, and a late end is
  acknowledged as idempotent. Unrelated history ends open gestures;
* `stale_context` for batches dropped on an epoch change;
* `TriggerAction` accepted → revalidated → `ActionFinished` invoked or
  refused;
* transport-only jog, so Slip mode leaves history untouched;
* shuttle indices with 0 = pause.

Tests:

* `tests/tst_kdenlive_dbus.cpp` covers the wire on a peer connection.
* `tests/tst_kdenlive_bus.cpp` runs under `dbus-run-session` on a private
  session bus. It covers two or more subscribers, directed acks and context,
  sender separation, lease forgery, unsubscribe silence, owner loss, the
  ≤ 30 Hz limit, the lease limit, and client ack correlation against a scripted
  server: late, foreign, duplicate and wrong-target acks, and no reuse of
  sequence numbers across leases. It also distinguishes absent from timeout.
* `control-surfaced bench-dbus N --kdenlive-service NAME` measures
  `Control`→`ControlAck` round trips against any implementation. On the mock
  (revision 1 wire) the earlier result was p50 ≈ 110 µs and p99 ≈ 177 µs. That
  is transport evidence only.

---

## 7. Ambiguities and suggested clarifications (reported to the coordinator)

1. **Resolved:** ControlAck outcomes now carry `session`, also on errors. The
   daemon matches on (session, seq), covered by `lateAckFromOldLeaseIsIgnored`.
2. **Shape of a refusal ack.** The `outcome` is assumed to be an envelope.
   Error outcomes carry no `firstSeq`/`lastSeq`/`target`/`gesture`, so the
   client correlates by the `seq` argument (the highest seq of the dropped
   batch). Confirm this, or add correlation fields to errors.
3. **Editing options on non-editing controls.** Whether MR1 controls and
   `param.focus` must reject `target`/`gesture`/`phase` is not stated. The mock
   rejects them as unknown options, and the daemon never sends them.
4. **Error code for oversized input.** Not specified. The mock uses
   `invalid_arguments` (with `field`) for too many options or strings that are
   too long, and `resource_limit` for queue and lease limits.
5. **Unknown `Invoke` command.** There is no `unsupported_command` code. The
   mock uses `unsupported_control` with field `command`.
6. **Names inside `limits` and `controlDescriptors`.** Not fixed. The mock
   uses:
   * `limits`: `maxLeases`, `maxPendingKeys`, `maxQueuedActions`, `maxOptions`,
     `maxStringInput`, `maxNotifyText`, `maxContextRateHz`;
   * `controlDescriptors`: `id`, `unit`, `integral`, `maxDelta`, `editing`,
     `absolute`, `options`, `axes`, `modes`.
7. **Context paths for MR3 targets.** `timeline.track` is settled as an object
   with the native id and sequence. The daemon takes `track.set`'s target from
   `timeline.track.target`, and the mock publishes it there. The paths for
   `audio.gain` (`audio.target`) and `edit.trim` (`edit.target`, plus the
   captured group/ripple scope) are still the daemon's assumption.
8. **An `end` with a nonzero delta.** The daemon may send the last pending
   delta together with `phase:"end"`. The mock applies it, then ends. Confirm
   this is allowed.
9. **End after an epoch change.** The daemon sends no `end` for gestures that
   an epoch change invalidated, because the host has already ended them.
   Confirm no explicit end is expected.
10. **Revision compatibility.** The daemon treats revision < 2 as absent (stock
    fallback) and accepts revision ≥ 2 of version 1.
