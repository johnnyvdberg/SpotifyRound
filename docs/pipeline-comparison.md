# How the Nautilus frame compares to production mobile renderers

Measured on the current tree (29 Oct 2026) with `DIVE3D_STATS=1`,
`frame_time` and `deploy/v3d-shaderdb/shaderdb.sh`. Reference facts are from
each engine's source and docs; the raw research note is
[renderer-frame-structure.md](renderer-frame-structure.md).

## 1. What we actually run

One render pass, 1080 × 1080, `Rgba8Unorm` colour + `Depth32Float` reversed-Z,
one sample, colour `Store`, depth `Discard`. Draw order: corner mask → sealed
models → solid models → seabed tiles → water (full-screen, last) → fading
models → glass → sprites → shafts → beams.

| leg | draws | instances | model tris | terrain tiles | sprites |
|---|---|---|---|---|---|
| cave | 16–28 | 232 | 39 k | 36 | 769 |
| abyss | 21–33 | 200 | 40 k | 32 | 769 |
| arctic | 31–34 | 353 | 45 k | 29 | 701 |
| shallows | 41–105 | 1 447 | 55–217 k | 65 | 609 |
| forest | 242–256 | 2 093 | 330 k | 69 | 520 |

Model triangles by LOD level, same frames:

```
forest : lod [312504, 18160, 0]     <- 95% still at full detail; LOD 2 never used
shallows: lod [199318, 6320, 0]
trench : lod [107402, 14080, 0]
```

Shader cost per pipeline (`inst / temps / threads`), from the V3D compiler:

| pipeline | vertex | fragment |
|---|---|---|
| seabed | **1059 / 64 / 2** | 40 / 15 / 4 |
| models | 275–541 / 32–55 / 2–4 | 53–761 / 16–49 / 2–4 |
| water | 36 / 6 / 4 | 266 / 20 / 4 |
| glass | 307 / 49 / 2 | 57 / 20 / 4 |

Two things jump out, and neither is the fragment shader everyone worries about:
the **vertex** stage is the expensive one (1059 instructions per seabed vertex),
and the LOD chain is present but almost never taken.

## 2. The reference pipelines

### Google Filament ([Renderer.cpp](https://github.com/google/filament/blob/main/filament/src/details/Renderer.cpp), [RenderPass.cpp](https://github.com/google/filament/blob/main/filament/src/RenderPass.cpp))

Shadows → structure (depth+normal, only if SSAO/contact shadows need it) →
**one colour pass, opaques then transparents** → post (TAA, bloom, grading as a
subpass). **No depth prepass**: it was deprecated in 1.4.5 and its APIs removed
in 1.5.0. Opaques sort front-to-back in Z buckets *then* by material; transparents
back-to-front. Lighting is clustered (froxel) with up to 256 punctual lights,
per-pixel. Materials are compiled offline into a package holding every variant,
pruned by `variantFilter`. Fragments default to `mediump` on mobile, and
`masked` materials switch to alpha-to-coverage instead of `discard`.

### Godot 4 Mobile ([render_forward_mobile.cpp](https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/forward_mobile/render_forward_mobile.cpp))

One render pass: **opaque → sky → transparent** (merged into the same pass),
tonemap as a **subpass**, MSAA resolved at pass end. No depth prepass for
opaques. Per-pixel shading, but **"Force Vertex Shading" is on by default on
mobile**. 8 omni/spot lights per mesh, 256 per view. Colour target is
`A2B10G10R10` "to enable AFBC on mobile devices", 24-bit reversed-Z. Mesh LOD is
generated at import with meshoptimizer and selected by a **screen-space pixel
threshold** (`mesh_lod_threshold`, default 1.0) with a per-node bias.

### Unity URP Forward ([UniversalRendererRenderGraph.cs](https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.universal/Runtime/UniversalRendererRenderGraph.cs))

