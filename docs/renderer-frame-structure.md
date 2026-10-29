# Frame structure: Filament, Godot 4 Mobile, URP Forward

Facts are from each renderer's source (`main`/`master`, Oct 2026) plus vendor docs. All three build a pass graph and **cull disabled passes**, so "order" is declaration/dependency order, not a guaranteed trace.

## 1. Google Filament (Android-first)

| Question | Finding |
|---|---|
| Per-frame passes | shadow maps (CSM + spot/point atlases) → structure (depth+normal, only if SSAO/contact shadows need it) → SSAO → SSR → **one colour pass, opaques then transparents** → resolved depth (only if non-MS depth is consumed) → motion vectors → TAA → DoF → bloom → colour grading (usually a *subpass*) → FXAA → upscale/blit. SSAO/SSR/DoF/TAA all default off. [1][2] |
| Depth prepass | **No.** "The depth prepass setting in View is now ignored and deprecated" (v1.4.5); "Removed depth-prepass related APIs" (v1.5.0). Instead opaque commands sort front-to-back in Z buckets, then by material ("bucket objects by Z, front-to-back and then sort by material"), relying on early-Z. Transparents sort back-to-front honouring explicit blend order. [2][4] |
| Lighting | Clustered: froxel binning (`Froxelizer`), up to **256** punctual lights ("more than the GPU buffer allows (i.e. 256)"), one dominant directional with shadows plus optional extra directionals. Per-pixel shading; IBL is the main ambient term. [1][3] |
| LOD / culling | No mesh-LOD system found (texture LOD bias only): per-view frustum culling of renderables and lights on the job system, visibility layers, automatic draw-call instancing. LOD is the app's job. [1][2] |
| Material strategy | One shader per material, compiled **offline** by `matc` into a package holding **all variants** (`directionalLighting`, `dynamicLighting`, `shadowReceiver`, `skinning`, `fog`, `vsm`, `ssr`, `stereo`, `motionVectors`); `variantFilter` prunes unreachable ones. Recent releases migrated fog/DIR/DYN to *dynamic specialization constants*. [3][4] |
| Precision | Fragment default is `default` = "typically `high` on desktop, `medium` on mobile"; the PBR doc ships an fp16-specific `D_GGX` (`saturateMediump`). [3][5] |
| Tile-GPU specifics | MSAA resolve deferred to pass end; depth allocated multisampled, resolved by a pass the graph culls if unused; colour grading as a subpass (comments name `ARM_shader_framebuffer_fetch` as the cheaper route); `masked` auto-enables alpha-to-coverage; mobile-only defaults: multi-bounce AO **off**, specular AO **none**. [1][3][4] |

## 2. Godot 4 (Mobile renderer; Compatibility noted)

| Question | Finding |
|---|---|
| Per-frame passes | shadow atlases (directional + positional) → motion vectors (optional) → **"Render Opaque"** (clears colour+depth, one draw list) → **"Draw Sky"** → **"Render Transparent"** (same draw list, `PASS_MODE_COLOR_TRANSPARENT`) → tonemap as a **subpass** of that same render pass, or a separate "Post Process Pass" when subpassing is impossible. MSAA resolves automatically at render-pass end (`resolve_attachments`). [6] |
| Depth prepass | **None for opaques.** Opaques go straight through `PASS_MODE_COLOR`. Only a per-material `depth_prepass_alpha` mode (opaque depth pre-pass for transparent geometry) exists. [6][8] |
| Lighting | Forward single-pass. Per-pixel, but **vertex shading is forced by default on mobile** ("Force Vertex Shading … enabled by default on mobile platforms"). Limits: **8 omni/spot per mesh, 256 per view**, 8 directional; no cluster/tile binning. [7][8] |
| LOD / culling | Automatic mesh LOD generated on import (meshoptimizer), selected by a **screen-space pixel metric** (`mesh_lod_threshold`, per-node bias), plus visibility ranges and occlusion culling. No SSAO/SSR/TAA on Mobile. [8][9] |
| Material strategy | Über-shader + specialization constants (`directional_lights`, `use_directional_soft_shadows`, `disable_fog`, `luminance_multiplier` …) with a pipeline cache keyed by specialization; fp16 shader group enabled when the device supports it (`FEATURE_FP16_BIT`). [6] |
| Tile-GPU specifics | Colour format chosen as `A2B10G10R10` "to enable AFBC on mobile devices"; transparents merged into the opaque pass to avoid a store/load; tonemap as a subpass; MSAA resolved at pass end; Reverse-Z 24-bit depth. Compatibility renderer instead uses RGBA8, forward single-pass with multi-pass shadowed lights, 8 lights/mesh. [6][7] |

