# Pad firmware releases

| File | What |
|---|---|
| `control-surface-sy181-15k3e-2.0.2.bin` | Firmware for the CH552G + TM1650 pad with 15 keys and 3 knobs (USB 1189:8890, serial key153). Current; verified on hardware in both input modes. |
| `control-surface-sy181-15k3e-2.0.1.bin` | Superseded: raw mode drops every 256 ms, commands can be dropped (see its JSON `knownIssues`). Kept for reference only. |
| `control-surface-sy181-15k3e-2.0.0.bin` | Superseded: fast knob rotation drops detents (see its JSON `knownIssues`). Kept for reference only. |
| `<name>.json` | Name, version, board, licence, SHA-256, size, source commit, toolchain, `verifiedOnHardware`, `knownIssues`, `supersededBy` |
| `LICENSE` | Licence notice (CC BY-SA 3.0) |

These images are built from `firmware/` in this repository by
`firmware/make-release.sh`, which builds twice and refuses to write a release
unless both builds are byte-identical. The JSON's `verifiedOnHardware` stays
`false` until a release has been flashed to a pad and all its inputs captured.
`hardwareVerification` records what was checked. A wizard should offer the
newest image without `supersededBy`.

Flash with the CH552 ROM bootloader only, for example
`firmware/flash-and-verify.sh` (it checks the SHA-256 and never writes the
chip's configuration registers). See `docs/FIRMWARE-PLAN.md` §7 for the
procedure, the verification steps and recovery.

## Licence and attribution

The firmware is a fork of **EpicLPer's CH552-OpenMacroPad**
(<https://github.com/EpicLPer/CH552-OpenMacroPad>), built on **Stefan
Wagner's** CH55x USB HID code (<https://github.com/wagiminator>). Both are
licensed under the Creative Commons Attribution-ShareAlike 3.0 Unported
licence, and so are these images: <http://creativecommons.org/licenses/by-sa/3.0/>.
`src/ch554.h` is WCH's register header. The changes from upstream are listed
in `firmware/README.md`; the source is in `firmware/`.
