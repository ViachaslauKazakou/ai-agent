// Tauri generates platform-specific application metadata during the build.
// Keeping this in a tiny build script lets the desktop crate remain separate
// from the reusable Rust library and from the CLI binary.
fn main() {
    tauri_build::build()
}
