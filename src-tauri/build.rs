// Tauri generates platform-specific application metadata during the build.
// Keeping this in a tiny build script lets the desktop crate remain separate
// from the reusable Rust library and from the CLI binary.
fn main() {
    // Configure the Windows runtime through the current Tauri build API. This
    // replaces the deprecated STATIC_VCRUNTIME environment variable and keeps
    // the dynamic runtime default on Windows without affecting other targets.
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new().static_vc_runtime(false));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
