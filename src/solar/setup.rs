//! Building the frame: its pipelines, bind groups and textures
//! (`Solar::with_seed`), and the starfield baked a few rows a frame.

use super::*;

/// One of the frame's pipelines (`Solar::with_seed`): its entry points and
/// what sets it apart; the layout, the target and the depth format are the
/// frame's own. [`FrameDraw::new`] gives the usual case: no blending, every
/// channel written, the depth mask tested but not written, nothing culled.
#[derive(Clone, Copy)]
pub(super) struct FrameDraw<'a> {
    pub(super) vs: &'a str,
    pub(super) fs: &'a str,
    pub(super) blend: Option<wgpu::BlendState>,
    pub(super) constants: &'a [(&'a str, f64)],
    /// Whether it writes the depth; `None` for no depth at all (drawn
    /// outside the frame's pass: the starfield bake).
    pub(super) depth: Option<bool>,
    pub(super) writes: wgpu::ColorWrites,
    /// Strictly in front of the mask (`Less`), for passes drawn only off
    /// their own disc.
    pub(super) strict: bool,
    pub(super) cull: Option<wgpu::Face>,
    pub(super) buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
}

impl<'a> FrameDraw<'a> {
    pub(super) fn new(vs: &'a str, fs: &'a str) -> Self {
        Self {
            vs,
            fs,
            blend: None,
            constants: &[],
            depth: Some(false),
            writes: wgpu::ColorWrites::ALL,
            strict: false,
            cull: None,
            buffers: &[],
        }
    }
}

