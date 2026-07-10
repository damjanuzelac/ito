fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = tauri_winres::WindowsResource::new();

        res.set_manifest_file("ito-tray.manifest");
        res.set(
            "FileDescription",
            "Ito Tray - Standalone local voice dictation",
        );
        res.set("ProductName", "Ito Tray");
        res.set("CompanyName", "Demox Labs");
        res.set(
            "LegalCopyright",
            "Copyright © 2025 Demox Labs. All rights reserved.",
        );
        res.set("FileVersion", "0.1.0.0");
        res.set("ProductVersion", "0.1.0.0");
        res.set("InternalName", "ito-tray");
        res.set("OriginalFilename", "ito-tray.exe");
        res.compile().unwrap();
    }
}
