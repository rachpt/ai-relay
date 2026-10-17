use std::{env, error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/ai-relay.ico");
    println!("cargo:rerun-if-changed=assets/ai-relay.manifest");

    // Check the target, not the build host, so cross builds select resources correctly.
    if env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "CARGO_MANIFEST_DIR is not set",
    ))?);
    let icon = root.join("assets/ai-relay.ico");
    let manifest = root.join("assets/ai-relay.manifest");
    let icon = icon.to_str().ok_or("icon path is not valid Unicode")?;
    let manifest = manifest
        .to_str()
        .ok_or("manifest path is not valid Unicode")?;

    winresource::WindowsResource::new()
        .set_icon_with_id(icon, "1")
        .set_manifest_file(manifest)
        .compile()?;
    Ok(())
}
