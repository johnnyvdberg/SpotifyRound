//! A bench for trying shader variants against the Pi 5's compiler.
//!
//! `SHADER_LAB_DIR=<dir> cargo test --release shader_lab -- --ignored` builds a
//! render pipeline for every `*.wgsl` file in the directory, read at run
//! time, so variants can be compiled one after another under the V3D
//! shader-db (`deploy/v3d-shaderdb`) without rebuilding the app. Each file
//! provides `vs_main` and one or more fragment entry points; a first-line
//! comment `// targets: rgba8unorm-srgb rgba8unorm rg8unorm` lists its colour
//! targets (one `rgba8unorm` if absent), and `// entries: fs_a fs_b` its
//! fragment entry points (`fs_main` if absent), `// vertex: vs_x` its vertex
//! entry point (`vs_main` if absent), and `// variants: A=1 B=0 | A=0 B=1` the
//! values of its `override` constants, one pipeline each. Pipelines are
//! labelled `lab <file> <entry> <variant>`, with bind group layouts derived
//! from the shader.

#[cfg(test)]
mod tests {
    use slint::wgpu_30::wgpu;

    fn format(name: &str) -> wgpu::TextureFormat {
        match name {
            "rgba8unorm-srgb" => wgpu::TextureFormat::Rgba8UnormSrgb,
            "rg8unorm" => wgpu::TextureFormat::Rg8Unorm,
            "r8unorm" => wgpu::TextureFormat::R8Unorm,
            "rgba16float" => wgpu::TextureFormat::Rgba16Float,
            "rgba32float" => wgpu::TextureFormat::Rgba32Float,
            _ => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// Write the scenes' composed shader sources to `SHADER_DUMP_DIR`, as
    /// starting points for variants.
    #[test]
    #[ignore = "writes shader sources; set SHADER_DUMP_DIR"]
    fn dump() {
        let Ok(dir) = std::env::var("SHADER_DUMP_DIR") else {
            return;
        };
        let solar = |file: &str| format!("{}{}", include_str!("solar/noise.wgsl"), file);
        for (name, source) in [
            ("planet-frame", crate::planet::shader_source()),
            ("planet-generator", crate::planet::generator_source()),
            ("solar-generate", solar(include_str!("solar/generate.wgsl"))),
            ("solar-frame", solar(include_str!("solar/frame.wgsl"))),
        ] {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{name}.wgsl")),
                source,
            )
            .expect("a dump");
        }
    }

    #[test]
    #[ignore = "compiles shader variants from SHADER_LAB_DIR"]
    fn compile() {
        let Ok(dir) = std::env::var("SHADER_LAB_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .expect("the lab directory")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "wgsl"))
            .collect();
        files.sort();
        for path in files {
            let source = std::fs::read_to_string(&path).expect("a shader");
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let header = |key: &str| {
                source.lines().take(6).find_map(|l| {
                    l.trim()
                        .strip_prefix(&format!("// {key}:"))
                        .map(str::to_owned)
                })
            };
            let targets: Vec<_> = header("targets")
                .unwrap_or_else(|| "rgba8unorm".into())
                .split_whitespace()
                .map(|t| {
                    Some(wgpu::ColorTargetState {
                        format: format(t),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                })
                .collect();
            let entries = header("entries").unwrap_or_else(|| "fs_main".into());
            let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&name),
                    source: wgpu::ShaderSource::Wgsl(source.clone().into()),
                });
            let vertex = header("vertex").unwrap_or_else(|| "vs_main".into());
            let variants: Vec<Vec<(String, f64)>> = match header("variants") {
                Some(list) => list
                    .split('|')
                    .map(|v| {
                        v.split_whitespace()
                            .filter_map(|kv| {
                                let (k, x) = kv.split_once('=')?;
                                Some((k.to_string(), x.parse().ok()?))
                            })
                            .collect()
                    })
                    .collect(),
                None => vec![Vec::new()],
            };
            for entry in entries.split_whitespace() {
                for variant in &variants {
                    let tag = variant
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let label = format!("lab {name} {entry} {tag}");
                    let constants: Vec<(&str, f64)> =
                        variant.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                    let options = wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    };
                    let _pipeline =
                        gpu.device
                            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                label: Some(&label),
                                layout: None,
                                vertex: wgpu::VertexState {
                                    module: &module,
                                    entry_point: Some(&vertex),
                                    buffers: &[],
                                    compilation_options: options.clone(),
                                },
                                fragment: Some(wgpu::FragmentState {
                                    module: &module,
                                    entry_point: Some(entry),
                                    targets: &targets,
                                    compilation_options: options,
                                }),
                                primitive: wgpu::PrimitiveState::default(),
                                depth_stencil: None,
                                multisample: wgpu::MultisampleState::default(),
                                multiview_mask: None,
                                cache: None,
                            });
                }
            }
            if let Some(err) = pollster::block_on(scope.pop()) {
                println!("lab {name}: {err}");
            }
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    }
}
