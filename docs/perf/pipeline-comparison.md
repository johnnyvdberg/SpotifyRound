# How the Nautilus frame compares to production mobile renderers

Measured on the current tree (29 Oct 2026) with `DIVE3D_STATS=1`,
`frame_time` and `deploy/v3d-shaderdb/shaderdb.sh`. Reference facts are from
each engine's source and docs; the raw research note is
[the appendix](#appendix-a-filament-godot-4-mobile-urp-forward).

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

### The wgpu and web cousins ([details](#appendix-b-bevy-threejs-vange-rs-bevy_terrain))

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
   builds every tile ([terrain.rs](../../src/dive3d/terrain.rs)). Our fragment
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

---

## Appendix A: Filament, Godot 4 Mobile, URP Forward

Facts are from each renderer's source (`main`/`master`, Oct 2026) plus vendor docs. All three build a pass graph and **cull disabled passes**, so "order" is declaration/dependency order, not a guaranteed trace.

### 1. Google Filament (Android-first)

| Question | Finding |
|---|---|
| Per-frame passes | shadow maps (CSM + spot/point atlases) → structure (depth+normal, only if SSAO/contact shadows need it) → SSAO → SSR → **one colour pass, opaques then transparents** → resolved depth (only if non-MS depth is consumed) → motion vectors → TAA → DoF → bloom → colour grading (usually a *subpass*) → FXAA → upscale/blit. SSAO/SSR/DoF/TAA all default off. [1][2] |
| Depth prepass | **No.** "The depth prepass setting in View is now ignored and deprecated" (v1.4.5); "Removed depth-prepass related APIs" (v1.5.0). Instead opaque commands sort front-to-back in Z buckets, then by material ("bucket objects by Z, front-to-back and then sort by material"), relying on early-Z. Transparents sort back-to-front honouring explicit blend order. [2][4] |
| Lighting | Clustered: froxel binning (`Froxelizer`), up to **256** punctual lights ("more than the GPU buffer allows (i.e. 256)"), one dominant directional with shadows plus optional extra directionals. Per-pixel shading; IBL is the main ambient term. [1][3] |
| LOD / culling | No mesh-LOD system found (texture LOD bias only): per-view frustum culling of renderables and lights on the job system, visibility layers, automatic draw-call instancing. LOD is the app's job. [1][2] |
| Material strategy | One shader per material, compiled **offline** by `matc` into a package holding **all variants** (`directionalLighting`, `dynamicLighting`, `shadowReceiver`, `skinning`, `fog`, `vsm`, `ssr`, `stereo`, `motionVectors`); `variantFilter` prunes unreachable ones. Recent releases migrated fog/DIR/DYN to *dynamic specialization constants*. [3][4] |
| Precision | Fragment default is `default` = "typically `high` on desktop, `medium` on mobile"; the PBR doc ships an fp16-specific `D_GGX` (`saturateMediump`). [3][5] |
| Tile-GPU specifics | MSAA resolve deferred to pass end; depth allocated multisampled, resolved by a pass the graph culls if unused; colour grading as a subpass (comments name `ARM_shader_framebuffer_fetch` as the cheaper route); `masked` auto-enables alpha-to-coverage; mobile-only defaults: multi-bounce AO **off**, specular AO **none**. [1][3][4] |

### 2. Godot 4 (Mobile renderer; Compatibility noted)

| Question | Finding |
|---|---|
| Per-frame passes | shadow atlases (directional + positional) → motion vectors (optional) → **"Render Opaque"** (clears colour+depth, one draw list) → **"Draw Sky"** → **"Render Transparent"** (same draw list, `PASS_MODE_COLOR_TRANSPARENT`) → tonemap as a **subpass** of that same render pass, or a separate "Post Process Pass" when subpassing is impossible. MSAA resolves automatically at render-pass end (`resolve_attachments`). [6] |
| Depth prepass | **None for opaques.** Opaques go straight through `PASS_MODE_COLOR`. Only a per-material `depth_prepass_alpha` mode (opaque depth pre-pass for transparent geometry) exists. [6][8] |
| Lighting | Forward single-pass. Per-pixel, but **vertex shading is forced by default on mobile** ("Force Vertex Shading … enabled by default on mobile platforms"). Limits: **8 omni/spot per mesh, 256 per view**, 8 directional; no cluster/tile binning. [7][8] |
| LOD / culling | Automatic mesh LOD generated on import (meshoptimizer), selected by a **screen-space pixel metric** (`mesh_lod_threshold`, per-node bias), plus visibility ranges and occlusion culling. No SSAO/SSR/TAA on Mobile. [8][9] |
| Material strategy | Über-shader + specialization constants (`directional_lights`, `use_directional_soft_shadows`, `disable_fog`, `luminance_multiplier` …) with a pipeline cache keyed by specialization; fp16 shader group enabled when the device supports it (`FEATURE_FP16_BIT`). [6] |
| Tile-GPU specifics | Colour format chosen as `A2B10G10R10` "to enable AFBC on mobile devices"; transparents merged into the opaque pass to avoid a store/load; tonemap as a subpass; MSAA resolved at pass end; Reverse-Z 24-bit depth. Compatibility renderer instead uses RGBA8, forward single-pass with multi-pass shadowed lights, 8 lights/mesh. [6][7] |

### 3. Unity URP (Forward path, brief)

| Question | Finding |
|---|---|
| Per-frame passes | setup/clear → optional `DepthOnly` / `DepthNormals` prepass → **opaque** (or GBuffer for deferred) → depth copy (scheduled *After Opaques*, *After Skybox* or *After Transparents*) → skybox → colour copy → **transparents** → UberPost → FinalPost (FXAA/upscale) → final blit. [10] |
| Depth prepass | **Off by default.** Depth Priming Mode = Disabled / Auto / Forced. Auto "performs depth priming only if a depth prepass already exists" and "isn't supported on Android, iOS and Apple TV". Priming is also unsupported with deferred, with MSAA, and "at runtime on mobile devices that use tile-based deferred rendering (TBDR)". Depth Texture Mode adds a prepass only if the depth texture is requested. [10] |
| Lighting | Forward: **max 9 lights per GameObject** (1 main per-pixel + 8 additional, per-pixel by default, switchable to per-vertex); per camera 32 additional on mobile, 256 desktop, 16 on GLES 3.0. Forward+ tiles the screen and bins lights per tile, removing the per-object limit. [11][12] |
| LOD / culling | LOD Group + cross-fade; frustum and occlusion culling; Unity 6 GPU Resident Drawer for instancing (not verified for mobile). |
| Material strategy | Über-shaders (Lit) + shader keyword variants; **SRP Batcher** keeps per-material constant buffers resident in GPU memory and batches bind/draw sequences — docs advise "use as few shader variants as possible". [13] |
| Tile-GPU specifics | The Universal Renderer page is explicit that a Copy Depth between opaque and transparent "causes a switch in render target", forcing the colour buffer (and MSAA data) to be stored to and reloaded from main memory; `After Transparents` "can lead to a significant improvement in memory bandwidth" on mobile. Tile-only/on-tile mode exists in newer URP source (`useTileOnlyMode`) but its docs were not readable (see flags). [10] |

### Deltas worth checking against a wgpu/V3D renderer

- **Depth prepass**: all three say *no* by default (Filament removed it, Godot never had one for opaques, URP gates it behind priming that TBDR excludes). Front-to-back sorting plus early-Z is the common answer.
- **One render pass**: Godot's `merge_transparent_pass` + subpass tonemap and Filament's subpass colour grading both exist to avoid tile store/load. On V3D that is the biggest lever: opaque + sky + transparent + resolve in one pass.
- **Discard** is avoided (Filament prefers alpha-to-coverage); `masked`/`discard` breaks early-Z and forces late depth writes on TBDR.
- **Precision**: Filament defaults fragments to mediump on mobile, Godot enables an fp16 shader group; WGSL has no mediump, but `f16` where the adapter allows it mirrors both.

### Flagged / not verified

- Filament pass order is a frame-graph declaration; no runtime frame was traced. "No mesh LOD" is a negative search result, not a vendor statement.
- Godot: whether Mobile ever emits a depth prepass beyond `depth_prepass_alpha`; purpose of the Mobile motion-vector pass (TAA is unsupported there).
- URP: exact per-frame order is inferred from `UniversalRendererRenderGraph.cs` and the renderer reference, not a single ordered spec. Unity 7 on-tile rendering docs redirected cross-origin and were not read.

### Sources

1. https://github.com/google/filament/blob/main/filament/src/details/Renderer.cpp
2. https://github.com/google/filament/blob/main/filament/src/RenderPass.cpp
3. https://github.com/google/filament/blob/main/docs/Materials.md.html
4. https://github.com/google/filament/blob/main/RELEASE_NOTES.md
5. https://google.github.io/filament/Filament.md.html
6. https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/forward_mobile/render_forward_mobile.cpp
7. https://docs.godotengine.org/en/stable/tutorials/rendering/renderers.html
8. https://docs.godotengine.org/en/stable/tutorials/shaders/shader_reference/spatial_shader.html
9. https://docs.godotengine.org/en/stable/tutorials/3d/mesh_lod.html
10. https://docs.unity3d.com/6000.0/Documentation/Manual/urp/urp-universal-renderer.html
11. https://docs.unity3d.com/6000.0/Documentation/Manual/urp/lighting/light-limits-in-urp.html
12. https://docs.unity3d.com/6000.0/Documentation/Manual/urp/rendering/forward-rendering-paths.html
13. https://docs.unity3d.com/6000.0/Documentation/Manual/SRPBatcher.html
14. https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.universal/Runtime/UniversalRendererRenderGraph.cs

---

## Appendix B: Bevy, three.js, vange-rs, bevy_terrain

Facts from source at pinned tags (Oct 2026) plus author docs. "None" below means a negative search of the renderer's pass list, not a vendor statement.

| Question | Bevy 0.20.1 | three.js r186 (WebGL) | vange-rs | bevy_terrain |
|---|---|---|---|---|
| Per-frame passes | optional prepass (depth / normal / motion / deferred) + depth downsample → **shadow passes per light view** → optional late prepass → **main opaque** → **main transparent** → tonemap → upscale. SSAO/bloom/SSR/DoF add their own passes. | shadow-map passes per light → **one scene pass**: opaque → transmissive (offscreen re-render of opaques) → transparent → optional post chain (EffectComposer / `PostProcessing`). | optional 1024² shadow depth pass → **one main pass** (terrain → vehicles → water → debug lines) → egui UI pass. RayVoxel terrain adds compute bake/mip passes. | Bevy shadow passes → **compute tiling prepass** per view → Bevy main opaque pass containing the terrain's **single indirect draw** → tonemap/upscale. |
| Depth prepass | **Off by default.** Opaque pass writes depth. Add `DepthPrepass` (or `NormalPrepass` / `MotionVectorPrepass` / `DeferredPrepass`) to the camera to get one. Unsupported on WebGL2 (`DEPTH_PREPASS_TEXTURE_SUPPORTED = false`). | **None.** Depth is written by the opaque pass; a depth-only pass must be authored with a depth material. | **None.** | **None** — its "prepass" is compute/culling, not depth. |
| Sorting / batching | Opaque + alpha-mask are `BinnedPhaseItem`s: binned by (pipeline, draw fn, material bind group, mesh slabs, lightmap), **no depth sort**. Transparent is `SortedPhaseItem` on distance, ascending = **back-to-front**. GPU preprocessing/indirect draws by default where the adapter supports it; `NoAutomaticBatching` / `NoIndirectDrawing` opt out. | Opaque: `painterSortStable` = groupOrder, renderOrder, **ascending projected z** (front-to-back), id. Transparent: `reversePainterSortStable` = descending z (back-to-front). Per object (bounding-sphere centre), not per triangle; `sortObjects = true`. | No general sort. Ray/voxel/slice paths = 1 draw. "Mesh" path = 1 draw per visible chunk, LOD picked per chunk. Vehicles = 1 instanced draw per unique mesh. | No sort: compute prepass refines the tile tree and writes indirect args; one indirect draw covers all tiles. |
| Lighting | Clustered-forward **PBR**, per-fragment; deferred optional. | `MeshLambertMaterial` = Lambert, **per-fragment since r144** (r142 and earlier were Gouraud), no specular → cheaper than `MeshStandardMaterial` (GGX + IBL + env BRDF), same light loops. | Per-pixel. Ray path marches the heightfield and traces shadow rays in the same fragment shader. One directional + optional local light + fog. | Per-pixel Bevy PBR via `apply_pbr_lighting`, but roughness forced to 1.0 and reflectance 0 → effectively diffuse-only. |
| LOD / geometry | **No mesh LOD.** Only `MeshletPlugin` (virtual geometry, experimental): requires `TEXTURE_INT64_ATOMIC`, Vulkan/Metal only, incompatible with MSAA, described as higher base overhead than the standard renderer. | **No built-in LOD** in the core renderer; `LOD` object + manual swap. Draw-call reduction via `InstancedMesh` / `BatchedMesh`. | Mesh path: per-chunk geometry + per-chunk LOD, wrap tiles bounded by `MAX_TILE_RADIUS`. Ray path: "maximum mipmaps" acceleration. Voxel path: 32-level mip chain. | UDLOD quadtree (uniform distance-dependent LOD), CDLOD-style vertex morph, `additional_refinement` for close-up detail; chunked-clipmap tile textures with duplicated borders + mips. |
| CPU-side dominates | Phase/bin building, extraction, pipeline specialization, per-view uniform prep. Draw count is not the bottleneck. | **Draw calls + per-object uniform uploads + JS state changes**; `renderer.info.render.calls` is the meter. | Draw count is tiny (≈3–15). Cost is per-frame buffer writes: water triangles rebuilt and staged every frame, vehicle instances re-uploaded, 2 uniform writes, terrain dirty-chunk bookkeeping. | Tile streaming / atlas uploads (author flags this as inefficient: textures duplicated through Bevy's image abstraction), plus compute dispatches. |

### Details worth copying into the comparison

**Bevy.** The 3D frame is a schedule, not a graph: `Core3dSystems::{Prepass, MainPass, EarlyPostProcess, PostProcess}` chained weakly. Prepass order is `early_prepass → early_deferred_prepass → late_prepass → late_deferred_prepass`; shadow systems are inserted *between* those stages (`per_view_shadow_pass::<EARLY>` before depth downsample, `::<LATE>` before `MainPass`), so shadow culling can consume prepass depth. Opaque and transparent are chained in `MainPass`. Depth format is `Depth32Float` despite an in-source comment that Vulkan recommends 24-bit. `Msaa` defaults to `Sample4`.

**three.js.** `render()` projects the scene, sorts, renders shadow maps, then `opaque → transmissive → transparent`; transmissive materials trigger `renderTransmissionPass`, which renders the opaque list into `_transmissionRenderTarget` first — the one implicit offscreen pass. WebGPU (`WebGPURenderer` + node materials) keeps the same list order; post-processing is a separate `PostProcessing` pass chain. There is no engine-level depth prepass in either backend.

**vange-rs.** `draw_world` opens exactly two render passes (shadow, main) and reuses one bind group for terrain/water. Terrain is an enum of strategies — `Ray` (one fullscreen-ish primitive, ray-marched per pixel), `RayVoxel` (3D-texture raymarch, one `draw(0..3)`), `Slice`, `Paint`, `Scatter`, `Mesh`. Shipped `render-full.ron` selects `RayVoxelTraced` with a 1024² shadow map; code defaults are `RayTraced` with `size: 0` (no shadow pass). Water is a CPU-built triangle list of flood tiles, one draw, depth-write off, alpha blended, sampling the terrain flood texture.

**bevy_terrain.** Author docs: tiles are refined in a compute prepass, then "drawn using a single draw indirect call and morphed together" — one draw for the whole terrain, per view. Its compute pass even uses `dispatch_workgroups_indirect`. Shading is Bevy PBR, so it inherits clustered lighting and shadows for free.

### Deltas vs a Pi 5 / V3D wgpu renderer

- All four avoid a depth prepass for opaques; Bevy's is opt-in and three.js/vange/bevy_terrain have none. Front-to-back ordering is Bevy's *transparent* rule only — opaque relies on binned batching, not Z.
- Only Bevy and bevy_terrain have a real "one pass + post" separation; vange-rs draws straight to the surface with a fog-colour clear (no HDR target, no tonemap).
- A single indirect terrain draw (bevy_terrain) and one-draw raymarching (vange-rs) are the two ways these projects keep draw counts near 1; Bevy instead batches many meshes into few multi-draws.
- `Depth32Float` (Bevy) vs a 24-bit depth is a bandwidth choice worth checking on V3D.

### Flagged / not verified

- No official Bevy low-end/mobile performance guide found for 0.20.1. Documented knobs only: `Msaa` defaults to 4x, WebGL2 cannot do the depth prepass, `WgpuSettingsPriority::WebGL2` clamps limits, meshlet warns of high base overhead, `NoIndirectDrawing` disables GPU preprocessing.
- vange-rs pass count is read from `draw_world`; the UI (egui) pass is added by the app shell and was not traced.
- bevy_terrain's "single indirect draw" is the author's claim; not measured with a GPU capture.
- three.js "front-to-back" is inferred from `painterSortStable` comparing projected z ascending, not from a doc sentence.
- screen-13 was checked and excluded: crate 0.13.0 is now "an easy-to-use **Vulkan** rendering engine", so it is no longer a wgpu cousin.

### Sources

**Bevy** — [core_3d pass wiring](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_core_pipeline/src/core_3d/mod.rs) · [Core3dSystems](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_core_pipeline/src/schedule.rs) · [prepass docs](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_core_pipeline/src/prepass/mod.rs) · [shadow scheduling](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_pbr/src/lib.rs) · [batching](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_render/src/batching/mod.rs) · [meshlet](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_pbr/src/meshlet/mod.rs)

**three.js** — [RenderList comparators](https://github.com/mrdoob/three.js/blob/r186/src/renderers/common/RenderList.js) · [WebGLRenderer frame](https://github.com/mrdoob/three.js/blob/r186/src/renderers/WebGLRenderer.js) · [Lambert fragment shader](https://github.com/mrdoob/three.js/blob/r186/src/renderers/shaders/ShaderLib/meshlambert.glsl.js) · [Lambert docs ("per-fragment shading")](https://github.com/mrdoob/three.js/blob/r186/src/materials/MeshLambertMaterial.js) · [InstancedMesh](https://threejs.org/docs/#api/en/objects/InstancedMesh)

**vange-rs** — [render/mod.rs](https://github.com/kvark/vange-rs/blob/master/src/render/mod.rs) · [terrain/mod.rs](https://github.com/kvark/vange-rs/blob/master/src/render/terrain/mod.rs) · [water.rs](https://github.com/kvark/vange-rs/blob/master/src/render/water.rs) · [Rendering-Techniques wiki](https://github.com/kvark/vange-rs/wiki/Rendering-Techniques) · [voxel blog post](https://vange.rs/2022/11/08/voxels.html)

**bevy_terrain** — [implementation.md](https://github.com/kurtkuehnert/bevy_terrain/blob/main/docs/implementation.md) · [render/mod.rs](https://github.com/kurtkuehnert/bevy_terrain/blob/main/src/render/mod.rs) · [tiling_prepass.rs](https://github.com/kurtkuehnert/bevy_terrain/blob/main/src/render/tiling_prepass.rs) · [fragment.wgsl](https://github.com/kurtkuehnert/bevy_terrain/blob/main/src/shaders/render/fragment.wgsl) · [thesis](https://github.com/kurtkuehnert/terrain_renderer/blob/main/Thesis.pdf)
