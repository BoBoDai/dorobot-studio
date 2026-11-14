fn main() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_VIDEO");
    if std::env::var_os("CARGO_FEATURE_VIDEO").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
    {
        // Conda's FFmpeg libraries use @rpath, whereas Homebrew uses absolute
        // install names. Embedding the selected library directory supports both.
        let tool = std::env::var_os("PKG_CONFIG").unwrap_or_else(|| "pkg-config".into());
        if let Ok(out) = std::process::Command::new(tool)
            .args(["--variable=libdir", "libavutil"])
            .output()
        {
            if out.status.success() {
                let dir = String::from_utf8_lossy(&out.stdout);
                let dir = dir.trim();
                if !dir.is_empty() {
                    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
                }
            }
        }
    }
}
