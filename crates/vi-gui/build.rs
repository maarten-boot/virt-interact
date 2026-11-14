//! Windows: embed the app icon and version information in the .exe, so
//! Explorer, the taskbar and shortcuts show it (crosscheck 2026-11-14-03,
//! improvement 1). Other platforms: nothing (macOS gets the icon from the
//! app bundle, Linux from the .desktop file).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/icon/virt-interact.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resource();
    }
}

/// The resource compiler is only available on a Windows host.
#[cfg(windows)]
fn windows_resource() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/icon/virt-interact.ico")
        .set("ProductName", "virt-interact")
        .set(
            "FileDescription",
            "virt-interact: read-only VM inventory over SSH",
        );
    if let Err(e) = res.compile() {
        // A missing resource compiler must not break the build; the app
        // then still shows its window icon.
        println!("cargo:warning=no Windows icon resource: {e}");
    }
}

#[cfg(not(windows))]
fn windows_resource() {
    println!("cargo:warning=cross-compiling for Windows: no icon resource");
}
