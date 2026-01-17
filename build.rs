fn main() {
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