Optional `DepthOnly`/`DepthNormals` prepass, off by default; **depth priming is
documented as unsupported on mobile TBDR**. Opaque → skybox → transparents →
post. 9 lights per object (1 per-pixel + 8), 32 additional per camera on mobile.
Über-shaders with keyword variants plus SRP Batcher.

### The wgpu and web cousins ([details](renderer-frame-structure-bevy-three-terrain.md))

- **Bevy 0.20** — opt-in prepass (`DepthPrepass`), then shadow passes, then one
  opaque and one transparent pass. Opaques are **binned** by pipeline/bind group/
  mesh slab, *not* depth-sorted; only transparents are sorted. Clustered-forward
  PBR, per-fragment. **No mesh LOD** (only experimental meshlets). GPU
  preprocessing and indirect draws by default. Draw count is not its bottleneck —
  phase/bin building and pipeline specialisation are.
- **three.js r186** — no prepass; one scene pass with opaque sorted
  front-to-back, then transmissive (its own offscreen pass), then transparent
  back-to-front. `MeshLambertMaterial` has been **per-fragment since r144**, so
  there is no Gouraud material left to copy. No built-in LOD. Draw calls and
  per-object uniforms dominate.
- **vange-rs** — the closest cousin in shape: **two** render passes (optional
  1024² shadow, then main), and in its default raymarch modes the whole terrain
  is **one draw**, because it raymarches a heightfield per pixel instead of
  submitting vertices. 3–15 draws total. Per-pixel lighting, one directional
  light plus fog.
- **bevy_terrain** — a compute tiling prepass refines a quadtree and writes
  indirect args, then the entire terrain is **one indirect draw** with CDLOD
  vertex morphing.

### What the tile-based guidance says (V3D included)

