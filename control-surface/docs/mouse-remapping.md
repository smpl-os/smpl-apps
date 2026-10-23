# Razer mouse remapping

`mouse-remapperd` is a separate process using the control-surface binding
parser, Engine, Kdenlive D-Bus client, key names, mouse-action triples, profile
matching, installed-app conditions and Hyprland window tracker. Settings uses
the same assignment editor and live/offline Kdenlive action catalog as Keypad.
It never constructs a pad backend,
requests the keypad bus name, or updates the keypad icon.

## Runtime and delivery

- `mouse-remapperd features --json` is offline and never opens input. API 1
  returns `vendor: "1532"`, `product: "00b4"`, `learning: true`,
  `identify: true`, `passthrough: true`, `appProfiles: "hyprland"`,
  `kdenliveApi: true`. The last flag means shared plugin dispatch is compiled
  in, not that a running Kdenlive instance has enabled its interface. Old
  reduced mouse backends without this flag are incompatible with this Settings.
  `identifyWheels: true` and `guidedCalibration: true` additionally advertise
  wheel/tilt identification and the guided side-grid calibration method.
  `guidedSetup: true` additionally requires the complete visible Razer setup
  workflow, including explicit arming, retry/skip, review and confirmed save.
- `mouse-remapperd run` needs the graphical user's session bus, read access
  to the USB `1532:00b4` evdev nodes, and write access to `/dev/uinput`.
  No root daemon, broad permission changes or access to other keyboards is
  required. smplOS owns exact-device udev access and device-bound
  `mouse-remap.service` (`ExecStart=/usr/bin/mouse-remapperd run`).
- Discovery uses sysfs IDs, then checks `EVIOCGID` and capabilities on every
  opened button node. Event numbers are never persisted. Exactly one physical
  mouse is supported; ambiguity or unavailable nodes refuses activation.
  The absolute-only auxiliary interface is left untouched.
- Each button interface has its own virtual passthrough device with copied
  key/relative/absolute/switch capabilities and input properties. Motion,
  ordinary and high-resolution wheels, unmapped key repeats, buttons and
  synchronization frames pass through. Hardware scancodes are omitted;
  LED/repeat-configuration feedback is not mirrored.
- All virtual outputs must be created before grabs. Activation waits for
  physical buttons to be released; it never takes over a held drag. Input
  overflow/read/write failures destroy the virtual devices, ungrab physical
  input, report the error and retry. Generated holds use reference counts and
  are released on focus/config/mode changes, disconnect and orderly shutdown.
  Consumed inputs remain suppressed until their physical release.
- Global profiles work through uinput on any compositor. App profiles require
  the existing Hyprland IPC tracker; unavailable/disconnected focus is reported
  explicitly and uses Global only. KWin per-app tracking is not implemented.
- Ship matching Settings and daemon binaries plus the OS service/access
  integration. CMake installs the binary; `scripts/release_assets.py` requires
  and smoke-checks it in the control-surface asset. Fleet releases must use
  the existing Qt 6.11/glibc 2.44 baseline, not a newer local build.

## Configuration

The daemon reads **`$HOME/.config/smplos/mouse.json`**, matching the existing
Settings path rather than moving data under `XDG_CONFIG_HOME`. It checks the
file every second, including atomic replacement. Invalid configurations
disable grabs and report an error; no saved data is replaced by defaults.

The stable device ID is `mmo-12`. Profiles and all existing unknown fields are
preserved when learning. The unrelated `generic` and other device entries
are never activated by this daemon.

```json
{
  "version": 1,
  "devices": {
    "mmo-12": {
      "learned": {
        "grid-1": { "source": "input1/keyboard", "code": 2 }
      },
      "profiles": [
        {
          "name": "Browser",
          "match": { "class": "^firefox$" },
          "bindings": { "grid-1": "alt+left" }
        },
        {
          "name": "global",
          "bindings": { "grid-1": "ctrl+c", "btn-back": { "mouse": "middle" } }
        }
      ]
    }
  }
}
```

