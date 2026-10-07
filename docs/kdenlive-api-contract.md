# Kdenlive control-surface API contract (K23, revision 2)

Status: **fixed wire contract, revision 2 (2026-10-07); K23 MR1–MR3 qualified**
(Kdenlive app SHA-256 `7a0f189da86538c1fccfe530fc95900fd94c635a550b15be3d6a9747598f4848`).
The normative texts are MAIN's:

* `k23-contract-revised.md`, SHA-256
  `39774d0c106e8c215b0c70a4e95d94a52ac9ecda34686bae78c3e5033514d6e8`
  (mirrored verbatim in §4.1–§4.7);
* the addendum `k23-contract-rev2-wire-clarifications.md`, SHA-256
  `b2121cc0d8cddc642498c10e482eb533ec6db1637744c8ceee06469c194d6de2`
  (mirrored verbatim in §4.8). It records the frozen host's exact serialization
  and changes no wire behaviour;
* the additive MR1a action contract `k23-contract-mr1a-actions.md`, SHA-256
  `1e45ae44127945221a4690b13cff4662c2f510469b9abeb80c752f6c5b0c15be`
  (mirrored verbatim in §4.9): 71 curated action candidates, context
  restrictions, and Slip-preview transport admission. MAIN has implemented it;
  it is **not yet qualified against this daemon** (no new lease yet);
* MR1a clarifications `k23-mr1a-mock-clarifications.md`, SHA-256
  `ccb3d676d68a3b9599aec1c14691d1c3a8fa24b7b266327189a4f49811b8f4f2`
  (mirrored verbatim in §4.10): the exact editing-action set, the trimming
  preview in `enabled`, admission and dispatch checks, and action history
  advancing `epoch`. No wire or pin change.

Previous mirrors: `e061f20c…a8357` (MR2: `colorWheels`, `emittedAtMs`),
`5da6dcee…cc5cf` (`session` in ControlAck outcomes); the first proposal in
this file was `1829fbc4…fc2a`. Where this file and MAIN's differ, MAIN's wins.
§7 lists what each of my clarification requests became.

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

**Qualified host (app `7a0f189d…`).** All ten controls and three commands of
MR1–MR3 are advertised; `edit.trim` offers `trimModes: ["resize"]` only. Its
`TriggerAction` allowlist has **7** ids: `monitor_play`, `monitor_pause`,
`monitor_loop_zone`, `switch_monitor`, `zoom_fit`, `view_zoom_in`,
`view_zoom_out`.

**K23-MR1a (§4.9, implemented, not yet qualified with this daemon).** The
allowlist grows to 71 curated candidates (only IDs the build registers are
listed). They cover every "action" row above, including marks, three-point
edits, cut/delete/extract, resize-to-cursor, remove gap, tools and edit modes,
selection, keyframe navigation, and undo/redo. Roll/slide tools, dialog actions
(save, render, marker editing, insert space) and tool-aware frame steps stay
out. Until a Kdenlive with MR1a runs, the daemon reports unlisted actions once
as not offered and types nothing. `control-surfaced list-capabilities` shows
what a running Kdenlive offers.

---

## 4. Revision 2 wire contract (verbatim mirror of MAIN's files)

Sections 4.1–4.7 are `k23-contract-revised.md` (`39774d0c…6e8`) after its
title and status paragraph; 4.8 is the addendum (`b2121cc0…de2`), 4.9 the
MR1a action contract (`1e45ae44…15be`) and 4.10 its clarifications
(`ccb3d676…f4f2`), each in full. Only heading levels and numbers differ from
MAIN's files.

### 4.1 Availability and authorization

- Session bus only, existing `org.kde.kdenlive-<pid>` service, object
  `/ControlSurface`, interface `org.kde.kdenlive.ControlSurface1`.
- Compiled only with `USE_DBUS`. A user-visible setting defaults **OFF**.
  When disabled, the object is absent. No daemon call can enable it.
- An explicit `QDBusAbstractAdaptor` exports only the methods below.
- `Subscribe` grants a lease bound to the caller's unique bus name. Every
  mutation requires that lease. Losing the bus name or unsubscribing drops
  queued work and ends its already-applied gesture. A lease is not transferable.
- Application-inactive, modal, recovery, closing and conflicting-edit states
  refuse mutation. A call must pass admission and dispatch checks.
- Unavailable-interface keyboard fallback remains the daemon's policy.
  **Domain refusals and timeouts never authorize keyboard fallback.**

### 4.2 Wire signatures

All `a{sv}` replies are envelopes described below. `t` means unsigned 64-bit.

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

Signals (destination-addressed to the relevant subscriber, not broadcast):

| Signal | Arguments |
|---|---|
| `ContextChanged` | `a{sv} context` |
| `ControlAck` | `t seq, s control, a{sv} outcome` |
| `ActionFinished` | `t requestId, a{sv} outcome` |
| `ActionsChanged` | none |

The additional `ActionFinished` signal separates acceptance from invocation.
It does **not** claim completion of the work an action starts.

### 4.3 Envelopes, leases and context

Success: `{ok: true, result: a{sv}}`.
Failure: `{ok: false, error: {code: s, message: s, field: s}}`.
No `false`-means-fallback convention. D-Bus invalid signature/type errors remain
transport errors; semantic/type validation inside variant maps is structured.

`Capabilities.result`:
`{version: u(1), revision: u(2), implementation: s, controls: as, commands: as,
contextKeys: as, limits: a{sv}}`.
Optional `controlDescriptors: aa{sv}` describes units/axes/absolute support.
Clients must not send controls absent from `controls`. Read-only unknown result
keys may be ignored; unknown behavior-changing input options are rejected.
Version 1 with revision **at least 2** is compatible. Older/other major versions
are treated as unavailable during capability negotiation, before any mutation
is sent. This permits configured stock-editor keyboard fallback only at that
preflight boundary, never after an accepted call, timeout or domain refusal.

`Subscribe.result`: `{session: s, context: a{sv}}`. Repeated subscription from
the same sender is idempotent. `GetContext.result` is the context itself.
`Unsubscribe.result`: `{unsubscribed: true}`.

Required context keys:

