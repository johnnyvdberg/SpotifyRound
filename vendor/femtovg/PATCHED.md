# femtovg 0.27.0, patched for tile-based GPUs

This is femtovg 0.27.0 from crates.io (MIT/Apache-2.0), used through
`[patch.crates-io]` in the top-level `Cargo.toml`. Only
`src/renderer/wgpu.rs` is changed; the examples and tests are left out.

On a tile-based GPU such as the Raspberry Pi 5's V3D, a render pass that loads
its attachments copies the whole framebuffer from memory into the tile buffer
first, and every stored attachment is copied back out again. Upstream starts
every pass with `LoadOp::Load` on colour and stencil and stores both, and draws
the window clear as a full-screen quad, so a 1080 × 1080 frame moved about
12 MB it never needed.

The changes:

- A frame whose first command clears the whole screen (Slint always does this
  with the window background) begins with `LoadOp::Clear` instead of loading
  and then drawing the clear.
- A `SetRenderTarget(Screen)` while already on the screen no longer opens a new
  pass.
- The stencil is cleared at the start of each pass and discarded at the end.
  Every fill and stroke leaves it at zero, and passes only break between
  commands, so its contents never matter across a pass boundary.

To update femtovg, copy the new release here and reapply these three changes.
