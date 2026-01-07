fn main() {
    // Embed Windows icon resource
    #[cfg(target_os = "windows")]
    {
        use std::path::Path;
        
        // Check if ICO file exists, otherwise skip (won't break build)
        if Path::new("appicon.ico").exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("appicon.ico");
            res.set("FileDescription", "MTT File Manager");
            res.set("ProductName", "MTT File Manager");
            res.set("CompanyName", "MTT");
            
            // Only fail if ICO exists but compilation fails
            if let Err(e) = res.compile() {
                eprintln!("Warning: Could not embed icon: {}", e);
                eprintln!("You can convert appicon.png to appicon.ico using:");
                eprintln!("  - https://convertio.co/png-ico/");
                eprintln!("  - ImageMagick: magick convert appicon.png -define icon:auto-resize appicon.ico");
            }
        } else {
            eprintln!("Note: appicon.ico not found - using default icon");
            eprintln!("Convert appicon.png to appicon.ico format and rebuild to use custom icon");
        }
    }
}