- `serial: t`: observational snapshot sequence; may change with playhead state.
- `epoch: t`: target/context generation, independent of playhead ticking.
- `ready: b`, `active: b`, `dialog: b`, `focus: s`.
- `project: s`, `sequence: s` (empty if unavailable).
- `activeMonitor: s` (`clip`, `project`, `none`), `position: i`,
  `fps: {num: i, den: i}`, `playing: b`, `speed: d`, `tool: s`.

Later context is additive: `timeline`, `effect`, `param`, `colorWheel`,
`colorWheels`, `hoveredColorWheel`. `colorWheels` is an array of the three explicit
wheel target descriptors for the focused Lift/Gamma/Gain widget, allowing three
knobs without focus changes. A mutable target is an opaque host-issued `target: s`;
labels, visual indices and asset names are not identities. Parameters describe
their actual displayed units/range/step and editing policy. Keyboard focus is
distinct from hover. Hover never retargets an existing gesture.

Common mutation options: `session: s`, `epoch: t`. Editing controls also carry
`target: s` and a nonempty caller-selected `gesture: s`. `phase` defaults to
`update` and may be `end` or `cancel`. An explicit end with zero delta is still
a barrier and must be delivered. The captured target and edit frame do not move
within a gesture. Context-changing discrete operations terminate editing
gestures before acting.

There is one active **editing** gesture per host/document, across parameter,
wheel, gain and trim controls (`limits.editingWriters = 1`). Another caller's
editing mutation receives `busy`; it does not terminate or join the owner's
gesture. Observation and transport remain available. Unrelated undo history
intentionally increments `epoch` and invalidates queued edits; updates belonging
to the current gesture do not increment it.

After end, idle, context change or history change, reusing the caller's gesture
ID in a later update starts a **new host generation** and undo entry. It is not
an idempotent replay; fresh IDs are recommended. Per-lease sequence ordering
still applies. A cancel without a matching live owned gesture is
`history_conflict`, including a late cancel after end/idle/history changes.

Context signals are capped at **30 per second** using monotonic timing
(minimum 34 ms spacing is acceptable). No polling/context timer remains when
there are no subscribers. Signals go only to current subscribers; an
unsubscribed caller must receive none even if another caller stays subscribed.
Signals include optional `emittedAtMs: t` (process-relative monotonic time) for
measuring emission spacing; delivery can bunch when a client is busy.

### 4.4 Actions

`ListActions.result`: `{actions: aa{sv}}`, each containing
`{id, text, enabled, checkable, checked, shortcut}`. This is a curated allowlist,
not all actions registered in the application.

`TriggerAction.result`: `{state: "accepted", requestId: t, id: s}`.
The queued action is revalidated before invocation. Its directed
`ActionFinished` reports `{ok:true,result:{state:"invoked",id:s}}` or an error.
Interactive or long-running actions are not automatically exposed.

`Notify` requires common options, text at most 256 characters, and a timeout
between 500 and 5000 ms. It returns `{shown:true}` and never changes the project.

### 4.5 Continuous controls and acknowledgements

- Sequence numbers are positive, strictly increasing per lease; never reset
  within a lease. New leases start a new sequence space.
- Keys include sender, lease, epoch, gesture, target, control and validated
  semantic options. Different callers/targets are never merged.
- Each applied coalesced batch acknowledges its highest sequence. The outcome
  envelope also carries `session: s` for lease correlation, including errors.
  The outcome
  result includes `state: "applied"`, `firstSeq`, `lastSeq`, `epoch`, `gesture`,
  `target`, `changed`, and control-specific state. A refusal is also acked.
- Duplicate/old sequences are not applied again; they receive an error with
  code `stale_sequence`. The daemon correlates acks by lease/sequence and target,
  not just control name. A late ack cannot release a newer batch.
- Relative deltas sum before a single clamped apply: this is **net-batch**
  behavior, not a promise of detent-by-detent clamping at boundaries.
- Reject NaN/infinity, oversized messages, unknown controls/options and
  out-of-range numeric arguments before queuing.
- Oversized text, identifiers, option counts or aggregate string input use
  `resource_limit`. Malformed types, nonfinite numbers and numeric values outside
  the declared range use `invalid_arguments`.
- Defaults: at most 8 leases, 64 pending keys, 32 queued actions, 32 options,
  4096 characters of aggregate string input per call; bounded deltas are
  advertised in capabilities. Queues reject excess with `resource_limit`.
- Applied view state is acknowledged without waiting for decoded/displayed
  frames. No waits, worker joins or nested event loops in the adaptor.
- Focus/project/sequence/owner changes invalidate pending work. Dropped batches
  receive `stale_context` while their sender remains subscribed.

### 4.6 Staged controls

#### MR1: view and transport

| ID | Relative unit | Options beyond common options |
|---|---|---|
| `playhead.jog` | integral frames | `monitor: active/clip/project`, `scrub: b` |
| `playhead.shuttle` | integral index steps | `monitor: active/clip/project` |
| `timeline.zoom` | integral steps, positive zooms in | `anchor: playhead/mouse` |

Shuttle absolute values are indices -7 through 7. Magnitudes map to
`0,1,2,4,5,8,16,60`; zero pauses explicitly. `SetControlValue` supports shuttle.
Jog is transport-only even in Slip mode. It uses requested monitor position
and never calls tool-aware JogShuttle slots that edit clips. It does not change
the global audio-scrubbing preference. Zoom reuses the native zoom-level mapping.

#### MR2: parameter and color-wheel editing

- `param.focus`: integral navigation through supported visible parameter
  bindings; common options. Does not change data.
- `param.nudge`: steps in advertised display units; `step: normal/fine`,
  `keyframe: existing/create` (default `existing`).
- `colorwheel.nudge`: `wheel: lift/gamma/gain`, `axis: value/r/g/b`;
  optional hue/saturation axes only if advertised. Steps and actual ranges are
  described in context. Value is the wheel's HSV-value axis, not perceptual luma.
- `Invoke("param.reset", args)` and `Invoke("colorwheel.reset", args)` use the
  same explicit target/keyframe policy and native defaults.

No implicit effect creation, no first-matching-effect fallback and no implicit
keyframe insertion. Static/single-key values update in place. Multi-key
animation needs a key at the captured frame, or explicit `create`. Managed
automation and unsupported value types are refused. Hover targeting must be
explicitly selected by the client from the published hovered target.
The `wheel` option may be omitted when the target is a wheel handle. If supplied,
it must match that handle; it never redirects the target.

