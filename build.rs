fn main() {
    println!("cargo:rerun-if-changed=ui");
    slint_build::compile("ui/main.slint").expect("failed to compile the Slint UI");

    // WESL links the shared shader module into each root shader at build time.
    // The output is plain WGSL in OUT_DIR; wgpu never sees the WESL syntax.
    let mut features = wesl::Features::default();
    features.set(
        "full_hd",
        std::env::var_os("CARGO_FEATURE_FULL_HD").is_some(),
    );
    let options = wesl::CompileOptions {
        validate: false,
        features,
        ..Default::default()
    };
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FULL_HD");
    let compiler = wesl::Compiler::new(options)
        .with_resolver(wesl::resolver::FileResolver::new("src/shaders"));
    let main = wesl::syntax::ModulePath::new(
        wesl::syntax::PathOrigin::Absolute,
        vec!["scene".to_string()],
    );
    let result = compiler
        .compile_module(&main)
        .inspect_err(|e| eprintln!("{e}"))
        .expect("failed to compile scene");
    result.emit_rerun_if_changed();
    result.write_artifact("scene");
    println!("cargo:rerun-if-changed=src/shaders");
}
