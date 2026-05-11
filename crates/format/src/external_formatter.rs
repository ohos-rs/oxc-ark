#[cfg(feature = "napi")]
use std::future::Future;
#[cfg(feature = "napi")]
use std::path::Path;
#[cfg(feature = "napi")]
use std::sync::Arc;

#[cfg(feature = "napi")]
use napi::{
    Status,
    bindgen_prelude::{FnArgs, Promise, block_on},
    threadsafe_function::ThreadsafeFunction,
};
#[cfg(feature = "napi")]
use serde_json::Value;

/// Type alias for the init external formatter callback function signature.
/// Takes num_threads as argument and returns plugin languages.
#[cfg(feature = "napi")]
pub type JsInitExternalFormatterCb = ThreadsafeFunction<
    // Input arguments
    FnArgs<(u32,)>, // (num_threads,)
    // Return type (what JS function returns)
    Promise<Vec<String>>,
    // Arguments (repeated)
    FnArgs<(u32,)>,
    // Error status
    Status,
    // CalleeHandled
    false,
>;

/// Type alias for the callback function signature.
/// Takes (options, tag_name, code) as separate arguments and returns formatted code.
#[cfg(feature = "napi")]
pub type JsFormatEmbeddedCb = ThreadsafeFunction<
    // Input arguments
    FnArgs<(Value, String, String)>, // (options, tag_name, code)
    // Return type (what JS function returns)
    Promise<String>,
    // Arguments (repeated)
    FnArgs<(Value, String, String)>,
    // Error status
    Status,
    // CalleeHandled
    false,
>;

/// Type alias for the callback function signature.
/// Takes (options, parser_name, file_name, code) as separate arguments and returns formatted code.
#[cfg(feature = "napi")]
pub type JsFormatFileCb = ThreadsafeFunction<
    // Input arguments
    FnArgs<(Value, String, String, String)>, // (options, parser_name, file_name, code)
    // Return type (what JS function returns)
    Promise<String>,
    // Arguments (repeated)
    FnArgs<(Value, String, String, String)>,
    // Error status
    Status,
    // CalleeHandled
    false,
>;

/// Callback function type for formatting embedded code with config.
/// Takes (options, tag_name, code) and returns formatted code or an error.
#[cfg(feature = "napi")]
type FormatEmbeddedWithConfigCallback =
    Arc<dyn Fn(&Value, &str, &str) -> Result<String, String> + Send + Sync>;

/// Callback function type for formatting files with config.
/// Takes (options, parser_name, file_name, code) and returns formatted code or an error.
#[cfg(feature = "napi")]
type FormatFileWithConfigCallback =
    Arc<dyn Fn(&Value, &str, &str, &str) -> Result<String, String> + Send + Sync>;

/// Callback function type for init external formatter.
/// Takes num_threads and returns plugin languages.
#[cfg(feature = "napi")]
type InitExternalFormatterCallback =
    Arc<dyn Fn(usize) -> Result<Vec<String>, String> + Send + Sync>;

#[cfg(feature = "napi")]
fn block_on_js_callback<F, T>(future: F) -> Result<T, String>
where
    F: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    #[cfg(target_family = "wasm")]
    {
        std::thread::spawn(move || block_on(future))
            .join()
            .map_err(|_| "JS callback thread panicked".to_string())?
    }

    #[cfg(not(target_family = "wasm"))]
    {
        block_on(future)
    }
}

/// External formatter that wraps a JS callback.
#[cfg(feature = "napi")]
#[derive(Clone)]
pub struct ExternalFormatter {
    pub init: InitExternalFormatterCallback,
    pub format_embedded: FormatEmbeddedWithConfigCallback,
    pub format_file: FormatFileWithConfigCallback,
}

#[cfg(feature = "napi")]
impl std::fmt::Debug for ExternalFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalFormatter")
            .field("init", &"<callback>")
            .field("format_embedded", &"<callback>")
            .field("format_file", &"<callback>")
            .finish()
    }
}

#[cfg(feature = "napi")]
impl ExternalFormatter {
    /// Create an [`ExternalFormatter`] from JS callbacks.
    pub fn new(
        init_cb: JsInitExternalFormatterCb,
        format_embedded_cb: JsFormatEmbeddedCb,
        format_file_cb: JsFormatFileCb,
    ) -> Self {
        let rust_init = wrap_init_external_formatter(init_cb);
        let rust_format_embedded = wrap_format_embedded(format_embedded_cb);
        let rust_format_file = wrap_format_file(format_file_cb);
        Self {
            init: rust_init,
            format_embedded: rust_format_embedded,
            format_file: rust_format_file,
        }
    }

    /// Initialize external formatter using the JS callback.
    pub fn init(&self, num_threads: usize) -> Result<Vec<String>, String> {
        (self.init)(num_threads)
    }