During playback, live grading is supported for static or single-key parameters
only (`liveGrading: true` in the target descriptor). It adjusts that existing
whole-clip value, with one gesture regardless of advancing playback frames.
Clock ticks alone do not end this gesture. Multi-key edits and explicit key
creation require stopped playback and return `busy` while playing; silently
writing a different frame each tick would violate the frozen-key policy and
would amount to an unimplemented automation-recording feature. When stopped,
seeking during a multi-key edit ends its gesture.

One owned undo entry/closed command group per gesture, including intermediate
updates. There is no globally open undo macro. Merge only matching gesture,
document, target, component set and edit frame. End after 600 ms idle, explicit
end, target/focus change, unrelated history, save or undo/redo. No-op gestures
make no history. Cancel restores the gesture only while its undo/state is still
owned; otherwise report `history_conflict`, never undo unrelated work.

#### MR3: timeline and audio

Target locations in `GetContext.result`:

| Operation | Handle and scope |
|---|---|
| `track.set` | `timeline.track.target`; `id` and `sequence` identify the native track |
| Track `audio.gain` | `timeline.track.gain.target` (same handle as `timeline.track.target`), present only for an available audio mixer |
| Clip `audio.gain` | `timeline.clipGain.target`; includes `clip`, `unit: dB`, and `policy: single_keyframe_existing_effect` |
| `edit.trim` | `timeline.trim.target`; `clip`, `clips`, `tracks`, and `modes` declare the complete selected edit scope |

The initial trim mode resizes only the declared clip(s). It does not move any
downstream clips or ripple tracks; ripple scope is empty. No client-supplied
track list can broaden this scope.

- `timeline.track`: integral steps in visual track order; context reports the
  native ID and sequence identity, not an A1/V1 label as identity.
- `timeline.scroll`: tenths of visible width.
- `audio.gain`: 0.1 dB per step, explicit target handle for clip/track gain;
  no automatic effect creation and no ambiguous duplicate-volume selection.
- `edit.trim`: integral frames with explicit `edge: start/end`,
  `mode: resize/ripple/roll/slip/slide`, and a host-issued edit target.
  Only qualified modes are advertised. The captured native group/ripple scope
  must be explicit in the context; no undeclared target-track edits.
  The initial capability is resize only, for a single clip or aligned linked
  A/V pair without mixes, composition-bearing tracks, retiming or managed
  automation. `mode` is required. `limits.trimGestureSteps` bounds retained
  coalesced resize steps (initially 128); end before starting a new gesture.
- `Invoke("track.set", args)`: explicit target, `what:
  mute/hide/lock/solo/target`, `value: b`, and `soloMode: exclusive/additive`.
  Applicability is checked by track type. Solo retains prior manual mute state.

Unsupported or not-yet-qualified controls return `unsupported_control` or
`unsupported_mode`; they are never silently mapped to a different operation.

### 4.7 Error codes and qualification

Core codes: `invalid_arguments`, `not_subscribed`, `stale_context`,
`stale_sequence`, `inactive`, `modal`, `not_ready`, `closing`, `busy`,
`resource_limit`, `unknown_action`, `action_disabled`, `unsupported_control`,
`unsupported_mode`, `target_not_found`, `unsupported_parameter`, `keyframe_required`,
`managed_parameter`, `unsupported_group`, `track_locked`, `history_conflict`, `edit_failed`.

The daemon must retain sequence/target correlation and distinguish unavailable
interfaces from domain errors. Tests use generated media and a private real
session bus, including multiple subscribers and ownership loss. Mock latency
measurements alone do not establish native editing correctness or latency.
The optional automation bridge and all MCP additions are deferred.

### 4.8 Addendum: K23 revision 2: eight wire clarifications

This addendum records the behavior of the frozen, accepted MR3 app
`7a0f189da86538c1fccfe530fc95900fd94c635a550b15be3d6a9747598f4848`.
It supplements `k23-contract-revised.md`, SHA-256
`39774d0c106e8c215b0c70a4e95d94a52ac9ecda34686bae78c3e5033514d6e8`.
No wire behavior or revision changes. The base file stays byte-identical during
exclusive lease `k23-keypad-sim-20261007`; do not substitute a new pin mid-lease.

1. **Transport and navigation options.** Yes: `playhead.jog`,
   `playhead.shuttle`, `timeline.zoom` and `param.focus` reject `target`,
   `gesture` and `phase`, including empty values. The result is
   `invalid_arguments`, with `error.field` naming the unknown option.
   Common options are `session` and `epoch`. Additional options are
   jog=`monitor,scrub`, shuttle=`monitor`, zoom=`anchor`, param.focus=none.
   `timeline.track` and `timeline.scroll` likewise accept only common options.

2. **Oversized input.** Oversized text, identifiers, option count and aggregate
   string input return `resource_limit`, not `invalid_arguments`.
   Queue/subscriber limits and accumulated-delta overflow also use
   `resource_limit`. Wrong types, NaN/infinity and an individual numeric value
   outside its declared range use `invalid_arguments`. For a multiply-invalid
   request, do not infer a universal error-priority rule.

3. **Unknown commands.** There is no `unsupported_command` code. An unknown
   `Invoke` command with otherwise valid common options returns
   `unsupported_control`. In the frozen host its `error.field` is the empty
   string, **not** `"command"`. Option validation precedes unknown-command
   dispatch, so extra unrecognized options can instead produce
   `invalid_arguments`. The mock must not invent a nonempty field.

4. **Exact capability keys.** The current `Capabilities.result.limits` map is:

   ```json
   {
     "subscriptions": 8,
     "pendingKeys": 64,
     "queuedActions": 32,
     "contextHz": 30,
     "maximumDelta": 10000,
     "trimGestureSteps": 128,
     "editingWriters": 1
   }
   ```

   Each value is a D-Bus signed 32-bit integer. `version` and `revision` are
   unsigned 32-bit integers. `controlDescriptors` is optional and is **absent**
   in the current host; there are no advertised nested key names to emulate.
   Do not require it. Use the advertised controls/commands and actual context
   target descriptors. Top-level `trimModes` is the string array `["resize"]`.
   The documented32-option/4096-character bounds are not additional keys in
   this capability map.

