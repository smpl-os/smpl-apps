# End-to-end acceptance against the qualified Kdenlive (K23 MR1–MR3)

Date: 2026-10-07. MAIN granted an exclusive lease, `k23-keypad-sim-20261007`,
to run this daemon against its real, qualified Kdenlive. The lease has been
released, and MAIN and the coordinator have the evidence.

| Pin | Value |
|---|---|
| Kdenlive app | `7a0f189da86538c1fccfe530fc95900fd94c635a550b15be3d6a9747598f4848` (MAIN's private build) |
| Contract | `k23-contract-revised.md` `39774d0c…6e8`, plus the addendum `b2121cc0…de2` (see `kdenlive-api-contract.md` §4) |
| Daemon binaries (SHA-256) | `4eebdb74…` S1–S6, `04de9b70…` S4d and S8 (with the navigation fix), `80699ba9…` S7 |

## Method

* **Isolation.** MAIN's lease driver started Kdenlive under `dbus-run-session`
  on a private Xvfb display (`:99`), with private XDG directories, a generated
  media fixture and a `dbus-monitor` capture. Nothing touched the user's
  session, desktop, input devices or the pad.
* **Daemon.** The real daemon ran in `simulate --kdenlive-service NAME --trace`.
  This path uses the real `KdenliveDBusClient` and `Engine`, but scripted pad
  input and a **non-emitting key sink**: keys are recorded and checked, never
  injected. The config was `data/config.example.jsonc` of that time with
  `accelFactor 1`.
* **GUI.** Where a scenario needed a GUI state (a selected clip, the effect
  stack open, a focused wheel), small XInput helpers ran on the private display.
* **Checks.** Each script uses `await ctx`, `expect refused`,
  `expect no-refusal` and `expect no-keys`. History, effect values and clip
  positions were read back through MAIN's driver.

Runs:

| Run | Interface | Result |
|---|---|---|
| 01 | on | Aborted by my harness, which waited for `/ControlSurface` at the welcome screen. The object registers only once the main window is set up. The editor exited 0. |
| 03 | on | S1–S6 and S8. The editor exited 0, the process is gone, and the helpers exited 0. |
| 04 | off | S7. The editor exited 0, the process is gone. |

## Scenarios

The logs and scripts are in `records/k23-acceptance-20261007/`. A log line
`EXPECT FAIL` is explained in the table.

| # | Scenario | Result | Evidence |
|---|---|---|---|
| S1 | Jog in Slip mode moves the monitor only: 7 → 17 → 13; history and clips unchanged. | pass | `s01-transport.log` |
| S1 | Zoom 8 → 6 → 8. | pass | `s01-transport.log` |
| S1 | Shuttle | **open B**: refused `busy` ("monitor unavailable for shuttle playback") on Xvfb; reported, nothing typed | `s01-transport.log`, `s01b-play.log` |
| S1b | `monitor_play`/`monitor_pause` actions | **open D**: on Xvfb `playing` toggled but the playhead did not advance; the GUI Play button did advance | `s01b-play.log` |
| S2 | Three knobs on the three `colorWheels` handles without changing focus: lift 0 → 0.1, gamma 1 → 0.94, gain 1 → 1.16. Undo count 6 → 9; each wheel undid and redid separately. | pass | `s02-wheels.log` |
| S3a | Parameter nudge on a static value: level 100 → 105, one undo. | pass | `s03a-nudge.log` |
| S3b | Live grading while playing: 105 → 108 while the playhead moved 42 → 91, one gesture. | pass | `s03b-live.log` |
| S3c | Key creation: refused `busy` while playing; stopped, a key at 104 (`liveGrading` false); 110 → 111; four undo boundaries. | pass | `s03c-create.log`, `s03c-effects.json` |
| S3d | Multi-key edit while playing | **not reachable**: the GUI disables the off-key widget, so no `param.target` was published (`param.name` null) | `s03d-multikey-playing.log` |
| S4a | Track page: A1 manual mute kept across an exclusive solo/unsolo of A2. | pass (`s04a2`). The first attempt (`s04a`) hit the navigation race fixed below. | `s04a2-tracks.log` |
| S4b | Clip gain 0 → 0.5 dB, one undo. | pass | `s04b-clipgain.log` |
| S4c | Track mixer gain 0 → 0.5 dB, one undo. | pass (`s04c2`); the first attempt hit the same race | `s04c2-trackgain.log` |
| S4d | Fast multi-detent track navigation with the fix: −3 stepped V2 → V1 → A1 → A2, no refusal. | pass. The precondition line expected track 1 but the run started on 3, so +3 clamped at the top track. | `s04d-nav.log` |
| S5 | Resize end +6 / start −4: clips 5 and 6 at 26–216 (in 26), later pair unmoved, two undos. Ripple → `unsupported_mode`; locked V1 → `track_locked`. | pass | `s05a-trim.log`, `s05b-ripple.log`, `s05c-locked.log` |
| S6a | A second daemon gets `busy` while the first owns an editing gesture. | pass | `s06a-owner.log`, `s06a-second.log` |
| S6b | A foreign save or undo bumps the epoch, and the next turn is a fresh gesture. A foreign undo during a gesture makes the daemon drop it with no end. Stale races are refused and nothing is typed. | pass | `s06b-foreign.log`, `s06b2-midgesture.log` |
| S6c | Acks forged by foreign senders (seq 7 and 999) are ignored. | pass | `s06c-forged.log`, `s06c-forged-params.txt` |
| S7 | Interface **off**: `UnknownObject` → Absent; zero mutation calls on the bus. | pass | `s07-off.log` |
| S8 | Save and reopen: values kept, native ids reassigned (the precondition line assumed the old id 6), all handles fresh, a new edit is one undo, and there is an undo boundary at the reopen. | pass | `s08-reacquire.log`, `s08-*.json` |

Not applicable: duplicate sequence numbers and late cancels, because the daemon
never sends them. The mock and bus tests cover them.

**Context pacing.** All 160 `ContextChanged` signals carried `emittedAtMs`.
The minimum spacing per subscriber was 34 ms; the minimum arrival spacing was
33.3 ms (`emittedAtMs-analysis.txt`).

## Findings

1. **Navigation race (fixed).** A fast multi-detent turn of `timeline.track`
   or `param.focus` was refused `stale_context`. The first step moves the
   host's target and bumps the epoch, and the next batch still carried the
   old epoch.
   * Fix: the daemon holds further detents of that knob until the new epoch
     arrives (250 ms fallback).
   * Regression test: `tst_engine navigationWaitsForItsOwnEpochChange`,
     mutation-checked.
   * Confirmed by S4d.
2. **Policy change after the run.** In S7 the daemon still had stock-key
   fallback enabled by default, so it recorded `I RIGHT RIGHT LEFT O`. These
   keys were not emitted. The daemon is now **API-only by default**: with the
   interface off it types nothing and shows one notice, and keystrokes need
   `"keyFallback": true` on the profile. The lease is released, so S7 was not
   re-run. The same path is covered by `tst_engine absentIsApiOnlyByDefault`,
   `absentAtCallTimeWithoutOptInOnlyNotices`, and
   `tst_kdenlive_dbus stockKdenliveWithoutInterface` (real D-Bus, object
   absent).
3. **Allowlist.** Only 7 actions are offered (see the contract, §3). Key
   bindings for marks, edits and history do nothing until K23-MR1a.
4. **Harness lessons.** `/ControlSurface` appears only after project setup.
   Clicking the timeline moves focus, the playhead and the active track. The
   effect stack needs a click on the clip. Scroll the stack with a scrollbar
   drag, not the wheel.

## Not covered

* A real display and real audio: shuttle and `monitor_play` (open items B
  and D).
* A multi-key edit while playing, through the GUI (S3d).
* Additive solo, which no default binding uses.
* The physical pad. It has been silent since programming; see
  `hardware-ch552.md`.
