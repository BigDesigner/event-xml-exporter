fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/app.ico")
            .set("ProductName", "Event XML Exporter")
            .set(
                "FileDescription",
                "Windows Event Log kayitlarini XML olarak disa aktarir",
            )
            .set("CompanyName", "GNNcyber")
            .set("OriginalFilename", "event_xml_exporter_rust.exe")
            .set("LegalCopyright", "Copyright (c) 2026 GNNcyber")
            .set_version_info(winresource::VersionInfo::FILEVERSION, 0x0001000000000000)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, 0x0001000000000000);
        resource
            .compile()
            .expect("Windows icon resource could not be compiled");
    }
}