- **No depth prepass.** PowerVR states there is "no performance benefit" on
  hidden-surface-removal hardware
  ([PowerVR](https://docs.imgtec.com/performance-guides/graphics-recommendations/html/topics/z-pre-pass-and-powervr.html)).
- **`discard` is the expensive one.** V3D used to disable Early-Z for any draw
  containing it; enabling Early-Z where depth writes are off was worth **+14.87%**
  ([FOSDEM 2025, V3D](https://archive.fosdem.org/2025/schedule/event/fosdem-2025-5553-getting-more-juice-out-from-your-raspberry-pi-gpu/)).
- **The 32-register cliff is real and in Mesa's source**: 64 physical registers at
  1×/2× threading, **32 at 4×**
  ([vir_register_allocate.c](https://gitlab.freedesktop.org/mesa/mesa/-/raw/main/src/broadcom/compiler/vir_register_allocate.c)).
  Exceed it and the driver halves threads 4→2→1 and recompiles.
- **SFU ops (`sin`/`pow`/`exp`) stall separately** from ALU and are counted
  separately in shader-db.
- **Pi 5 has ~17 GB/s of shared LPDDR4X**; a 1080² 32-bit store is ~4.7 MB, so
  tile load/store cycles are a real line item.

## 3. Side by side

| axis | Nautilus | Filament | Godot Mobile | URP |
|---|---|---|---|---|
| Opaque passes | 1 | 1 | 1 | 1 |
| Depth prepass | no | removed | no | off by default; unsupported on TBDR |
| Opaque order | material-major, then depth | depth bucket, then material | material/draw list | material |
| Transparents | after opaque, same pass | after opaque, same pass | merged into same pass | after opaque |
| Sky/water | full-screen, **last** | colour pass | after opaque | after opaque |
| Lighting | per-vertex + per-**instance** CPU fold | clustered per-pixel, 256 lights | per-pixel, vertex shading forced on mobile | 9/object per-pixel |
| LOD | 3 levels, error+stray+edge in px | none in engine | import-generated, screen-space px | LOD Group |
| Materials | per-material pipelines (48) | offline variants | specialization constants | keywords + SRP Batcher |
| Precision | fp32 only (WGSL) | mediump default mobile | fp16 shader group | fp16 where supported |
| Target | **texture → UI compositor** | swapchain | swapchain | swapchain |

| axis | Nautilus | Bevy | three.js | vange-rs |
|---|---|---|---|---|
| Passes | 1 | 2–3 + shadows | 1 + shadows | 2 (shadow + main) |
| Depth prepass | no | opt-in | no | no |
| Opaque order | material-major, then depth | binned, no depth sort | front-to-back | none |
| Terrain submission | 29–86 tile draws | meshes batched | meshes | **1 draw**, raymarched |
| Model draws | 16–256 | GPU-preprocessed | per object | 3–15 |
| LOD | 3 levels, error+stray+edge px | none (meshlets) | manual `LOD` | per-chunk LOD |
| Lighting | per-vertex + CPU instance fold | clustered per-fragment | per-fragment Lambert/Standard | per-pixel |

## 4. Are we fundamentally different?

**No.** The frame is a textbook mobile forward renderer: one pass, transparents
after opaques, no depth prepass, material-specialised pipelines, reversed-Z,
automatic LOD. Every one of those choices matches at least one of Filament,
Godot Mobile, Bevy, three.js or vange-rs — including the one that looks most
unusual here, per-vertex lighting (Godot forces it on mobile by default). The
ordering detail varies more than the pass structure: Filament and three.js sort
depth-first, Bevy bins opaques without any depth sort, and we sort
material-major with depth inside each group.

Four things are genuinely ours:

1. **We are vertex-heavy where everyone else is pixel-heavy.** Our seabed vertex
   shader is 1059 instructions; a typical engine's is 50–150, because their
   terrain colours come out of a texture or a baked vertex stream. We generate
   the shading procedurally *per vertex, per frame* even though the CPU already
   builds every tile ([terrain.rs](../src/dive3d/terrain.rs)). Our fragment
   shaders are correspondingly lean (terrain 40, water 266) — the opposite of the
   usual profile.
2. **Lighting is folded on the CPU, per instance** (0.001 ms for 355 instances,
   ~0.3 ms inside the scene build). No reference engine does this; they all keep
   lights on the GPU. It is a legitimate specialisation for four lights.
3. **We render into a texture that a UI toolkit composites**, with a fixed
   1080² and no dynamic resolution. Mobile engines scale resolution under load;
   this design deliberately cannot.
4. **We submit the seabed as 29–86 tile draws** where bevy_terrain and vange-rs
   submit one. At this size that is CPU noise, not the frame — but it is the one
   structural place a modern terrain renderer would differ.

And two places we are simply behind:

- **LOD is effectively inert.** With `LOD_ERROR = 0.5` px *and* the
  `stray < 0.1 || edge × px < 1` clause, 95% of model triangles are full detail
  and LOD 2 is never selected. Godot's comparable knob defaults to a 1.0 px
  *error* and has a bias to trade quality for speed; ours is stricter and has no
  bias.
- **No reduced-precision path.** Filament and Godot both ship fp16/mediump on
  mobile. WGSL has no precision qualifiers, and V3D 7.1's fp16 support is
  partial, so this is a small gap rather than a big one — but it is a gap.

## 5. What the comparison implies

1. Keep the shape: one pass, no prepass, water last, material specialisation.
   The references do the same and the TBDR guidance agrees.
2. Cut the **vertex** stage first. Baking the procedural seabed shading into the
   vertex buffer the CPU already writes removes most of `vs_terrain`'s 1059
   instructions without changing a pixel; shortening `vs_model` to 32 temps
   would also restore 4-thread occupancy on 330 k triangles per frame.
3. Make LOD actually select. Raising the pixel budget and dropping the
   `stray`/`edge` gate to a proper screen-space criterion is the cheapest
   geometry reduction available.
4. Leave the fragment shaders alone except for the water (266 inst over the whole
   frame) and the vault pipelines still at 2 threads.
5. Later, if terrain submission ever matters: a compute cull/LOD prepass and a
   single indirect draw is the pattern bevy_terrain uses. It is not worth it at
   29–86 tiles today, and vange-rs's raymarch trades vertex work for pixel work,
   which is the wrong trade on a GPU that is already fill-bound.

## 6. Low-spec lighting: the references already do what this renderer does

The nine engines above all keep lighting on the GPU, but they split it between
the vertex and pixel stages very differently, and the mobile/low-end setting is
consistent across the two engines that expose one:

| engine | main/directional light | additional or local lights |
|---|---|---|
| **Unity URP** | **per pixel** (or None) | **Per Vertex**, Per Pixel or Disabled, with a per-object limit ([URP Asset](https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@15.0/manual/universalrp-asset.html); [`shadeAdditionalLightsPerVertex`](https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@15.0/api/UnityEngine.Rendering.Universal.LightData.html)) |
| **Godot 4.4+** | per vertex with "force vertex shading", pixel shadows from the first directional light only | **per vertex**, a simplified light model "like 3.x"; omni/spot shadows are not available on vertex-shaded materials ([PR #83360](https://github.com/godotengine/godot/pull/83360)) |
| **Filament** | per pixel, clustered | per pixel, up to 256; no per-vertex mode at all — mobile is won with `mediump`/fp16 and approximations ([Filament](https://google.github.io/filament/Filament.md.html)) |
| **PowerVR** | — | the Golden Rules' answer: "reduce the overall number of calculations by moving them earlier in the pipeline where there are fewer instances to process" ([PDF](http://cdn.imgtec.com/sdk-documentation/PowerVR%20Performance%20Recommendations.The%20Golden%20Rules.pdf)) |

So there are two low-end strategies in the wild:

- **Per-vertex lighting** (URP's additional lights, Godot's forced vertex
  shading, the PowerVR rule): keep the light count off the pixels. The cost is
  that shadows and specular falloff disappear between vertices.
- **Per-pixel with reduced precision** (Filament): keep the lighting per pixel,
  but run it in `mediump` and simplify the BRDF. This needs fp16, which the
  Pi's V3D only partly has, and it is the strategy that has already been ruled
  out here (WGSL has no precision qualifiers).

**Nautilus is already on the first strategy, and further than either
reference.** The `full-hd` build — the one the kiosk runs — computes the whole
light at the vertex in `vs_model`: sun, sky, caustics, the searchlight and the
four point lights (503–543 instructions), and the fragment shader only samples
the surface texture and adds the material effects (53–165 instructions, no
`light_at` at all). URP still evaluates its main light per pixel; Godot keeps
pixel shadows for the first directional light. The seabed is vertex-lit and
baked the same way. So there is no low-spec lighting solution left to import
for the kiosk's path.

Two things follow:

1. The remaining per-pixel cost is **texture sampling and material effects**,
   not lighting. `plain textures` saves 1.57 ms (§7.5 of
   [pipeline-optimization.md](pipeline-optimization.md)), while the whole
   `fs_model` is already 53–165 instructions. A different lighting model would
   not touch it.
2. The remaining *vertex* cost is the 503-instruction `vs_model` (plus the
   215-instruction binning variant). Our shader is three to ten times the
   50–150-instruction vertex shaders those engines use for vertex lighting,
   because it carries five lights and the caustic fetch. Godot's simplified
   model exists for exactly that. Our `CHEAP_MODELS` switch is a runtime
   uniform branch, so it does not reduce the register pressure that keeps
   `vs_model` at two threads; a compile-time simplified variant would be the
   only way to test whether it can reach the 32-register cliff and four
   threads. Untested.

The `!full_hd` variant (the half-resolution build, not the kiosk) still calls
`light_at` in the fragment *and* multiplies in the vertex light, so that path
does apply every light twice. It is not what the Pi runs, but it is a latent
inconsistency if that build is ever brought back.