5. **Audio and trim targets.** These are paths within `GetContext.result`:
   track gain=`timeline.track.gain.target` (same opaque handle as
   `timeline.track.target`); clip gain=`timeline.clipGain.target`;
   trim=`timeline.trim.target`. There is no `audio.target` or `edit.target`.
   Missing descriptors mean the target is unavailable. Trim supplies `clip`,
   `clips`, `tracks` and `modes`, defining the complete selected scope.
   Initial support is resize of a single clip or qualified aligned linked A/V
   pair. No downstream ripple, no implicit additional tracks and no
   client-supplied scope expansion. Ripple scope is semantically empty; there
   is no separate `rippleScope` field in the frozen context. Unsupported modes
   are refused, not mapped to resize.

6. **Nonzero end delta.** Yes. An editing `Control` with `phase:"end"` may
   carry a valid nonzero delta: the host applies that delta, then ends the
   gesture. It first flushes earlier queued work and treats end as a distinct
   barrier. A zero-delta end is also a barrier and must not be discarded.
   The same target/epoch/history/ownership and numeric checks still apply.

7. **Epoch invalidation.** Correct: do not send an end for the invalidated
   gesture. The host ends its editing ownership when the context/history
   generation changes; already-applied changes remain as completed undo
   history, not an automatic rollback. Discard unsent old-generation updates
   and prevent late acknowledgements from completing a newer gesture.
   Old-epoch mutations receive `stale_context`. A subsequent gesture requires
   fresh context/targets; a late cancel cannot undo unrelated history.

8. **Revision negotiation.** Version1 with revision>=2 is accepted.
   Older revisions or another major version are unavailable during initial
   capability negotiation, before mutations. Configured keyboard fallback
   is allowed only at that preflight boundary, never following an accepted
   call, timeout, domain refusal or late acknowledgement. During this lease
   fallback is observed through a non-emitting key sink, not global input.

Source evidence: `src/controlsurface/controlsurface.cpp` options/admission
at236-293, epoch termination at214-217, capabilities at409-421, barriers at483-513,
and Invoke at528-532; `parametercontrol.cpp:451-470` applies before end;
`timelinecontrol.cpp:312-430` provides the equivalent gain/trim behavior and
`:240-283` builds the native target paths. The base contract already contains
the oversize, target-scope and revision policy; this addendum makes the exact
frozen serialization and boundary behavior explicit.

### 4.9 K23 MR1a: curated editing actions and transport admission

This additive revision-2 contract supplements `k23-contract-revised.md`
(SHA-256 `39774d0c106e8c215b0c70a4e95d94a52ac9ecda34686bae78c3e5033514d6e8`)
and `k23-contract-rev2-wire-clarifications.md`
(`b2121cc0d8cddc642498c10e482eb533ec6db1637744c8ceee06469c194d6de2`).
No method signature, error envelope, version negotiation or keyboard-fallback
policy changes. Implementation identifies itself as `Kdenlive K23 MR1a`.

#### Discovery and execution

`ListActions` now has71 curated candidate IDs. It returns only IDs actually
registered by this build; conditional category-marker actions may be absent.
Do not hardcode a returned count. Each entry retains
`{id,text,enabled,checkable,checked,shortcut}`.

`ActionsChanged` is emitted when the listed action metadata changes, including
enabled/checked state, not only when IDs appear/disappear. Refresh ListActions
on this signal: initial undo/redo availability is not permanent.

TriggerAction still returns `accepted` and a requestId; its directed
ActionFinished reports `invoked` or a structured error after queued revalidation.
Invoked means the native action callback ran, not that it necessarily changed
data or that deferred work finished. Existing native undo grouping is retained;
no open macro spans event-loop work. Native mark-out retains its usual exclusive
end: pressing it at frame1040 stores zoneOut1041.

Editing actions reject another caller's active edit gesture with `busy`.
They also reject a native drag in progress. Context and QAction availability
are rechecked at dispatch, including action deletion/replacement.

#### Exact candidate inventory

| Area | IDs |
|---|---|
| Play, pause, shuttle actions | `monitor_play`, `monitor_pause`, `monitor_seek_backward`, `monitor_seek_forward` |
| Zone/clip playback | `monitor_play_zone`, `monitor_play_zone_cursor`, `monitor_loop_zone`, `monitor_loop_clip` |
| Monitor switching/zoom | `switch_monitor`, `monitor_zoomin`, `monitor_zoomout`, `monitor_zoomreset` |
| Timeline zoom | `zoom_fit`, `view_zoom_in`, `view_zoom_out` |
| Boundaries | `seek_start`, `seek_end`, `seek_clip_start`, `seek_clip_end`, `seek_zone_start`, `seek_zone_end` |
| Edit/marker navigation | `monitor_seek_snap_backward`, `monitor_seek_snap_forward`, `monitor_seek_guide_backward`, `monitor_seek_guide_forward` |
| In/out | `mark_in`, `mark_out` |
| Quick/category markers | `add_marker_guide_quickly`, `add_marker_guide_1` through `add_marker_guide_10` |
| Marker deletion | `delete_clip_marker`, `delete_sequence_marker` |
| Three-point editing | `insert_to_in_point`, `overwrite_to_in_point`, `remove_lift`, `remove_extract` |
| Cut, lift/delete, extract/ripple | `cut_timeline_clip`, `cut_timeline_all_clips`, `delete_timeline_clip`, `extract_clip` |
| Resize to cursor | `resize_timeline_clip_start`, `resize_timeline_clip_end` |
| Remove gap | `delete_space`, `delete_space_all_tracks` |
| Tools | `select_tool`, `razor_tool`, `spacer_tool`, `ripple_tool`, `slip_tool` |
| Native edit modes | `normal_mode`, `overwrite_mode`, `insert_mode` |
| Selection | `select_timeline_clip`, `deselect_timeline_clip`, `select_add_timeline_clip`, `select_timeline_zone`, `select_track`, `select_all_tracks` |
| Native keyframes | `keyframe_add`, `keyframe_next`, `keyframe_previous` |
| History | `edit_undo`, `edit_redo` |

All action forms in the current keypad example config are covered by these
candidate IDs. Existing continuous controls and typed track/gain/trim commands
remain available; this is not a new editing engine.

