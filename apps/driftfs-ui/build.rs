fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI compilation failed");

    #[cfg(target_os = "windows")]
    {
        let _ = embed_resource::compile("driftfs-ui.rc", embed_resource::NONE);
    }
}
