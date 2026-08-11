fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = tauri_winres::WindowsResource::new();

        res.set_manifest_file("ito.manifest");
        res.set("FileDescription", "Ito - Groq voice dictation");
        res.set("ProductName", "Ito");
        res.set("CompanyName", "Demox Labs");
        res.set(
            "LegalCopyright",
            "Copyright © 2025 Demox Labs. All rights reserved.",
        );
        res.set("FileVersion", "0.1.0.0");
        res.set("ProductVersion", "0.1.0.0");
        res.set("InternalName", "ito");
        res.set("OriginalFilename", "ito.exe");
        res.compile().unwrap();
    }
}