#### Context restrictions

- `delete_timeline_clip` requires focus inside the current timeline.
  Otherwise it returns `target_not_found`: the native QAction is focus-sensitive
  and could instead delete a bin clip or an effect. No silent redirection.
- Source-zone insert/overwrite requires an open clip-monitor source and a
  compatible timeline target. Missing source/targets return `target_not_found`.
- Quick markers require a clip or native guide context; a selected composition
  is not treated as a clip marker target.
- The descriptor's `enabled` combines native QAction state with host context
  restrictions. Caller-specific writer ownership is checked when invoked.
- Explicit `keyframe_add` retains native add/remove behavior. It is not an
  implicit keyframe insertion during a parameter gesture; managed parameters
  retain their existing native ownership protection.

#### Shuttle and pause

The real-daemon S1 attempt was made while `tool:"slip"` and the native monitor
trimming preview was active. Native forward/rewind intentionally refuse that
preview. This is not an editing-writer conflict.

Nonzero shuttle and playback-start actions return `busy` in that state.
Use `select_tool`, reacquire the current epoch, then shuttle normally.
Zero shuttle and `monitor_pause` still pause; they do not edit a clip.
Pure `playhead.jog` remains transport-only even in Slip mode.

The host now prevents a refused Play toggle from leaving `playing:true` while
the renderer is stopped. Pause is not skipped merely because the Play action's
cached state is false, and stopping is allowed during trimming. Nonzero
requestShuttle reports actual native playback-start failure instead of blindly
reporting success after a void slot call.

#### Deliberate exclusions

No dialog-opening actions: file open/save/save-as (save can prompt on first save
or I/O problems), interactive marker editing, track creation/deletion,
duration/speed dialogs, replacement confirmations, render dialogs/jobs and
insert-space dialogs. They are absent and return `unknown_action`, not a
keyboard-fallback invitation. Existing guarded save/job APIs remain separate.

Tool-aware frame/second QAction steps are excluded because they can edit Slip
clips; use `playhead.jog` with an explicit frame delta instead. Roll/slide tools
not registered by this native build are not fabricated. There is no invented
clear-zone or next/previous-selected-clip action: use advertised boundary/snap
navigation and explicit selection operations.

Track target/mute/solo/lock/hide and faders remain the typed MR3 operations.
Effect/wheel parameter editing stays MR2. An effect-enable/compare command
without a reviewed native action is not invented. Optional automation-editor
point editing and loop/reverse/AI job bridges remain deferred.

### 4.10 K23 MR1a mock clarifications E-H

This clarifies the frozen MR1a product behavior, without changing its wire
revision or the action addendum `1e45ae44127945221a4690b13cff4662c2f510469b9abeb80c752f6c5b0c15be`.

**E. Editing-action set.** Exactly the following30 candidate IDs participate
in the editing-writer and native-drag checks:

```
mark_in mark_out add_marker_guide_quickly
add_marker_guide_1 add_marker_guide_2 add_marker_guide_3
add_marker_guide_4 add_marker_guide_5 add_marker_guide_6
add_marker_guide_7 add_marker_guide_8 add_marker_guide_9
add_marker_guide_10 delete_clip_marker delete_sequence_marker
insert_to_in_point overwrite_to_in_point remove_lift remove_extract
extract_clip cut_timeline_clip cut_timeline_all_clips delete_timeline_clip
resize_timeline_clip_start resize_timeline_clip_end
delete_space delete_space_all_tracks keyframe_add edit_undo edit_redo
```

There is no per-descriptor `editing` flag in this version. An action can
legitimately make no change; membership is not inferred from a history delta.
Other native operations retain their existing behavior.

**F. Slip/trimming and enabled.** Yes, the actual host includes the trimming
restriction in `ListActions.enabled`. For the active project monitor in
trimming preview, these playback actions are listed but disabled:
`monitor_play`, `monitor_play_zone`, `monitor_play_zone_cursor`,
`monitor_loop_zone`, `monitor_loop_clip`, `monitor_seek_backward`,
`monitor_seek_forward`. Calling one still returns structured `busy`
(unless an earlier admission check rejects it). Pause remains available.
The guard is the actual native trimming-preview state, not only a tool string.
The enabled/checked change triggers ActionsChanged. Caller-specific writer
ownership is not encoded in the shared descriptor and is checked on invocation.

**G. Synchronous context restriction.** Yes: TriggerAction checks context
restrictions before acceptance and rechecks them at dispatch. Failure at
admission returns the error directly, with no accepted request. Failure after
acceptance arrives through the directed ActionFinished. There is no fallback
permission in either case.

**H. Action history and epoch.** Yes: a discrete action's native undo/history
change advances epoch through the document history observer. It is not treated
as another update in an owned continuous parameter/gain/trim gesture.
TriggerAction flushes and ends editing gestures before dispatch. Clients must
reacquire context and fresh gesture state after the action, not reuse the old
epoch. No-op/non-history actions need not advance epoch unless they otherwise
change the context fence.

---

## 5. How the daemon uses it (client policy)

### 5.1 States and the API-only default

The client walks `Capabilities` → `Subscribe` → `ListActions` and lands in one
of four states:

| State | When | Pad behaviour in a Kdenlive window |
|---|---|---|
| Available | Version 1, revision ≥ 2, subscribed | Advertised controls, commands and allowlisted actions only. Anything else is reported once and **not** typed. |
| Absent | `ServiceUnknown`, `NameHasNoOwner`, `UnknownObject`, `UnknownInterface` or `UnknownMethod`; or an incompatible version/revision at negotiation | **API only (default):** nothing is sent or typed; one notice per attachment ("Kdenlive control interface not enabled …") in the log and as a desktop notification. Only a profile with `"keyFallback": true` types the configured stock shortcuts. |
| Pending | Timeout, other transport error, a refused `Subscribe`, or an incompatible answer after this daemon already sent that instance a mutation | Nothing is sent and nothing is typed. Retried on input or refocus (Pending after 2 s, Absent after 5 s). |
| Detached | No Kdenlive window focused | Other profiles apply (uinput keys as configured). |

