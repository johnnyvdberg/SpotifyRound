//! Every shader the app builds, parsed and validated with naga (the same
//! front end wgpu uses) without a GPU, so a mistake in a shader fails
//! `cargo test` rather than the scene that first creates its module.
//!
//! The uniform blocks the CPU fills are checked here too: each module's tests
//! compare the size naga lays a WGSL struct out at with the Rust struct
//! written into it ([`struct_size`]).

use slint::wgpu_30::wgpu::naga;

/// `source` parsed, validated, and validated again with its `override`s at
/// their defaults, as a pipeline without constants would see it. Panics
/// with naga's report, which names the line, on any error.
pub fn validate(name: &str, source: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    let info = validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
    let (resolved, _) = naga::back::pipeline_constants::process_overrides(
        &module,
        &info,
        None,
        &naga::back::PipelineConstants::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: overrides at their defaults: {e}"));
    validator
        .validate(&resolved)
        .unwrap_or_else(|e| panic!("{name}: with overrides resolved: {e}"));
    module
}

/// The size naga gives WGSL struct `name` in `source` (its span), as a
/// uniform buffer is laid out.
pub fn struct_size(source: &str, name: &str) -> usize {
    let module = validate(name, source);
    module
        .types
        .iter()
        .find_map(|(_, ty)| match &ty.inner {
            naga::TypeInner::Struct { span, .. } if ty.name.as_deref() == Some(name) => {
                Some(*span as usize)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no struct `{name}` in the shader"))
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn the_solar_frame_shader_is_valid() {
        validate("solar frame", crate::solar::FRAME_SHADER);
    }

    #[test]
    fn the_solar_generator_shader_is_valid() {
        validate("solar generator", crate::solar::stream::GENERATE_SHADER);
    }

    #[test]
    fn the_solar_mip_shader_is_valid() {
        validate("solar mips", crate::solar::stream::MIP_SHADER);
    }

    #[test]
    fn the_nautilus_shader_is_valid() {
        validate("dive3d scene", crate::dive3d::SCENE_SHADER);
    }

    #[test]
    fn the_globe_shader_is_valid() {
        validate("globe", &crate::globe::shader_source());
    }

    #[test]
    fn the_observatory_shader_is_valid() {
        validate("observatory", crate::observatory::SHADER);
    }

    #[test]
    fn the_microscope_shader_is_valid() {
        validate("microscope", crate::microscope::SHADER);
    }

    #[test]
    fn the_progress_ring_shader_is_valid() {
        validate("ring", crate::ring::SHADER);
    }

    #[test]
    fn the_gpu_cost_shader_is_valid() {
        validate("gpu cost", crate::gpu_cost::SHADER);
    }

    /// Each uniform block the CPU writes, against the WGSL struct it fills:
    /// a field added on one side only shifts everything after it.
    #[test]
    fn uniform_blocks_match_their_rust_structs() {
        use super::struct_size;
        use std::mem::size_of;
        let solar = crate::solar::FRAME_SHADER;
        let checks = [
            (
                "solar Params",
                struct_size(solar, "Params"),
                size_of::<crate::solar::Params>(),
            ),
            (
                "solar Body",
                struct_size(solar, "Body"),
                size_of::<crate::solar::Body>(),
            ),
            (
                "solar Gen",
                struct_size(crate::solar::stream::GENERATE_SHADER, "Gen"),
                size_of::<crate::solar::stream::GenParams>(),
            ),
            (
                "dive3d Globals",
                struct_size(crate::dive3d::SCENE_SHADER, "Globals"),
                size_of::<crate::dive3d::Globals>(),
            ),
            (
                "dive3d Instance",
                struct_size(crate::dive3d::SCENE_SHADER, "Instance"),
                size_of::<crate::dive3d::world::Instance>(),
            ),
            (
                "globe Uniforms",
                struct_size(&crate::globe::shader_source(), "Uniforms"),
                size_of::<crate::globe::Uniforms>(),
            ),
            (
                "observatory Params",
                struct_size(crate::observatory::SHADER, "Params"),
                size_of::<crate::observatory::Params>(),
            ),
            (
                "microscope Params",
                struct_size(crate::microscope::SHADER, "Params"),
                size_of::<crate::microscope::Params>(),
            ),
            (
                "ring Uniforms",
                struct_size(crate::ring::SHADER, "Uniforms"),
                size_of::<crate::ring::Uniforms>(),
            ),
        ];
        for (name, shader, rust) in checks {
            assert_eq!(shader, rust, "{name}: WGSL {shader} bytes, Rust {rust}");
        }
    }
}