**The code in this example is illustrative, not a factory side-key map.**
For full setup, choose **Set up this mouse** in the Mouse Buttons card header.
The wizard highlights primary left/right/middle, wheel up/down and tilt,
grid 1 through 12, upper-left DPI-stage up/down and scroll mode. Click
**Capture next press**, move the pointer over the diagram, then press and
release the highlighted physical control. Only explicitly armed input is
captured; navigation clicks cannot advance a capture automatically.
The captured input is shown before **Next**; **Try again** clears this step,
and **Skip** retains its existing mapping. Captures remain staged until final
**Review** and **Save setup**. Cancel/Escape/client loss discards them all.
A 15-second armed timeout leaves this step retryable/skippable, not guessed
or silently marked successful.

Save uses the same verified backup and atomic writer as learning, and
refuses to replace a configuration changed since setup began. Primary
identity remains restricted to the matching BTN_LEFT/RIGHT/MIDDLE codes.
Wheel captures store the canonical REL_WHEEL/REL_HWHEEL axis with direction
`+1` or `-1`; legacy and high-resolution reports name the same input.
Existing key captures retain their `{source,code}` shape. Wheel captures
add `"type":2,"direction":1` (or `-1`). Skipped/unrelated fields and profiles
are preserved. Existing `btn-back`, `btn-forward` and `btn-dpi` slots remain
as compatibility assignments; separate `btn-dpi-up`, `btn-dpi-down` and
`btn-scroll-mode` positions describe the actual extra physical controls.

Use **Calibrate side grid** in the Mouse Buttons card header, above the
diagram (not in Keypad), once: the diagram highlights each numbered
position and prompts for physical buttons 1 through 12. Press and release
each highlighted position. The daemon captures actual source/code pairs,
refuses duplicates and chord/modifier input, and saves all 12 together with
one verified backup. Cancel/timeout/client loss discards incomplete calibration
without changing saved data. Recalibrating can replace an older side-grid map
atomically, even when onboard assignments have swapped the codes.
Use Learn for a single changed grid/extra button. Sources identify a physical USB interface
suffix plus its pointer/keyboard capabilities, not a `/dev/input/eventN`
number or a general keyboard. Moving the mouse to another USB port does not
change that suffix.

The first matching app profile wins. An absent slot falls through to Global
unless `fallthrough` is false, in which case it retains its physical behavior.
`"none"` explicitly suppresses a button. Shared binding forms supported here
are keyboard shortcuts/sequences, media keys, mouse clicks/wheels, commands
and `ifInstalled` conditions. Single chords and mouse clicks are held;
multi-chord shortcuts tap in order on press. Commands launch detached once
per press without a shell, matching the keypad's launch semantics, so apps
survive daemon restarts. Launch failures are reported; detached process exit
statuses are not monitored. Modes, context layers and advanced `do` sequences
use the same Engine as Keypad. Keypad cheatsheets and keypad held-control
layers are explicitly refused rather than accepted without functioning.

For Kdenlive, add/select its app profile, enable **Kdenlive control surface
API**, select a diagram button, choose the Kdenlive action category and then a
concrete action. The catalog queries the running plugin read-only, falling
back to the keypad daemon's offline catalog when no plugin is available.
Re-entering the tab or choosing Refresh updates discovery. Regular status
heartbeats update only live status, not the assignment dropdown models.
The first diagram button is selected on entry, so action categories are
populated even before a diagram click.

API profiles require an app match and `kdenlive: true`; API actions are not
allowed in Global. `action`, `control` and `request` bindings (including those
inside layers/sequences) dispatch through the existing Kdenlive client with
the focused PID, plugin capabilities, target/context, lease and refusal
handling. Enable **Control surface interface** in Kdenlive itself (off by
default). Absent/pending/refusing services are reported explicitly and never
fall back to keys. Mouse profiles reject `keyFallback: true`; even a saved
binding's `fallback` keys cannot be emitted by API dispatch. Ordinary keyboard
shortcuts are separate, explicitly selected assignments.