impl Solar {
    pub(crate) fn with_seed(gpu: &Graphics, seed: u64) -> Self {
        let device = &gpu.device;
        let streamer = Streamer::new(gpu);
        let d2 = wgpu::TextureViewDimension::D2;
        let array = wgpu::TextureViewDimension::D2Array;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar frame"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, d2),
                sampler_entry(2),
                texture_entry(3, d2),
                texture_entry(4, d2),
                texture_entry(5, d2),
                texture_entry(6, array),
                texture_entry(7, array),
                texture_entry(8, d2),
                texture_entry(9, d2),
                texture_entry(10, d2),
                texture_entry(11, array),
                texture_entry(12, array),
                texture_entry(13, d2),
                texture_entry(14, wgpu::TextureViewDimension::Cube),
                texture_entry(15, wgpu::TextureViewDimension::Cube),
            ],
        });
        // The rocks' crater maps, a group of their own for the models: in the
        // frame's group they would be the seventeenth texture a stage, over
        // the V3D's limit of sixteen for the pipelines that add a group.
        let crater_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar rock craters"),
            entries: &[texture_entry(2, array)],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar frame"),
            source: wgpu::ShaderSource::Wgsl(FRAME_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar frame"),
            bind_group_layouts: &[Some(&layout), Some(&streamer.noise().layout)],
            immediate_size: 0,
        });
        let model_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar models"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&streamer.noise().layout),
                Some(&crater_layout),
            ],
            immediate_size: 0,
        });
        // Until the models' thread has them, the rocks' maps are a texel each.
        let craters = rock_maps(gpu, &crater_layout, None);
        // The nebula's and the plasma storm's gas, a group of its own for
        // their two pipelines (as the craters are, for the same limit).
        let gas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar gas"),
            entries: &[texture_entry(3, wgpu::TextureViewDimension::Cube)],
        });
        let gas_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar gas"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&streamer.noise().layout),
                Some(&gas_layout),
            ],
            immediate_size: 0,
        });
        let gas = [bake_gas, bake_storm].map(|bake| {
            let texture = bake(gpu, &shader, streamer.noise());
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Solar gas"),
                layout: &gas_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });
            (texture, group)
        });
        // Every frame pipeline tests the depth mask (see `draw`); the mask's
        // own pipeline writes it and nothing else. Only the meshes cull.
        let make = |label: &str, d: FrameDraw| {
            let options = wgpu::PipelineCompilationOptions {
                constants: d.constants,
                ..Default::default()
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(d.vs),
                    buffers: d.buffers,
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(d.fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: d.blend,
                        write_mask: d.writes,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: d.cull,
                    ..Default::default()
                },
                // None: drawn outside the frame's pass (the starfield bake).
                depth_stencil: d.depth.map(|write| wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write),
                    // Strictly in front for passes that draw only off their
                    // own disc (the limb): its mask then rejects the disc.
                    depth_compare: Some(if d.strict {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::LessEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
            (label.to_string(), made)
        };
        // A mapped world's mesh: its back faces culled (counter-clockwise
        // seen from outside, see `grid_indices`).
        let globe = |label: &str,
                     fs: &str,
                     blend: Option<wgpu::BlendState>,
                     constants: &[(&str, f64)],
                     writes: wgpu::ColorWrites| {
            let draw = FrameDraw {
                blend,
                constants,
                writes,
                cull: Some(wgpu::Face::Back),
                ..FrameDraw::new("vs_globe", fs)
            };
            make(label, draw)
        };
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        blend: Option<wgpu::BlendState>,
                        constants: &[(&str, f64)]| {
            make(
                label,
                FrameDraw {
                    blend,
                    constants,
                    ..FrameDraw::new(vs, fs)
                },
            )
        };
        let over = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        // The nebula and the plasma storm, read from the gas map (`fs_gas`),
        // and the storm laid over the sky (`fs_coarse`): each in its own
        // layout, into the frame (tested against its depth) or not.
        let gas_pipeline = |label: &str,
                            layout: &wgpu::PipelineLayout,
                            fs: &str,
                            constants: &[(&str, f64)],
                            format: wgpu::TextureFormat,
                            blend: Option<wgpu::BlendState>,
                            depth: bool| {
            let options = wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_background"),
                    buffers: &[],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth.then_some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
            (label.to_string(), made)
        };
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        // Twice the product: the shader outputs half the factor, so the 8-bit
        // target's clamp to 1 still lets it brighten (up to 2x) as well as
        // darken.
        let double_multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Src,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let mut pipelines = std::collections::HashMap::new();
        // The depth mask: each body's solid disc, at its distance rank.
        let none = wgpu::ColorWrites::empty();
        pipelines.insert(
            Pipe::Mask,
            make(
                "Solar mask",
                FrameDraw {
                    depth: Some(true),
                    writes: none,
                    ..FrameDraw::new("vs_body", "fs_mask")
                },
            ),
        );
        // The frame's corners, which the round display never shows.
        pipelines.insert(
            Pipe::Corners,
            make(
                "Solar corners",
                FrameDraw {
                    depth: Some(true),
                    writes: none,
                    ..FrameDraw::new("vs_corners", "fs_corners")
                },
            ),
        );
        pipelines.insert(
            Pipe::MaskInscribed,
            make(
                "Solar mask inscribed",
                FrameDraw {
                    depth: Some(true),
                    writes: none,
                    ..FrameDraw::new("vs_mask_inscribed", "fs_mask_inscribed")
                },
            ),
        );
        // For the Pi estimate only: what survives the mask, shaded by nothing.
        pipelines.insert(
            Pipe::Probe,
            make(
                "Solar probe",
                FrameDraw {
                    writes: none,
                    ..FrameDraw::new("vs_body", "fs_probe")
                },
            ),
        );
        pipelines.insert(
            Pipe::ProbeStrict,
            make(
                "Solar probe strict",
                FrameDraw {
                    writes: none,
                    strict: true,
                    ..FrameDraw::new("vs_body", "fs_probe")
                },
            ),
        );
        pipelines.insert(
            Pipe::ProbeFull,
            make(
                "Solar probe full",
                FrameDraw {
                    writes: none,
                    ..FrameDraw::new("vs_background", "fs_probe_full")
                },
            ),
        );
        // The starfield, baked once a system, and the background reading it.
        pipelines.insert(
            Pipe::Bake,
            make(
                "Solar starfield bake",
                FrameDraw {
                    depth: None,
                    ..FrameDraw::new("vs_bake", "fs_bake_stars")
                },
            ),
        );
        pipelines.insert(
            Pipe::BakeSky,
            make(
                "Solar sky cube bake",
                FrameDraw {
                    depth: None,
                    ..FrameDraw::new("vs_bake", "fs_bake_sky")
                },
            ),
        );
        // The baked background reads the cube and nothing else, the star's
        // glare added over it round the star.
        pipelines.insert(
            Pipe::SpaceBaked,
            pipeline(
                "Solar space baked",
                "vs_background",
                "fs_space_baked",
                None,
                &[],
            ),
        );
        for (name, label, vs) in [
            (Pipe::Glare, "Solar glare", "vs_glare"),
            (Pipe::GlareFull, "Solar glare full", "vs_glare_full"),
        ] {
            pipelines.insert(name, pipeline(label, vs, "fs_glare", add, &[]));
        }
        let (label, vs, fs) = ("Solar probe glare", "vs_glare", "fs_probe_glare");
        let made = make(
            label,
            FrameDraw {
                writes: none,
                ..FrameDraw::new(vs, fs)
            },
        );
        pipelines.insert(Pipe::ProbeGlare, made);
        pipelines.insert(
            Pipe::Space,
            pipeline("Solar space", "vs_background", "fs_space", None, &[]),
        );
        pipelines.insert(
            Pipe::Dust,
            pipeline("Solar burn dust", "vs_dust", "fs_dust", add, &[]),
        );
        pipelines.insert(
            Pipe::Rim,
            pipeline("Solar burn rim", "vs_rim", "fs_rim", add, &[]),
        );
        // The baked sky with the next jump's star marked, out to the jump
        // point.
        pipelines.insert(
            Pipe::Target,
            pipeline(
                "Solar space target",
                "vs_background",
                "fs_space_target",
                None,
                &[],
            ),
        );
        // Each drive's light over the whole frame, and with the flare at the
        // heading for the jump's last second.
        for (k, name) in voyage::LOOKS.iter().enumerate() {
            let k = k as u32 + 1;
            // Its start-up at the jump point, over the sky, rising into its
            // light: the drives' only, a gate or wormhole throwing the ship
            // on itself. A pipeline a stage of it (`STAGE`), each with only
            // what shows then.
            if (k as usize) <= tour::DRIVES.len() {
                // The engagement's added over the drive's light rising.
                let stages = tour::SPOOL_STAGES.len() - 1;
                for stage in 1..=stages {
                    let constants = [("WARP", k as f64), ("STAGE", stage as f64)];
                    let made = pipeline(
                        &format!("Solar spool {name} stage {stage}"),
                        "vs_background",
                        "fs_spool",
                        if stage == stages { add } else { None },
                        &constants,
                    );
                    pipelines.insert(Pipe::Spool { drive: k, stage }, made);
                }
                let constants = [("WARP", k as f64), ("RISING", 1.0)];
                let made = pipeline(
                    &format!("Solar spool {name} light"),
                    "vs_background",
                    "fs_warp",
                    None,
                    &constants,
                );
                pipelines.insert(Pipe::SpoolLight(k), made);
            }
            for (flare, label) in [
                (false, format!("Solar warp {name}")),
                (true, format!("Solar warp flare {name}")),
            ] {
                let key = Pipe::Warp { look: k, flare };
                let constants = [("WARP", k as f64), ("FLARE", f64::from(u8::from(flare)))];
                let made = pipeline(&label, "vs_background", "fs_warp", None, &constants);
                pipelines.insert(key, made);
            }
            // Its light torn open over the next system as the jump drops
            // out: ahead of the front opaque, before everything else, and
            // writing the depth so nothing it hides is shaded; behind the
            // front blended over everything drawn.
            let constants = [("WARP", k as f64)];
            let label = format!("Solar drop ahead {name}");
            let ahead = make(
                &label,
                FrameDraw {
                    constants: &constants,
                    depth: Some(true),
                    ..FrameDraw::new("vs_drop_ahead", "fs_drop_ahead")
                },
            );
            pipelines.insert(Pipe::DropAhead(k), ahead);
            pipelines.insert(
                Pipe::DropOut(k),
                pipeline(
                    &format!("Solar drop out {name}"),
                    "vs_drop_behind",
                    "fs_drop_out",
                    over,
                    &constants,
                ),
            );
        }
        // Between the legs of a jump: each stopover's own draw over the sky
        // (`fs_event`; the black hole in place of it, `fs_coarse`), rocks or debris
        // round the ship for some; what is seen inside a leg over the
        // drive's light (`fs_flux`); and the ship's own effects over
        // everything (`fs_overlay`): the alarm, hits on the shield and
        // flashes.
        for event in voyage::Event::ALL {
            let constants = [("EVENT", event.id() as f64)];
            let (key, label) = (
                Pipe::Event(event.id()),
                format!("Solar event {}", event.slug()),
            );
            let (vs, fs, blend) = ("vs_background", "fs_event", over);
            // The leviathans and the encounters drawn in parts are drawn in pieces
            // instead (below).
            if event == voyage::Event::Nebula {
                let (gases, frame) = (&gas_pipeline_layout, wgpu::TextureFormat::Rgba8UnormSrgb);
                let made = gas_pipeline(&label, gases, "fs_gas", &constants, frame, over, true);
                pipelines.insert(key, made);
            } else if event.coarse() {
                // Below, with the layout it shares with the leviathans' glow.
            } else if event != voyage::Event::Leviathans && event_parts(event, [0; 3]).is_none() {
                pipelines.insert(key, pipeline(&label, vs, fs, blend, &constants));
            }
            if event.rocks() {
                let label = format!("Solar rocks {}", event.slug());
                let made = make(
                    &label,
                    FrameDraw {
                        blend: over,
                        constants: &constants,
                        depth: Some(true),
                        ..FrameDraw::new("vs_rock", "fs_rock")
                    },
                );
                pipelines.insert(Pipe::Rocks(event.id()), made);
            }
        }
        // The gate and the Dyson swarm drawn bounded instead, a pipeline a
        // part (`PART`, drawn as `event_parts` lists them): the gate's field
        // a polygon in the ring's plane, its buoys and nodes a quad each; the
        // swarm's panels from `dyson_panels`, the near side's and the far
        // side's told apart by their facing (seen from outside a panel winds
        // clockwise on the frame), at depths of their own strictly in front
        // of what is there (see `fs_dyson_panel`), and its star's glow and
        // disc each on the square it reaches across.
        let (near, far) = (Some(wgpu::Face::Front), Some(wgpu::Face::Back));
        let panel_attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4
        ];
        let panel = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Panel>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &panel_attributes,
        })];
        let gate = voyage::Event::Gate;
        for (name, vs, fs, part, blend) in [
            ("field", "vs_gate_field", "fs_gate_field", 0, over),
            ("buoys", "vs_glow", "fs_gate_glint", 1, add),
            ("nodes", "vs_glow", "fs_gate_glint", 2, add),
        ] {
            let label = format!("Solar event {} {name}", gate.slug());
            let made = pipeline(&label, vs, fs, blend, &[("PART", part as f64)]);
            pipelines.insert(Pipe::EventPart(gate.id(), name), made);
        }
        let hive = voyage::Event::HiveCube;
        for (name, part) in [("haze", 1), ("beam", 2)] {
            let label = format!("Solar event {} {name}", hive.slug());
            let made = pipeline(
                &label,
                "vs_cube_glow",
                "fs_cube_glow",
                add,
                &[("PART", part as f64)],
            );
            pipelines.insert(Pipe::EventPart(hive.id(), name), made);
        }
        let dyson = voyage::Event::DysonSwarm;
        for (name, part, blend, write, cull) in [
            ("near", 1, over, true, near),
            ("near lit", 2, add, false, near),
            ("far", 3, over, true, far),
            ("far lit", 4, add, false, far),
            ("glow", 1, add, false, None),
            ("star", 2, add, false, None),
        ] {
            let label = format!("Solar event {} {name}", dyson.slug());
            let constants = [("PART", part as f64)];
            let (vs, fs, buffers) = match cull {
                Some(_) => ("vs_dyson_panel", "fs_dyson_panel", &panel[..]),
                None => ("vs_dyson_star", "fs_dyson_star", &[][..]),
            };
            let (depth, strict) = (Some(write), true);
            let made = make(
                &label,
                FrameDraw {
                    blend,
                    constants: &constants,
                    depth,
                    strict,
                    cull,
                    buffers,
                    ..FrameDraw::new(vs, fs)
                },
            );
            pipelines.insert(Pipe::EventPart(dyson.id(), name), made);
        }
        for flux in voyage::Flux::ALL {
            let constants = [("EVENT", flux.id() as f64)];
            let label = format!("Solar flux {}", flux.slug());
            pipelines.insert(
                Pipe::Flux(flux.id()),
                pipeline(&label, "vs_background", "fs_flux", add, &constants),
            );
        }
        // The leviathans in pieces, each on quads bounded to where its light
        // reaches (`PART`): every lobe's core, the fins and the filaments over
        // the sky; the lobes' wide halos into a texture a quarter of the
        // frame's resolution, added over the sky with the plankton.
        for (part, name) in [(1, "lobes"), (2, "fins"), (3, "filaments")] {
            let constants = [("PART", part as f64)];
            let (vs, label) = ("vs_leviathan", format!("Solar event leviathans {name}"));
            let made = pipeline(&label, vs, "fs_leviathan", add, &constants);
            pipelines.insert(Pipe::Leviathan(part), made);
            let label = format!("Solar probe leviathans {name}");
            let fs = "fs_probe_leviathan";
            let made = make(
                &label,
                FrameDraw {
                    constants: &constants,
                    writes: none,
                    ..FrameDraw::new(vs, fs)
                },
            );
            pipelines.insert(Pipe::ProbeLeviathan(part), made);
        }
        let halo_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar leviathan halos"),
            entries: &[texture_entry(0, d2), sampler_entry(1)],
        });
        let glow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar leviathan glow"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&streamer.noise().layout),
                Some(&halo_layout),
            ],
            immediate_size: 0,
        });
        let leviathan = |label: &str,
                         layout: &wgpu::PipelineLayout,
                         (vs, fs): (&str, &str),
                         format: wgpu::TextureFormat,
                         writes: wgpu::ColorWrites,
                         depth: bool| {
            let constants = [("PART", 4.0)];
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: add,
                        write_mask: writes,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth.then_some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
            (label.to_string(), made)
        };
        let (halo, all) = (LEVIATHAN_HALO_FORMAT, wgpu::ColorWrites::ALL);
        let shaders = ("vs_leviathan", "fs_leviathan");
        let made = leviathan(
            "Solar event leviathans halos",
            &pipeline_layout,
            shaders,
            halo,
            all,
            false,
        );
        pipelines.insert(Pipe::Leviathan(4), made);
        let shaders = ("vs_leviathan", "fs_probe_leviathan");
        let made = leviathan(
            "Solar probe leviathans halos",
            &pipeline_layout,
            shaders,
            halo,
            none,
            false,
        );
        pipelines.insert(Pipe::ProbeLeviathan(4), made);
        let shaders = ("vs_leviathan_glow", "fs_leviathan_glow");
        let frame = wgpu::TextureFormat::Rgba8UnormSrgb;
        let made = leviathan(
            "Solar event leviathans glow",
            &glow_layout,
            shaders,
            frame,
            all,
            true,
        );
        pipelines.insert(Pipe::LeviathanGlow, made);
        // The plasma storm and the remnant at half the frame's resolution
        // before its pass, then laid over the sky.
        for event in voyage::Event::ALL.into_iter().filter(|e| e.coarse()) {
            let constants = [("EVENT", event.id() as f64)];
            let label = format!("Solar event {} coarse", event.slug());
            let (layout, fs) = if event.gas() {
                (&gas_pipeline_layout, "fs_gas")
            } else {
                (&pipeline_layout, "fs_event")
            };
            let small = LEVIATHAN_HALO_FORMAT;
            let made = gas_pipeline(&label, layout, fs, &constants, small, None, false);
            pipelines.insert(Pipe::Coarse(event.id()), made);
            let label = format!("Solar event {}", event.slug());
            // The black hole replaces the sky: nothing to blend over.
            let blend = if event.replaces_sky() { None } else { over };
            let made = gas_pipeline(
                &label,
                &glow_layout,
                "fs_coarse",
                &constants,
                frame,
                blend,
                true,
            );
            pipelines.insert(Pipe::Event(event.id()), made);
        }
        // The battle's starships as meshes (`ships`): from a vertex buffer,
        // placed by the shader's own battle (`vs_ship`), writing the depth.
        let ship_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4];
        let ship_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<models::ShipVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ship_attributes,
        };
        let ship_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Solar ships"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ship"),
                buffers: &[Some(ship_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ship"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        });
        pipelines.insert(Pipe::Ships, ("Solar ships".to_string(), ship_pipeline));
        // Every other model family's meshes, instanced: a pipeline each.
        let instance_attributes =
            wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4];
        for (f, name) in models::FAMILIES.iter().enumerate() {
            let family = f as u32 + 1;
            let mesh_layout = wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<models::ShipVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ship_attributes,
            };
            let instance_layout = wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<models::Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            };
            let constants = [("MODEL", family as f64)];
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            let label = format!("Solar model {name}");
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&label),
                layout: Some(&model_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_model"),
                    buffers: &[Some(mesh_layout), Some(instance_layout)],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_model"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: options,
                }),
                // Closed and wound counter-clockwise outward, which on the
                // frame (x right, y up, depth away: a mirror) is clockwise:
                // the far side need never be rasterised.
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
            pipelines.insert(Pipe::Model(family), (label, made));
        }
        // A battle's effects after its meshes, each kind on its own bounded
        // geometry (`battle_effects`), tested against them at its own depth
        // without writing it, added; their dim light apart, behind them.
        let fx_attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4
        ];
        for ((kind, (part, vs, fs, _)), far) in FX_KINDS
            .iter()
            .enumerate()
            .flat_map(|k| [(k, false), (k, true)])
        {
            let key = Pipe::BattleFx { kind, far };
            let label = if far {
                format!("Solar battle effects {part} dim")
            } else {
                format!("Solar battle effects {part}")
            };
            let constants = [("FX_FAR", if far { 1.0 } else { 0.0 })];
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            let layout = wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Fx>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &fx_attributes,
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[Some(layout)],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: add,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
            pipelines.insert(key, (label, made));
        }
        for (name, label, vertex, fragment) in [
            (
                Pipe::OverlayFlash,
                "Solar ship overlay flash",
                "vs_overlay",
                "fs_overlay_flash",
            ),
            (
                Pipe::OverlayHit,
                "Solar ship overlay hit",
                "vs_overlay_hit",
                "fs_overlay_hit",
            ),
            (
                Pipe::OverlayRim,
                "Solar ship overlay rim",
                "vs_overlay_rim",
                "fs_overlay_beam",
            ),
            (
                Pipe::OverlayBeam,
                "Solar ship overlay beam",
                "vs_overlay_beam",
                "fs_overlay_beam",
            ),
        ] {
            pipelines.insert(
                name,
                make(
                    label,
                    FrameDraw {
                        blend: over,
                        ..FrameDraw::new(vertex, fragment)
                    },
                ),
            );
        }
        for (name, entry, blend) in [
            (Pipe::Eclipse, "fs_eclipse", multiply),
            (Pipe::RingShadow, "fs_ring_shadow", multiply),
        ] {
            pipelines.insert(
                name,
                pipeline(&format!("Solar {name}"), "vs_body", entry, blend, &[]),
            );
        }
        // The star, worlds whose maps are not in and moons as meshes too
        // (`vs_globe`, `GLOBE`), named `globe…` so they are drawn as one.
        let all = wgpu::ColorWrites::ALL;
        const STAR_GLOBE: [(&str, f64); 2] = [("GLOBE", 3.0), ("AIR", 0.0)];
        pipelines.insert(
            Pipe::GlobeStar,
            globe("Solar globestar", "fs_globe_star", over, &STAR_GLOBE, all),
        );
        for kind in Kind::ALL {
            let ringed = matches!(kind, Kind::GasGiant | Kind::IceGiant);
            for rings in [false, true].into_iter().filter(|&r| ringed || !r) {
                let look = FlatLook::of(kind, rings);
                let name = Pipe::GlobeFlat(look);
                if pipelines.contains_key(&name) {
                    continue;
                }
                let label = format!("Solar {name}");
                let made = globe(&label, "fs_globe_flat", over, &look.constants(), all);
                pipelines.insert(name, made);
            }
        }
        for slot in 0..2 {
            for air in [false, true] {
                let name = Pipe::GlobeMoon { slot, air };
                let air = f64::from(u8::from(air));
                let constants = [("GLOBE", 2.0), ("SLOT", slot as f64), ("AIR", air)];
                let label = format!("Solar {name}");
                let made = globe(&label, "fs_globe_moon", over, &constants, all);
                pipelines.insert(name, made);
            }
        }
        // The rings, drawn as an annulus (`vs_ring`).
        pipelines.insert(
            Pipe::Ring,
            pipeline("Solar ring", "vs_ring", "fs_ring_annulus", over, &[]),
        );
        // The limb, off its own disc only (a strict depth test): an annulus
        // lit at its vertices for a body wholly ahead, else traced.
        pipelines.insert(
            Pipe::Limb,
            make(
                "Solar limb",
                FrameDraw {
                    blend: add,
                    strict: true,
                    ..FrameDraw::new("vs_body", "fs_limb")
                },
            ),
        );
        pipelines.insert(
            Pipe::LimbRing,
            make(
                "Solar limb ring",
                FrameDraw {
                    blend: add,
                    strict: true,
                    ..FrameDraw::new("vs_limb_ring", "fs_limb_ring")
                },
            ),
        );
        pipelines.insert(
            Pipe::ProbeLimbRing,
            make(
                "Solar probe limb ring",
                FrameDraw {
                    writes: none,
                    strict: true,
                    ..FrameDraw::new("vs_limb_ring", "fs_probe")
                },
            ),
        );
        pipelines.insert(
            Pipe::ProbeGlobe,
            globe("Solar probe globe", "fs_probe_globe", None, &[], none),
        );
        let mut seen = std::collections::HashSet::new();
        for slot in 0..2 {
            // A mapped world from afar as a mesh (`fs_globe`), one pipeline
            // for each set of features a kind has: only its own code, in as
            // few registers as it needs.
            let looks = Kind::ALL.into_iter().flat_map(|k| {
                let rings = matches!(k, Kind::GasGiant | Kind::IceGiant);
                [false, true]
                    .into_iter()
                    .filter(move |&r| rings || !r)
                    .map(move |r| SurfaceLook::of(k, r))
            });
            for look in looks {
                let mesh = Pipe::Globe(look, slot);
                if !seen.insert(mesh) {
                    continue;
                }
                let mut constants = vec![("SLOT", slot as f64)];
                constants.extend(look.constants());
                let label = format!("Solar {mesh}");
                let all = wgpu::ColorWrites::ALL;
                pipelines.insert(mesh, globe(&label, "fs_globe", over, &constants, all));
                if look.sea {
                    // A sea's added light, a pass of its own.
                    let glow = Pipe::GlobeGlow(look, slot);
                    let label = format!("Solar {glow}");
                    let glow_pipeline = globe(&label, "fs_globe_glow", add, &constants, all);
                    pipelines.insert(glow, glow_pipeline);
                }
            }
            // The ground close up, its magnified map lookups made crisp.
            pipelines.insert(
                Pipe::GroundDetail(slot),
                pipeline(
                    &format!("Solar ground detail {slot}"),
                    "vs_body",
                    "fs_ground",
                    over,
                    &[("SLOT", slot as f64), ("DETAIL", 1.0)],
                ),
            );
            // The added light with the sea's glint, for worlds with seas.
            pipelines.insert(
                Pipe::GlowSea(slot),
                pipeline(
                    &format!("Solar glow sea {slot}"),
                    "vs_body",
                    "fs_glow",
                    add,
                    &[("SLOT", slot as f64), ("SEA", 1.0)],
                ),
            );
            for (name, key, entry, blend) in [
                ("ground", Pipe::Ground(slot), "fs_ground", over),
                ("detail", Pipe::Detail(slot), "fs_detail", double_multiply),
                ("clouds", Pipe::Clouds(slot), "fs_clouds", over),
                ("glow", Pipe::Glow(slot), "fs_glow", add),
            ] {
                pipelines.insert(
                    key,
                    pipeline(
                        &format!("Solar {name} {slot}"),
                        "vs_body",
                        entry,
                        blend,
                        &[("SLOT", slot as f64)],
                    ),
                );
            }
        }
        // The stacked cloud layers drawn in low orbit, per slot.
        for slot in 0..2 {
            for layer in 1..=CLOUD_LAYERS {
                pipelines.insert(
                    Pipe::CloudLayer(slot, layer),
                    pipeline(
                        &format!("Solar cloud layer {slot} {layer}"),
                        "vs_body",
                        "fs_cloud_layer",
                        over,
                        &[("SLOT", slot as f64), ("LAYER", layer as f64)],
                    ),
                );
            }
        }
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar uniforms"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let effects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar battle effects"),
            size: (FX_INSTANCES * std::mem::size_of::<Fx>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let panels = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar Dyson panels"),
            size: (DYSON_PANELS as usize * std::mem::size_of::<Panel>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid = {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Solar mesh grids"),
                contents: bytemuck::cast_slice(&grid_indices()),
                usage: wgpu::BufferUsages::INDEX,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar maps"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // No anisotropy unless `SOLAR_ANISO` asks for it: the traced
            // shaders' coordinates jump a whole turn where the longitude
            // wraps, and the V3D sized its footprint from that (a dotted seam
            // down every planet; about 2 ms in a close orbit). The meshes
            // never wrap inside a triangle, so they can use it.
            anisotropy_clamp: map_anisotropy(),
            ..Default::default()
        });
        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar frame"),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                });
                // Drawn through an sRGB view: the passes blend in linear light
                // and the frame Slint reads is encoded.
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    ..Default::default()
                });
                Frame { texture, view }
            })
            .collect();
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Solar depth mask"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let tour = tour_for(seed, 0, false, None);
        let halos = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Solar leviathan halos"),
                size: wgpu::Extent3d {
                    width: SIZE / LEVIATHAN_HALO,
                    height: SIZE / LEVIATHAN_HALO,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: LEVIATHAN_HALO_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let halo_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar leviathan halos"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let halo_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar leviathan halos"),
            layout: &halo_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&halos),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&halo_sampler),
                },
            ],
        });
        let coarse = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Solar coarse"),
                size: wgpu::Extent3d {
                    width: SIZE / COARSE,
                    height: SIZE / COARSE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: LEVIATHAN_HALO_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let coarse_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar coarse"),
            layout: &halo_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&coarse),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&halo_sampler),
                },
            ],
        });
        let rings = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Solar ring profiles"),
            size: wgpu::Extent3d {
                width: RING_TEXELS,
                height: RING_ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            pipelines,
            drive_override: None,
            layout,
            uniforms,
            effects,
            panels,
            grid,
            sampler,
            bind_group: None,
            leviathan_halos: (halos, halo_group),
            coarse: (coarse, coarse_group),
            model_library: Some(models::spawn_library()),
            models: None,
            scratch: ModelScratch::default(),
            rings,
            rings_for: u64::MAX,
            craters,
            crater_layout,
            gas,
            bound: (u64::MAX, usize::MAX, usize::MAX),
            starfields: [0, 1].map(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar starfield"),
                    size: wgpu::Extent3d {
                        width: STARFIELD_FACE,
                        height: STARFIELD_FACE,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                })
            }),
            sky_cubes: [0, 1].map(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar sky cube"),
                    size: wgpu::Extent3d {
                        width: SKY_CUBE_FACE,
                        height: SKY_CUBE_FACE,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                })
            }),
            starfield_owner: [None, None],
            frames,
            depth,
            streamer,
            base_seed: seed,
            visit: 0,
            visit_start: 0.0,
            tour,
            pending: Vec::new(),
            console: console::Console::default(),
            stars: (u64::MAX, Vec::new()),
            incidents: (u64::MAX, 0, Vec::new()),
            raised: None,
            diverted: None,
            hud: Hud::default(),
            hud_at: f64::NEG_INFINITY,
            labels: Vec::new(),
            dust: [0.0; 3],
            dust_at: None,
            view_override: None,
            showcase: None,
            #[cfg(test)]
            drawn: Default::default(),
        }
    }

    pub(super) fn bind(&mut self, gpu: &Graphics) {
        if self.rings_for != self.tour.system.seed {
            paint_rings(gpu, &self.rings, &self.tour.system);
            self.rings_for = self.tour.system.seed;
        }
        let sky = self.streamer.sky_ready(self.tour.system.seed).unwrap_or(0);
        let stars = self.starfield_slot(self.tour.system.seed).unwrap_or(0);
        let wanted = (self.streamer.version, sky, stars);
        if self.bind_group.is_some() && self.bound == wanted {
            return;
        }
        self.bind_group = Some(self.group(gpu, sky, stars));
        self.bound = wanted;
    }

    /// The frame's bind group, with galaxy sky `sky` and baked starfield
    /// `stars`.
    pub(super) fn group(&self, gpu: &Graphics, sky: usize, stars: usize) -> wgpu::BindGroup {
        let full = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let a = self.streamer.slot_maps(0);
        let b = self.streamer.slot_maps(1);
        let views = [
            full(self.streamer.sky(sky)),
            full(&a.day),
            full(&a.relief),
            full(&a.extra),
            array(&a.moon_day),
            array(&a.moon_relief),
            full(&b.day),
            full(&b.relief),
            full(&b.extra),
            array(&b.moon_day),
            array(&b.moon_relief),
        ];
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for (i, view) in views.iter().enumerate().skip(1) {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let rings = full(&self.rings);
        entries.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::TextureView(&rings),
        });
        // Read back through the sRGB view it was written through.
        let starfield = self.starfields[stars].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 14,
            resource: wgpu::BindingResource::TextureView(&starfield),
        });
        let sky_cube = self.sky_cubes[stars].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 15,
            resource: wgpu::BindingResource::TextureView(&sky_cube),
        });
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar frame"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// The starfield slot finished for the system of `seed`, if any.
    pub(super) fn starfield_slot(&self, seed: u64) -> Option<usize> {
        (0..2).find(|&k| self.starfield_owner[k] == Some((seed, STARFIELD_HEIGHT)))
    }

    /// Bake a band of the starfield of the system that needs one: the
    /// current one, or the next once its galaxy is in (during the jump).
    /// Into a slot neither system has finished, reading the galaxy map the
    /// streamer painted and drawing the stars a texel wide, so the frame's
    /// background is one texture read instead of three layers of hashed
    /// stars a pixel.
    pub(super) fn bake_stars(&mut self, gpu: &Graphics, encoder: &mut wgpu::CommandEncoder) {
        let seeds = [self.tour.system.seed, self.tour.next.seed];
        let Some((seed, sky)) = seeds.iter().find_map(|&seed| {
            let done = self.starfield_slot(seed).is_some();
            let sky = self.streamer.sky_ready(seed)?;
            (!done).then_some((seed, sky))
        }) else {
            return;
        };
        let slot = match (0..2).find(|&k| self.starfield_owner[k].map(|o| o.0) == Some(seed)) {
            Some(k) => k,
            None => {
                // Not one either system still needs.
                let free = (0..2).find(|&k| match self.starfield_owner[k] {
                    None => true,
                    Some((s, _)) => !seeds.contains(&s),
                });
                let Some(k) = free else {
                    return;
                };
                self.starfield_owner[k] = Some((seed, 0));
                k
            }
        };
        let row = self.starfield_owner[slot].map_or(0, |o| o.1);
        let (face, top) = (row / STARFIELD_FACE, row % STARFIELD_FACE);
        let rows = STARFIELD_ROWS.min(STARFIELD_FACE - top);
        // The other slot's textures at bindings 14 and 15: this one's are
        // the targets.
        let group = self.group(gpu, sky, 1 - slot);
        if row == 0 {
            // The galaxy alone, small: all of it at once.
            for face in 0..6 {
                let view = self.sky_cubes[slot].create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Solar sky cube bake"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let (label, pipeline) = self.pipe(Pipe::BakeSky);
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_bind_group(1, &self.streamer.noise().group, &[]);
                let texels = (SKY_CUBE_FACE * SKY_CUBE_FACE) as u64;
                crate::gpu::counted_draw(&mut pass, label, texels, 0..3, face..face + 1);
            }
        }
        let view = self.starfields[slot].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: face,
            array_layer_count: Some(1),
            ..Default::default()
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar starfield bake"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if top == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let (label, pipeline) = self.pipe(Pipe::Bake);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            pass.set_scissor_rect(0, top, STARFIELD_FACE, rows);
            crate::gpu::counted_draw(
                &mut pass,
                label,
                (STARFIELD_FACE * rows) as u64,
                0..3,
                face..face + 1,
            );
        }
        self.starfield_owner[slot] = Some((seed, row + rows));
    }

    /// The starship meshes onto the GPU, once their thread has them.
    pub(super) fn receive_models(&mut self, gpu: &Graphics) {
        let Some(rx) = &self.model_library else {
            return;
        };
        let Ok(mut library) = rx.try_recv() else {
            return;
        };
        let library_maps = library.rock_maps.take();
        use wgpu::util::DeviceExt;
        let vertices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Solar ship vertices"),
                contents: bytemuck::cast_slice(&library.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let indices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Solar ship indices"),
                contents: bytemuck::cast_slice(&library.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar model instances"),
            size: (MODEL_INSTANCES * std::mem::size_of::<models::Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let library = models::Library {
            vertices: Vec::new(),
            indices: Vec::new(),
            ranges: library.ranges,
            clusters: library.clusters,
            rock_maps: None,
        };
        if let Some(maps) = &library_maps {
            self.craters = rock_maps(gpu, &self.crater_layout, Some(maps));
        }
        self.models = Some(Models {
            vertices,
            indices,
            instances,
            library,
        });
        self.model_library = None;
    }
}
