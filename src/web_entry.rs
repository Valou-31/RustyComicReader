//! Browser entry point — the `wasm32` counterpart to `main.rs`'s
//! `eframe::run_native`. Started once by `index.html`'s inline module
//! script, which calls this crate's generated `init` (via `data-trunk`'s
//! wasm-bindgen glue), which in turn invokes this `#[wasm_bindgen(start)]`
//! function.

use crate::app::ComicApp;
use wasm_bindgen::JsCast;

/// Id of the `<canvas>` element `index.html` provides for eframe to render
/// into.
const CANVAS_ID: &str = "comic_reader_canvas";

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    // Without this, a panic just silently kills the app with nothing in the
    // browser console — see `main.rs::install_crash_log_hook` for the native
    // equivalent (a panic here has no filesystem to log to).
    console_error_panic_hook::set_once();

    let document = web_sys::window()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("no global `window`"))?
        .document()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("no `document` on `window`"))?;
    let canvas = document
        .get_element_by_id(CANVAS_ID)
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("no element with id `comic_reader_canvas`"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    eframe::WebRunner::new()
        .start(canvas, eframe::WebOptions::default(), Box::new(|_cc| Ok(Box::new(ComicApp::new()))))
        .await
}