## 3. Unity URP (Forward path, brief)

| Question | Finding |
|---|---|
| Per-frame passes | setup/clear → optional `DepthOnly` / `DepthNormals` prepass → **opaque** (or GBuffer for deferred) → depth copy (scheduled *After Opaques*, *After Skybox* or *After Transparents*) → skybox → colour copy → **transparents** → UberPost → FinalPost (FXAA/upscale) → final blit. [10] |
| Depth prepass | **Off by default.** Depth Priming Mode = Disabled / Auto / Forced. Auto "performs depth priming only if a depth prepass already exists" and "isn't supported on Android, iOS and Apple TV". Priming is also unsupported with deferred, with MSAA, and "at runtime on mobile devices that use tile-based deferred rendering (TBDR)". Depth Texture Mode adds a prepass only if the depth texture is requested. [10] |
| Lighting | Forward: **max 9 lights per GameObject** (1 main per-pixel + 8 additional, per-pixel by default, switchable to per-vertex); per camera 32 additional on mobile, 256 desktop, 16 on GLES 3.0. Forward+ tiles the screen and bins lights per tile, removing the per-object limit. [11][12] |
| LOD / culling | LOD Group + cross-fade; frustum and occlusion culling; Unity 6 GPU Resident Drawer for instancing (not verified for mobile). |
| Material strategy | Über-shaders (Lit) + shader keyword variants; **SRP Batcher** keeps per-material constant buffers resident in GPU memory and batches bind/draw sequences — docs advise "use as few shader variants as possible". [13] |
| Tile-GPU specifics | The Universal Renderer page is explicit that a Copy Depth between opaque and transparent "causes a switch in render target", forcing the colour buffer (and MSAA data) to be stored to and reloaded from main memory; `After Transparents` "can lead to a significant improvement in memory bandwidth" on mobile. Tile-only/on-tile mode exists in newer URP source (`useTileOnlyMode`) but its docs were not readable (see flags). [10] |

## Deltas worth checking against a wgpu/V3D renderer

- **Depth prepass**: all three say *no* by default (Filament removed it, Godot never had one for opaques, URP gates it behind priming that TBDR excludes). Front-to-back sorting plus early-Z is the common answer.
- **One render pass**: Godot's `merge_transparent_pass` + subpass tonemap and Filament's subpass colour grading both exist to avoid tile store/load. On V3D that is the biggest lever: opaque + sky + transparent + resolve in one pass.
- **Discard** is avoided (Filament prefers alpha-to-coverage); `masked`/`discard` breaks early-Z and forces late depth writes on TBDR.
- **Precision**: Filament defaults fragments to mediump on mobile, Godot enables an fp16 shader group; WGSL has no mediump, but `f16` where the adapter allows it mirrors both.

## Flagged / not verified

- Filament pass order is a frame-graph declaration; no runtime frame was traced. "No mesh LOD" is a negative search result, not a vendor statement.
- Godot: whether Mobile ever emits a depth prepass beyond `depth_prepass_alpha`; purpose of the Mobile motion-vector pass (TAA is unsupported there).
- URP: exact per-frame order is inferred from `UniversalRendererRenderGraph.cs` and the renderer reference, not a single ordered spec. Unity 7 on-tile rendering docs redirected cross-origin and were not read.

## Sources

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