    /// Convert this external formatter to oxc_formatter::ExternalCallbacks.
    /// Path, format_options and options are captured for embedded formatting.
    pub fn to_external_callbacks(
        &self,
        _path: &Path,
        _format_options: &oxc_formatter::FormatOptions,
        options: Value,
    ) -> oxc_formatter::ExternalCallbacks {
        let format_embedded = Arc::clone(&self.format_embedded);
        let embedded_cb: oxc_formatter::EmbeddedFormatterCallback =
            Arc::new(move |language: &str, code: &str| {
                let Some(parser_name) = language_to_prettier_parser(language) else {
                    return Err(format!("Unsupported language: {language}"));
                };
                (format_embedded)(&options, parser_name, code)
            });
        oxc_formatter::ExternalCallbacks::new()
            .with_embedded_formatter(Some(embedded_cb))
            .with_tailwind(None)
    }

    /// Format non-js file using the JS callback.
    pub fn format_file(
        &self,
        options: &Value,
        parser_name: &str,
        file_name: &str,
        code: &str,
    ) -> Result<String, String> {
        (self.format_file)(options, parser_name, file_name, code)
    }
}

// ---

/// Map oxc_formatter embedded language tags to Prettier parser names.
fn language_to_prettier_parser(language: &str) -> Option<&'static str> {
    match language {
        "tagged-css" | "styled-jsx" => Some("css"),
        "tagged-graphql" => Some("graphql"),
        "tagged-html" => Some("html"),
        "tagged-markdown" => Some("markdown"),
        "angular-template" => Some("angular"),
        "angular-styles" => Some("scss"),
        _ => None,
    }
}

// These wrappers expose async JS callbacks to the synchronous formatter core.
// WASI exports run inside the napi runtime, so the blocking wait happens on a
// separate thread there to avoid starting the same runtime recursively.

/// Wrap JS `initExternalFormatter` callback as a normal Rust function.
#[cfg(feature = "napi")]
fn wrap_init_external_formatter(cb: JsInitExternalFormatterCb) -> InitExternalFormatterCallback {
    let cb = Arc::new(cb);
    Arc::new(move |num_threads: usize| {
        let cb = Arc::clone(&cb);
        block_on_js_callback(async move {
            #[expect(clippy::cast_possible_truncation)]
            let status = cb.call_async(FnArgs::from((num_threads as u32,))).await;
            match status {
                Ok(promise) => match promise.await {
                    Ok(languages) => Ok(languages),
                    Err(err) => Err(format!("JS initExternalFormatter promise rejected: {err}")),
                },
                Err(err) => Err(format!(
                    "Failed to call JS initExternalFormatter callback: {err}"
                )),
            }
        })
    })
}

/// Wrap JS `formatEmbeddedCode` callback as a normal Rust function.
#[cfg(feature = "napi")]
fn wrap_format_embedded(cb: JsFormatEmbeddedCb) -> FormatEmbeddedWithConfigCallback {
    let cb = Arc::new(cb);
    Arc::new(move |options: &Value, tag_name: &str, code: &str| {
        let cb = Arc::clone(&cb);
        let options = options.clone();
        let tag_name = tag_name.to_string();
        let code = code.to_string();
        block_on_js_callback(async move {
            let status = cb
                .call_async(FnArgs::from((options, tag_name.clone(), code)))
                .await;
            match status {
                Ok(promise) => match promise.await {
                    Ok(formatted_code) => Ok(formatted_code),
                    Err(err) => Err(format!(
                        "JS formatter promise rejected for tag '{tag_name}': {err}"
                    )),
                },
                Err(err) => Err(format!(
                    "Failed to call JS formatting callback for tag '{tag_name}': {err}"
                )),
            }
        })
    })
}

/// Wrap JS `formatFile` callback as a normal Rust function.
#[cfg(feature = "napi")]
fn wrap_format_file(cb: JsFormatFileCb) -> FormatFileWithConfigCallback {
    let cb = Arc::new(cb);
    Arc::new(
        move |options: &Value, parser_name: &str, file_name: &str, code: &str| {
            let cb = Arc::clone(&cb);
            let options = options.clone();
            let parser_name = parser_name.to_string();
            let file_name = file_name.to_string();
            let code = code.to_string();
            block_on_js_callback(async move {
                let status = cb
                    .call_async(FnArgs::from((
                        options,
                        parser_name.clone(),
                        file_name.clone(),
                        code,
                    )))
                    .await;
                match status {
                    Ok(promise) => match promise.await {
                        Ok(formatted_code) => Ok(formatted_code),
                        Err(err) => Err(format!(
                            "JS formatFile promise rejected for file: '{file_name}', parser: '{parser_name}': {err}"
                        )),
                    },
                    Err(err) => Err(format!(
                        "Failed to call JS formatFile callback for file: '{file_name}', parser: '{parser_name}': {err}"
                    )),
                }
            })
        },
    )
}
