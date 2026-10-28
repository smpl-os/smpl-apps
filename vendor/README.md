# Alpha-safe CPU presentation

These are source copies of crates.io `i-slint-renderer-skia 1.15.1` and
`softbuffer 0.4.8`, with their original copyright and license files. Slint and
its backend/compiler are pinned to 1.15.1 so a routine update cannot silently
bypass the presentation patch. The workspace's crates.io patches apply to
every application and release build.

Local changes:

- Skia `software_surface.rs`: wrap BGRA8888 as `AlphaType::Premul`, not
  `Opaque`, and report 32 bits per pixel. This keeps transparent window space
  and translucent backgrounds without fading opaque text, icons or shadows.
- Skia's softbuffer dependency enables the opt-in `wayland-alpha` feature.
- Softbuffer `wayland/buffer.rs`: that feature selects `Argb8888` both on
  creation and resize; without it the upstream XRGB behavior is unchanged.
  Memory-file creation, sizing and mapping errors propagate instead of
  panicking/being ignored; checked pool-size arithmetic rejects overflow before
  allocating. Resizing resets buffer age because the newly created backing
  buffer must be repainted, not treated as an intact previous frame.
- Softbuffer's manifest omits upstream example/benchmark-only dev dependencies
  and its benchmark target (those sources are not vendored).

Application code selects `skia-software` explicitly, not Skia's GPU-capable
default. The full rasterizer preserves shadows, gradients, transformations
and paths that the lightweight Slint software renderer does not fully support.
GPU-specific source files are unmodified upstream code, not selected at runtime.
Do not delete those files: the crate still compiles its normal platform APIs.

When updating, rebase these small changes onto the corresponding published
sources and rerun the theme contracts, native CPU X11/Wayland probes, calendar
fixtures and workspace checks described in the root README. Verify actual
ARGB buffers through resize/reopen, not only renderer snapshots.

Focused allocation/format tests:

```sh
cargo test --locked -p softbuffer --lib
```

Run this from the workspace root so it uses the application's locked
dependency versions and enabled presenter features.
