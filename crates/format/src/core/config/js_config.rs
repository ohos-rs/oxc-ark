// Derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::sync::Arc;

use napi::{Status, bindgen_prelude::Promise, threadsafe_function::ThreadsafeFunction};
use serde_json::Value;

/// JS callback to load a single JavaScript/TypeScript config file.
/// Takes an absolute path string and returns the config object directly.
pub type JsLoadJsConfigCb = ThreadsafeFunction<
    // Arguments: absolute path to config file
    String,
    // Return value: config object as serde_json::Value
    Promise<Value>,
    // Arguments (repeated)
    String,
    // Error status
    Status,
    // CalleeHandled
    false,
>;

/// Callback type for loading a JavaScript/TypeScript config file.
///
/// Wraps the NAPI `ThreadsafeFunction` and blocks the current thread until the JS callback resolves.
/// Uses `Arc` so it can be shared across CLI, Stdin, and LSP code paths.
pub type JsConfigLoaderCb = Arc<dyn Fn(String) -> Result<Value, String> + Send + Sync>;

/// Create a JS config loader callback from the NAPI JS callback.
///
/// The returned function blocks the current thread until the JS callback resolves.
///
/// The loader can be invoked from any thread, including:
/// - rayon worker threads (walk visitors during on-demand discovery)
/// - Tokio worker threads (direct file resolution, stdin, LSP)
/// - bare threads (any other context that obtained a `JsConfigLoaderCb`)
#[cfg(not(target_family = "wasm"))]
pub fn create_js_config_loader(cb: JsLoadJsConfigCb) -> JsConfigLoaderCb {
    let handle = tokio::runtime::Handle::current();
    Arc::new(move |path: String| {
        let cb = &cb;
        let handle = handle.clone();
        let fut = async move { cb.call_async(path).await?.into_future().await };

        let res = if tokio::runtime::Handle::try_current().is_ok() {
            // Let the Tokio worker be reused while we block on the NAPI promise.
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else {
            // Outside a Tokio runtime, drive the future directly via the captured handle.
            handle.block_on(fut)
        };
        res.map_err(|e| e.reason.clone())
    })
}

/// WASI NAPI entry points are synchronous, so there is no Tokio runtime to drive the
/// promise. Blocks on the NAPI future directly.
#[cfg(target_family = "wasm")]
pub fn create_js_config_loader(cb: JsLoadJsConfigCb) -> JsConfigLoaderCb {
    Arc::new(move |path: String| {
        let cb = &cb;
        napi::bindgen_prelude::block_on(
            async move { cb.call_async(path).await?.into_future().await },
        )
        .map_err(|e| e.reason.clone())
    })
}
