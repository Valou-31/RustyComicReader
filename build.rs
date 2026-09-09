#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("src/logo.ico");
    res.compile().expect("failed to embed Windows icon resource");
}

#[cfg(not(windows))]
fn main() {}
