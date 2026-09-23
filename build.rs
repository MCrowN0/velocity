fn main() {
    println!("cargo:rerun-if-changed=src/sprite.wgsl");
    let source = std::fs::read_to_string("src/sprite.wgsl").unwrap();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::PUSH_CONSTANT,
    )
    .validate(&module)
    .unwrap();
    for (name, stage) in [
        ("vs_main", naga::ShaderStage::Vertex),
        ("fs_main", naga::ShaderStage::Fragment),
    ] {
        let mut options = naga::back::spv::Options::default();
        options
            .flags
            .remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
        let words = naga::back::spv::write_vec(
            &module,
            &info,
            &options,
            Some(&naga::back::spv::PipelineOptions {
                shader_stage: stage,
                entry_point: name.into(),
            }),
        )
        .unwrap();
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        std::fs::write(
            std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join(format!("{name}.spv")),
            bytes,
        )
        .unwrap();
    }
}
