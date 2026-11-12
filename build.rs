fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logo.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Exe-Icon nicht eingebettet: {e}");
        }
    }
}
