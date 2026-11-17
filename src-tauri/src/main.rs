// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Linux: WebKitGTK's DMA-BUF renderer (2.42+) freezes the window on many GPU and
    // driver setups (NVIDIA, some Wayland compositors): the page stops repainting and no
    // click goes through. Turn it off unless the user set it themselves. This has to run
    // before WebKitGTK starts, so before the app is built.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    bennovel_lib::run()
}