Kdenlive's interface defaults **off**, so the notice is what a user sees until
they enable it. The user's direction is that every Kdenlive binding is an API
call; keystrokes into Kdenlive are an explicit per-profile opt-in, never a
default. Plain `"keys"` bindings are typed as written, because the user wrote
them; the shipped Kdenlive profile has none.

A domain refusal (`ok:false`, in `TriggerAction` or `ActionFinished`,
`Invoke`, or `ControlAck`) is reported, never typed. If an action call proves
the interface vanished, the stock key is typed only with `keyFallback` and
only into the same Kdenlive window (same pid and address) that was asked;
without `keyFallback` the user gets the notice.

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
* `ActionsChanged` (MR1a: also for `enabled`/`checked` changes, e.g. undo
  availability or a focus change) triggers a `ListActions` refresh. At most
  one refresh is in flight per lease; signals that arrive meanwhile collapse
  into one more refresh. The client keeps each action's `enabled`, but it does
  not gate on it: the host revalidates every `TriggerAction` at dispatch, and
  a refusal (`target_not_found`, `action_disabled`, `busy`) is reported, never
  typed.
* A `busy` refusal of `playhead.shuttle` or a playback action adds a hint to
  switch to the Selection tool when the host lists `monitor_play` disabled
  (its trimming-preview signal, §4.10 F) or the context's `tool` is `slip`.
  The trimming preview refuses playback; it is not a writer conflict.

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
  * on success, it echoes the `target` that was sent.

  It settles every message ≤ `seq` under that key. It releases the key only if
  `seq` is the key's newest message, so a late ack cannot release a newer
  batch. Unknown, duplicate, foreign and wrong-target acks release nothing.
* Coalescing keys:
  * transport: `control|options`;
  * editing: `slot|control|options|gesture|target`.

  At most one message is in flight per key, sent no more often than every 8 ms,
  with a 60 ms ack timeout and at most 64 live keys.
* **Navigation waits for its own epoch.** A `timeline.track` or `param.focus`
  ack with `changed: true` moves the host's target, so the host bumps `epoch`.
  Further detents of that knob are held until the new epoch arrives (250 ms
  fallback) and are then sent with it. Without this, a fast multi-detent turn
  was refused with `stale_context` in the real-editor acceptance.

### 5.3 Gestures and targets

* Editing controls take their target from the context (qualified paths, §4.8
  item 5):
  * `param.nudge`/`param.reset` → `param.target`
  * `colorwheel.*` with a `wheel` option → the `colorWheels[]` entry of that
    wheel; otherwise `colorWheel.target`, but only if the focused wheel is
    that wheel (a descriptor without a `wheel` kind is taken as is)
  * `audio.gain` → `timeline.clipGain.target`; track gain with
    `"targetFrom": "timeline.track.gain.target"`
  * `edit.trim` → `timeline.trim.target`, always with explicit `edge` and
    `mode` (only advertised `trimModes`; `ripple` is refused by the host, never
    mapped to resize)
  * `track.set` → `timeline.track.target`

  A binding may opt into hover with `"targetFrom": "hoveredColorWheel.target"`;
  a hovered wheel of another kind than the binding's `wheel` yields no target.
  With no target, nothing is sent. The `wheel` option is still sent as a
  cross-check (Kdenlive refuses a contradiction with `unsupported_parameter`).
* A gesture (`cs-<pid>-<n>`) is scoped to one binding (slot, control and
  options), one target and one epoch. An open gesture keeps the target it
  captured; only a new gesture reads the target from the context, so a moving
  hover never retargets a turn in progress. It ends with an explicit `phase:"end"`
  barrier in these cases:
  * 500 ms idle (`gestureIdleMs`; the host ends at 600 ms);
  * a mode cycle;
  * before any discrete action or `Invoke`;
  * a change of target or binding options;
  * before **another binding starts a gesture** (one editing writer per host,
    so turning the gamma knob ends the lift knob's gesture: one undo entry
    each);
  * before any **non-editing control** (jog, shuttle, zoom, scroll,
    `timeline.track`, `param.focus`), which Kdenlive would end it for anyway;
  * `edit.trim` after `limits.trimGestureSteps − 1` batches (the next batch
    starts a new gesture).
* No end is sent for a gesture the host already ended: on an `epoch` change,
  or a `stale_context`/`target_not_found` refusal, queued motion is dropped
  without being sent (§4.8 item 7). A frame-bound gesture (a multi-key edit,
  `liveGrading: false`) is also forgotten without an end when `position`
  moves, because the host ended it on the seek.
* `busy` (another editing writer, or a multi-key edit while playing),
  `unsupported_group`, `history_conflict` and every other refusal are reported
  and never typed.
* The end barrier is sent immediately, even with zero delta. Ordering after the
  in-flight update is guaranteed by D-Bus per-sender ordering.
* A window change drops all pending work and unsubscribes.
* Context pacing: the client keeps `contextTiming()` per lease: signals
  received, signals stamped with `emittedAtMs`, the smallest `emittedAtMs`
  spacing and the smallest arrival spacing. Only the former measures the host's
  limiter; `bench-dbus` prints both. Snapshots from `Subscribe` are not
  counted. Nothing in the daemon is paced by context arrival: control pacing
  is ack-driven (one message in flight per key).

### 5.4 Per-context layers (data/config.example.jsonc)

Every Kdenlive binding is an API call. The first matching layer that binds a
slot wins, then the profile's own bindings (the default fallback):

| Layer | When | knob 1 | knob 2 | knob 3 |
|---|---|---|---|---|
| `color-wheels` | `colorWheels` non-empty | lift (`colorwheel.nudge`, own handle); hold+turn fine; press cycles value→R→G→B | gamma, same | gain, same |
| `effect-parameter` | focus `effectStack` and a `param.target` | `param.nudge` (step and keyframe modes); hold+turn fine; press toggles step | `playhead.jog`; press play | `param.focus` (parameter scroll) |
| `track-video`, `track-mixer` | page `track` (key 13) | `timeline.track` | track gain (`audio.gain` on `timeline.track.gain.target`) | clip gain (`timeline.clipGain.target`) |
| `trim` | page `trim` (key 13) | `playhead.jog` | `edit.trim` start edge, resize | `edit.trim` end edge, resize |
| `clip-monitor`, `project-monitor` | focus of that monitor | `playhead.jog`; press play | `playhead.shuttle`; press pause | jog ×10 frames; press `switch_monitor` |
| `timeline` | focus `timeline` | `playhead.jog`; hold+turn `timeline.scroll`; press play | `timeline.zoom`; press `zoom_fit` | `timeline.track`; press `switch_monitor` |
| (profile) | anything else | as `timeline` | | |

