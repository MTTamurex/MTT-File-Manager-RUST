fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set("FileDescription", "MTT File Manager Search Service");
        resource.set("ProductName", "MTT File Manager");
        resource.set("CompanyName", "MTT");
        resource
            .compile()
            .expect("Failed to compile search service version resources");
    }
}