Primary left/right clicks only accept left/right click assignments.
Single-button Learn cannot capture the primary left/right/middle inputs;
full setup can record their matching identity, but cannot replace it
or assign two positions to the same learned input. Relative captures
distinguish opposite directions on the same axis.
Learning accepts an evdev key/button on release, not an onboard macro.
Normal vertical/horizontal wheel events already have diagram positions:
`wheel-up`, `wheel-down`, `wheel-tilt-left`, `wheel-tilt-right`. These work
on keyboard-tagged as well as pointer-tagged Naga interfaces. Ordinary and
high-resolution reports for the same direction in a synchronization frame
identify once; mapped scrolling dispatches through the shared Engine once
per 120 high-resolution units, without doubling legacy/high-res pairs.
Unmapped scrolling remains unchanged outside Identify.
Optional DPI controls may emit no evdev input in their current onboard mode.
Razer's manual says its DPI-stage assignments can be reconfigured in Synapse;
that does not imply an evdev event is available for their current assignment.
This backend never writes onboard memory or resets firmware. A no-input
learning timeout reports that limitation explicitly, not a successful capture.
Reprogrammed onboard profiles can change side codes; relearn rather than
applying an assumed factory map.

Learning follows configuration symlinks, checks the current document again
before replacement, and writes atomically without direct-write fallback.
Each existing document is backed up exactly and verified under
`backups/mouse-learn-<UTC>-<UUID>.json`; permissions are preserved and learning
never prunes those backups. A failure leaves the configuration untouched and
is visible in status. Settings detects external writes before its next edit,
reloads after learning, and preserves the `learned` field.

## Session D-Bus API (version 1)

Name/interface: `org.smplos.MouseRemapper1`.
Object: `/org/smplos/MouseRemapper1`.

| Method | Signature | Behavior |
|---|---|---|
| `Status` | `() -> s` | JSON; read-only except refreshing this client's identify lease |
| `SetIdentify` | `(b) -> ()` | Pause assigned actions; select known pressed slots and wheel/tilt directions; primary clicks/motion remain usable |
| `Learn` | `(s slot) -> ()` | Capture the next single-key press/release for the selected grid/extra/DPI/tilt slot |
| `Calibrate` | `() -> ()` | Guide through all 12 side-grid positions, atomically saving only a complete capture |
| `CancelLearn` | `() -> ()` | Stop learning without changing assignments |
| `StartSetup` | `() -> ()` | Start full Razer setup with staged captures and a highlighted target; nothing saved |
| `ArmSetup` | `() -> ()` | After navigation buttons are released, arm exactly the current capture |
| `NextSetup` | `() -> ()` | Confirm a captured step and select the next control |
| `SkipSetup` | `() -> ()` | Discard this step's capture and keep its existing mapping |
| `RetrySetup` | `() -> ()` | Clear this step's staged input and return to unarmed capture |
| `SaveSetup` | `() -> ()` | Only from final review, atomically save captured positions with a verified backup |
| `CancelSetup` | `() -> ()` | Discard staging and exit Identify, restoring normal input |

`InputEvent(s json)` reports `{slot, input, source, type, code, value, sequence}` while identifying or
learning. Unknown inputs have an empty `slot`; Settings never guesses their
diagram position. Status also carries `lastSlot`, `lastInput`, `sequence`
so a polling UI can select the pressed known button and open its editor.
Pointer movement does not flood the bus.
Status `events` retains the latest 64 structured events for read-only
diagnostics, so a quick event followed by another press remains inspectable.
All Naga non-primary keys/buttons (including keyboard keys on mixed pointer
interfaces) and wheel/tilt input are suppressed during Identify: no side
typing, browser navigation or editor scrolling. Primary left/right/middle
clicks and motion still pass through so Stop/Cancel can be used. Other
keyboards are never opened.