Keys: wheel resets (6–8) in the wheel layer; keyframe actions, the keyframe
mode and `param.reset` in the parameter layer; mute/hide, solo (exclusive),
lock and target toggles of the focused track on the track page (from the
state Kdenlive publishes, `$!ctx:` paths); on the trim page, resize the
selection's start/end to the playhead, remove the gap and extract; in the
monitors, play the zone, loop the clip (clip monitor) or zone (project
monitor), and jump to the zone's start/end (keys 6–9); curated actions
otherwise, with key 13 cycling the page (edit → track → trim). Every action the
default config binds is an MR1a candidate (`tst_kdenlive_dbus
mr1aActionsInventoryAndRestrictions` checks this).

Configuration features (all tested):

* slots `turn`, `ccw`/`cw`, `press`, and `shift.turn|ccw|cw` for turning a
  held knob (its `press` then fires on release, only if it did not turn);
* per-binding `scale` and `accel` (overrides `settings.accelFactor`; 1 turns
  acceleration off);
* `when` on context paths, `/regex/`, lists, `!value`, and `$mode.<name>`;
* validation with errors that name the place (`profile kdenlive layer trim
  knob2.turn: …`) for references that can never work, and warnings for
  unknown control/command names and similar (`check-config`);
* hot reload: the daemon watches the config file (and its directory, for
  editors that save by rename) and the learned hardware map; a valid edit
  replaces the config after 300 ms, an invalid one is reported (log and
  desktop notification) and the running config stays;
* `control-surfaced list-capabilities [--json] [--kdenlive-service NAME]`:
  read-only `Capabilities`, `ListActions` and `GetContext` (no lease) for every
  `org.kde.kdenlive-<pid>`; prints controls, commands, limits, actions,
  current focus and editing handles, every context path a `when` can test,
  which configured layers apply now, and which configured names this Kdenlive
  does not offer.

---

## 6. Mock conformance (src/mock)

`MockKdenlive` exports only the contract members through an explicit
`QDBusAbstractAdaptor` (`ExportAdaptors`); test helpers are not exported. It
follows the qualified host where the text leaves room, and the addendum (§4.8)
exactly:

* sender-bound leases (owner loss via `QDBusServiceWatcher`), with the lease
  echoed in every ControlAck outcome; a `Control` from a caller with no lease
  is dropped silently, with no signal of any kind;
* admission order: options/size → lease → unknown options → epoch → ready,
  closing, active, modal. Ready, closing, active and modal are checked again
  at dispatch, for both control batches and queued actions;
* per-control option sets as in §4.8 item 1: transport and navigation
  controls reject `target`/`gesture`/`phase` (`invalid_arguments`, `field`
  names the option); `param.focus`, `timeline.track` and `timeline.scroll`
  accept only common options;
* `resource_limit` for oversized option counts, identifiers, text and
  aggregate string input, queues and leases; `invalid_arguments` for wrong
  types, non-finite numbers and values outside a declared range;
* `Invoke` validates options before dispatch; an unknown command with valid
  options is `unsupported_control` with an **empty** `field`;
* `Capabilities`: version/revision `u`, the exact `limits` map of §4.8 item 4
  (int32 values including `trimGestureSteps` 128 and `editingWriters` 1),
  `trimModes: ["resize"]`, advertised `contextKeys` per stage, and **no**
  `controlDescriptors`;
* finite deltas within `maximumDelta` (10000 relative; `param.focus` 64),
  integral except `param.nudge`/`colorwheel.nudge`/`audio.gain`;
* MR2 context as Kdenlive publishes it (`focusWheels`, `hoverWheel`,
  `focusParam`): `colorWheel`, the three `colorWheels` handles,
  `hoveredColorWheel`, `param` and `effect`; values refresh with a serial
  bump, never an epoch; live grading during playback for static/single-key
  values, `busy` for multi-key edits and key creation while playing;
* MR3 context with the qualified shapes: `timeline.track` (`target`, native
  `id`, `sequence`, `audio`, `name`, `locked`, `muted`, `hidden`, `targeted`,
  `solo`, `gain`), `timeline.selection`, `timeline.trim` (`target`, `clip`,
  `clips`, `tracks`, `modes`) and `timeline.clipGain` (`target`, `clip`,
  `unit: dB`, `policy`); resize-only trim (`ripple` → `unsupported_mode`,
  locked track → `track_locked`); `track.set` with solo retaining manual mute;
* wheel handle validation: unknown handle → `target_not_found`, contradicting
  `wheel` → `unsupported_parameter`, numeric axis other than `value` →
  `invalid_arguments`;
* `emittedAtMs` on every `ContextChanged`; `stale_sequence`; pending keys built
  from sender, session, epoch, gesture, target, control and options; one apply
  per event-loop pass, plus a configurable busy-GUI delay;
* directed `ControlAck`, `ContextChanged` and `ActionFinished` (targeted signals
  on the bus, connection-scoped on peers); a 34 ms monotonic context limiter
  that is idle without subscribers;
* staged capabilities (`--stage 1|2|3`) and `--off`, which owns the service
  but not the object; `--tick-ms N` simulates playback; console commands
  (`wheel`, `hover`, `param`, `keyframes`, `play`, `track`, `clip`, `grouped`,
  `history`, `tool`, `dialog`, `position`, `state`, …);
* one history entry per changed gesture, none for a no-op; a nonzero `end`
  delta applies, then ends (§4.8 item 6); a late cancel is `history_conflict`;
  reusing an ended gesture id starts a new generation; unrelated history ends
  open gestures and starts a new epoch;
* one editing writer host-wide (`busy` for another caller); gestures also end
  on any non-editing control and, for multi-key edits, on a seek; grouped
  propagation (console `grouped on`) → `unsupported_group`;
* `TriggerAction` accepted → revalidated → `ActionFinished` invoked or refused;
  transport-only jog (Slip mode leaves history untouched); shuttle indices with
  0 = pause.

MR1a (§4.9) in the mock:

