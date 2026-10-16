fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/opensc.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/opensc.ico")
            .set("ProductName", "OpenSC Launcher")
            .set("FileDescription", "Open Shadow Company Launcher")
            .compile()
            .expect("compile Windows icon and version resource");
    }
}