Status contains `apiVersion`, `connected`, `ready`, `error`, `notice`,
`configPath`, `learned` and `unlearned` (slot names), `identify`, `learning`, `lastSlot`,
`lastInput`, `sequence`, `focusBackend`, `focusWarning`, `kdenliveApi`,
`kdenliveAvailable` and `kdenliveProfile`. The latter two describe the focused
profile's actual runtime plugin availability, separately from compiled support.
Connection alone
does not imply readiness. Settings verifies the configuration path before
enabling live controls. Action-launch errors are retained as notices.
`calibrating` distinguishes guided capture; `learning` identifies the next
diagram position to highlight. A captured grid position is not guessed from
the current keyboard code.
`guidedSetup` reports compiled complete-wizard support. `setup` contains
`active`, `phase` (`idle`, `armed`, `captured`, `review`), `target`, `step`,
`total`, `captured` (count) and `rows` (review descriptions). Setup targets
take precedence over incidental navigation clicks when selecting the diagram.

Identify/learning belongs to one D-Bus unique client. Another client receives
an explicit refusal. Disconnecting that client clears modes. `Status` from
the owner refreshes a 10-second lease; learning itself times out in 15 seconds.
An armed setup timeout instead returns the current step to `idle`, preserving
earlier staging for explicit retry/skip. Closing Settings or losing the lease
still discards all unconfirmed setup data.
Settings keeps one worker-thread bus connection, polls while the tab is open,
exits modes when leaving the tab/changing device, and selects the diagram
button on a changed input sequence.

## Validation

`tst_mouse` exercises exact source matching, app/Global fallthrough, disabled
bindings, malformed/unsupported configs, shared holds, repeats, cleanup,
identify suppression, motion/scroll passthrough, atomic learning, verified
backups, duplicate capture rejection and preservation of unrelated settings.
It uses temporary configurations and an injected event sink, not live input.
The Settings model and UI-contract tests cover diagram selection and live
control wiring. `python tests/run_mouse_editor_probe.py` exercises the actual
shared Slint editor's category and concrete Kdenlive action popups under
authenticated private Xvfb/FemtoVG, including selection/persistence across
status heartbeats. The native mouse suite also exercises real private-bus
plugin dispatch for actions, controls, requests and context layers, absent
plugins, focus loss and cancellation of delayed work when identifying.
The production MainWindow private-bus fixture verifies actual diagram
selection across all 12 calibrated positions, extra buttons and wheel/tilt
directions, next-slot calibration highlighting and mode exit.
It clicks the actual visible calibration and full-setup buttons at 900x560,
then exercises all 22 wizard positions, capture/next/retry/skip/review/save,
Cancel and keyboard Escape. `tst_mouse` separately routes simulated key
press/releases and high-resolution/legacy wheel pairs through the capture
state machine, tests atomic save/preservation, and refuses concurrent edits.
Physical-device/compositor acceptance is a separate,
coordinated local step; building or running the offline feature check does
not activate input capture.

## Onboard DPI backup and guarded remapping

`mouse-remapperd onboard-backup --json` uses only whitelisted vendor read
commands, not the input grabber or Settings D-Bus service. It dynamically finds
one exact `1532:00b4` USB interface-0 hidraw node, checks the opened character
device's IDs and sysfs identity again, and refuses ambiguity. HID feature
transport requires an `O_RDWR` descriptor and `HIDIOCSFEATURE` even for read
queries; the report's command IDs remain read-only. This is not an assignment
write, firmware update, reset, OpenRazer installation or permission change.
Permission errors require a narrowly scoped foreground `pkexec` invocation,
not `sudo` in a terminal or broad `chmod`/udev access.