* `ListActions` lists exactly the 71 candidates (approximate texts and
  shortcuts), with `checked` for the current tool and edit mode and `enabled`
  from the host context restrictions. `ActionsChanged` goes to every
  subscriber whenever an id, `enabled` or `checked` changes (coalesced per
  event-loop pass);
* context restrictions, checked at `TriggerAction` and again at dispatch:
  * `delete_timeline_clip` outside timeline focus → `target_not_found`;
  * insert/overwrite without a clip-monitor source (console `source off`) or
    without a target track → `target_not_found`;
  * markers in the clip monitor without a source → `target_not_found`;
  * undo/redo with nothing to undo/redo → `action_disabled`;
* exactly the 30 editing actions of §4.10 E (including `mark_in`/`mark_out`)
  are `busy` while another caller owns an editing gesture or a native drag
  runs (console `drag on`); ownership is not in the descriptor. Those that
  change history add one entry and start a new epoch (§4.10 H); marks only
  move the zone;
* trimming preview (a state of its own, entered with the Slip tool; console
  `trimming on|off`): on the project monitor, the 7 playback actions of
  §4.10 F are listed disabled (announced with `ActionsChanged`) and refused
  `busy`, as is nonzero shuttle (`Control` or `SetControlValue`), leaving
  `playing` unchanged; zero shuttle, `monitor_pause` and the clip monitor still
  work; `select_tool` leaves it (the tool is a target key, so a new epoch);
* `mark_out` at frame N stores zone out N + 1; dialog actions →
  `unknown_action`.

§4.10 settled the mock's earlier assumptions (§7 E–H).

Tests:

* `tests/tst_kdenlive_dbus.cpp` covers the wire on a peer connection, the
  engine → client → mock path, and `list-capabilities` (available, interface
  off, incompatible revision; no lease taken).
* `tests/tst_kdenlive_bus.cpp` runs under `dbus-run-session` on a private
  session bus: two or more subscribers, directed acks and context, sender
  separation, lease forgery, unsubscribe silence, owner loss, the ≤ 30 Hz
  limit, the lease limit, client ack correlation against a scripted server
  (late, foreign, duplicate and wrong-target acks; no sequence reuse across
  leases), absent versus timeout, and bunched arrival with every
  `emittedAtMs` spacing ≥ 34 ms.
* `control-surfaced bench-dbus N --kdenlive-service NAME` measures
  `Control`→`ControlAck` round trips against any implementation, then prints
  the context pacing. Mock numbers are transport evidence only.
* The real-editor acceptance against the qualified Kdenlive is recorded in
  `docs/E2E-ACCEPTANCE.md`.

---

## 7. Clarification requests and their status

All earlier requests are settled by the base text (`39774d0c…`) or the
addendum (`b2121cc0…`); the daemon and mock follow them.

| # | Question | Status |
|---|---|---|
| 1 | ControlAck outcomes carry `session`, also on errors | Answered (§4.5) |
| 2 | Correlating refusal acks | Answered (§4.5): lease/sequence and target |
| 3 | Editing options on non-editing controls | Answered (§4.8 item 1): rejected, `invalid_arguments`, `field` names the option |
| 4 | Error code for oversized input | Answered (§4.5, §4.8 item 2): `resource_limit`; malformed or out of range: `invalid_arguments` |
| 5 | Unknown `Invoke` command | Answered (§4.8 item 3): `unsupported_control`, empty `field`, options validated first |
| 6 | Names in `limits` / `controlDescriptors` | Answered (§4.8 item 4): exact int32 map; `controlDescriptors` absent |
| 7 | MR3 target paths and trim scope | Answered (§4.6 MR3, §4.8 item 5) |
| 8 | `end` with a nonzero delta | Answered (§4.8 item 6): applies, then ends |
| 9 | Explicit `end` after an epoch change | Answered (§4.8 item 7): none |
| 10 | Revision compatibility | Answered (§4.3, §4.8 item 8): version 1, revision ≥ 2; older is unavailable only before any mutation |
| 11 | `unsupported_group` code | Answered: core code (§4.7) |
| 12 | Late cancel | Answered: `history_conflict` (§4.3) |
| 13 | One active editing gesture per host | Answered: `limits.editingWriters = 1`, `busy` (§4.3) |
| 14 | Gesture ends during playback | Answered (§4.6 MR2): live grading is one gesture; multi-key edits and key creation need stopped playback |
| 15 | Reusing an ended gesture id | Answered: new host generation (§4.3) |
| 16 | Unrelated history and the epoch | Answered: it bumps the epoch (§4.3) |
| 17 | `wheel` with a wheel handle | Answered: optional; must match if supplied (§4.6 MR2) |

Still open, from the real-editor acceptance (none blocks the daemon):

* **A. Action allowlist.** Answered by MR1a (§4.9): 71 candidates covering
  every action the config binds. Awaiting a pinned lease to qualify it with
  this daemon.
* **B. Shuttle.** Explained by MR1a: S1 ran with `tool: "slip"`, whose native
  monitor trimming preview refuses forward/rewind (`busy`, not a writer
  conflict). `select_tool` plus a fresh epoch permits shuttle. The daemon now
  hints at this; to be re-run under a new lease.
* **C. Wheel descriptor `name`.** Each `colorWheels` entry reports `name:
  "lift_r"`, also for gamma and gain. It is cosmetic (the daemon uses
  `target` and `wheel`), but a per-wheel name would read better in tools.
* **D. `monitor_play`** toggled `playing` without advancing the playhead.
  MR1a fixes the host's fake `playing` after a refused Play (and pause now
  always pauses). To be re-run under a new lease.

MR1a points the mock had to assume, now answered by §4.10 (the mock follows):

| # | Question | Answer |
|---|---|---|
| E | Which actions are "editing actions" (writer and drag checks)? | Exactly 30 ids, including `mark_in`/`mark_out`; no descriptor flag |
| F | Is the trimming restriction in `enabled`? | Yes: the 7 playback actions are listed disabled and refused `busy`; the guard is the native trimming state on the project monitor, not the tool string |
| G | Context restrictions at `TriggerAction` too? | Yes: at admission (direct error) and at dispatch (`ActionFinished`) |
| H | Does an action's own history step bump `epoch`? | Yes, through the history observer; reacquire context afterwards |
