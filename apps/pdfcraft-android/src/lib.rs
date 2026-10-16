//! PdfCraft for Android.
//!
//! The same egui app as the desktop and web builds (`pdfcraft-ui-egui`), started from a
//! GameActivity (`android/app/.../MainActivity.kt`). The Kotlin side does what egui can't:
//! the system file pickers (Storage Access Framework), "Open with" and share intents. Files
//! cross the bridge as bytes or as paths in the app's cache (see `jni_bridge`).

#![cfg(target_os = "android")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod jni_bridge;

use pdfcraft_ui_egui::PdfCraftApp;
use winit::platform::android::activity::AndroidApp;

/// The app was called PrintCraft before; settings saved then are under this key.
const LEGACY_STORAGE_KEY: &str = "printcraft";

/// Entry point: android-activity calls this on its own thread once the Activity exists.
#[unsafe(no_mangle)]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute in edition 2024; the symbol is android-activity's contract.
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("PdfCraft"));
    log::info!("PdfCraft {} starting", env!("CARGO_PKG_VERSION"));

    jni_bridge::init(&app);
    let persistence_path = app.internal_data_path().map(|d| d.join("app.ron"));
    let options = eframe::NativeOptions { android_app: Some(app), persistence_path, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let result = eframe::run_native(
        "PdfCraft",
        options,
        Box::new(|cc| {
            let mut app = PdfCraftApp::new();
            if let Some(json) = cc.storage.and_then(|s| s.get_string("pdfcraft").or_else(|| s.get_string(LEGACY_STORAGE_KEY))) {
                app.restore(&json);
            }
            jni_bridge::attach(&mut app, &cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        log::error!("PdfCraft stopped: {e}");
    }
}
