fn main() {
    #[cfg(target_os = "windows")]
    stage_pdfium_runtime();

    // Embed Windows icon resource
    #[cfg(target_os = "windows")]
    {
        use std::env;
        use std::path::PathBuf;

        // Get the manifest directory (project root)
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let icon_path = PathBuf::from(&manifest_dir).join("appicon.ico");

        // Check if ICO file exists
        if icon_path.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon_path.to_str().unwrap());
            res.set("FileDescription", "MTT File Manager");
            res.set("ProductName", "MTT File Manager");
            res.set("CompanyName", "MTT");

            // Embed application manifest for Per-Monitor V2 DPI awareness
            // and Windows 10/11 compatibility.  Without this, the DWM applies
            // bitmap-scaling which adds overhead and visual blur.
            let manifest_path = PathBuf::from(&manifest_dir).join("app.manifest");
            if manifest_path.exists() {
                res.set_manifest_file(manifest_path.to_str().unwrap());
                println!("cargo:rerun-if-changed={}", manifest_path.display());
            }

            // Compile the resource
            if let Err(e) = res.compile() {
                eprintln!("Warning: Could not embed icon: {}", e);
            } else {
                println!("cargo:rerun-if-changed={}", icon_path.display());
            }
        } else {
            eprintln!("Warning: appicon.ico not found at {}", icon_path.display());
            eprintln!("Using default Windows icon.");
        }
    }
}

#[cfg(target_os = "windows")]
fn stage_pdfium_runtime() {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    println!("cargo:rerun-if-env-changed=PDFIUM_DYNAMIC_LIB_PATH");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_dir = PathBuf::from(env::var("OUT_DIR").unwrap())
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf);

    let Some(target_dir) = target_dir else {
        eprintln!("Warning: could not resolve target directory for pdfium.dll staging");
        return;
    };

    let dll_name = "pdfium.dll";
    let source = env::var_os("PDFIUM_DYNAMIC_LIB_PATH")
        .map(PathBuf::from)
        .map(|dir| dir.join(dll_name))
        .filter(|path| path.exists())
        .or_else(|| {
            [
                manifest_dir.join("vendor").join(dll_name),
                manifest_dir.join("vendor").join("pdfium").join(dll_name),
            ]
            .into_iter()
            .find(|path| path.exists())
        });

    let Some(source) = source else {
        println!("cargo:warning=pdfium.dll not found in vendor/ or PDFIUM_DYNAMIC_LIB_PATH; standalone PDF viewer will require the runtime to be staged manually.");
        return;
    };

    let destination = target_dir.join(dll_name);

    if let Err(err) = fs::copy(&source, &destination) {
        eprintln!(
            "Warning: failed to stage pdfium.dll from {} to {}: {}",
            source.display(),
            destination.display(),
            err
        );
    } else {
        println!("cargo:rerun-if-changed={}", source.display());
    }
}
