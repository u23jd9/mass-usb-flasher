fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_manifest_file("app.manifest");
        res.set_icon("icon.ico"); // если есть иконка
        res.compile().unwrap();
    }
}