The helper verifies 91-byte framing (report ID plus 90-byte vendor report),
transaction `0x1f`, CRC, command, status, argument size and echoed button
address for normal-layer functions. It reads the serial (stored only as
SHA-256), firmware, single-profile counts, and all 42 normal/Hypershift query
replies for the 21 documented primary/wheel/top/grid controls. A second
independent read must match exactly before any backup JSON is emitted.
Schema 2 explicitly has `scope:"normal-layer-only"`: `assignments` contains
only the 21 verified normal-layer functions, while `readEvidence` retains
both complete 42-query raw argument maps. `hypershiftVerified:false` makes
clear that independent Hypershift storage was not established. Actual
receiver firmware 1.1 (`01010000`) returns the normal-layer byte and identical
normal function for every Hypershift request; these are evidence, not verified
Hypershift assignments. Normal replies still require exact address echoes
and valid function lengths. The mechanical
scroll-mode control has no verified assignment address and is not guessed.
The backup is user data; preserve and verify the output file, never commit it.
Running the backup does not replace installed binaries or affect existing
learned controls.

If strict address validation fails, `onboard-probe --json` records two passes
of all 42 read-only requests with the requested prefix and returned argument
bytes. This distinguishes response-echo quirks from invalid function lengths
without accepting a mismatch. Its output explicitly says `diagnosticOnly:true`
and `usableAsBackup:false`; it is not accepted for planning or restoration.
It does not relax normal-layer backup validation or write onboard assignments.

`mouse-remapperd onboard-plan --backup FILE --json` validates that complete
backup against a fresh double read of the same receiver. It describes only
normal-layer top buttons `0x0b`/`0x0c` becoming F23/F24 (HID keyboard usages
`0x72`/`0x73`), refusing collisions with other onboard buttons. Both original
seven-byte functions are included for a possible exact two-button restore.
Only normal-layer addresses would be written; all other normal-layer
assignments must remain unchanged. Because this receiver does not establish
independent Hypershift state, unchanged effective Hypershift behavior cannot
be promised. Backup/probe/plan commands never write assignments. Enabling
even those two buttons requires specific user approval after the backup/readback and proposed
change are reviewed. A successful read is not proof of a successful restore.
Emitting spare keys also does not implement held workspace layers.

Protocol facts were cross-checked against the MIT-licensed projects
[mouseo at 60559ee](https://github.com/pengowray/mouseo/tree/60559ee0a6e8a25e362923ed0a5abf41d1265d4a)
and [razerqdhid at 4363a88](https://github.com/geezmolycos/razerqdhid/tree/4363a8836ab970c712fa8f2a9dd530a7e7bc5011).
This helper is an independent native implementation; it does not run their
personal layout/apply/test scripts or clone their runtime code. The button
read is `0x028c`, and the documented assignment/restore command is `0x020c`;
the latter is deliberately refused by the read-query transport.

After that approval, `onboard-apply-dpi --backup FILE --sha256 HASH
--confirm-dpi-to-f23-f24 --json` checks the exact saved file hash, device,
complete normal preimage and raw read evidence again. The write transport
permits only normal profile-1 buttons `0x0b`/`0x0c` and their exact original
DPI next/previous or proposed F23/F24 functions. It cannot set another button,
layer, profile, arbitrary key, firmware or reset command. After each write,
a fresh double-read verifies the expected complete normal-layer state,
including all other controls.

`onboard-restore-dpi --backup FILE --sha256 HASH --confirm-restore-dpi --json`
restores those exact original functions; it accepts only original/mapped
states on the two targets, with all other normal controls and device identity
matching the backup. Already-restored controls require no write. On an apply
failure, automatic restoration uses the same checks and verifies the original
normal state. A failed acknowledgement may still mean a write occurred, so
readback is authoritative. If readback is unavailable or unrelated assignments
changed concurrently, restoration is explicitly refused/unverified rather
than overwriting unknown state or claiming success. Keep the original backup
and investigate; never reset or flash the mouse as a recovery shortcut.

These are one-shot, explicitly confirmed foreground administrator commands.
They do not install or start another daemon, broaden hidraw permissions,
change Settings mappings, or implement workspace layers. The device-bound
evdev remapper learns the newly emitted inputs through the existing wizard.
