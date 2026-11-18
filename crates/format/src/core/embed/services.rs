//! Root `SessionServices` assembly: [`for_root`] builds the build's default service set,
//! installed by every session-taking root (JS / CSS / Markdown / HTML / Svelte-Astro script).
//! One name, one definition per build; the napi one additionally takes the `ExternalServices` transport.
//!
//! "Which languages may dispatch at all from this host" is the host crate's own gate
//! (e.g. the CSS crate's front matter gate dispatches only `yaml` / `toml`), not a concern here;
//! a root needing a bespoke set would assemble the `SessionServices` struct literally, none does today.

use std::sync::Arc;

use oxc_formatter_core::SessionServices;
#[cfg(feature = "napi")]
use oxc_formatter_core::TailwindSorter;
#[cfg(feature = "napi")]
use tracing::debug_span;

#[cfg(feature = "napi")]
use crate::core::ExternalServices;

use super::dispatcher::ResolvedDispatchConfig;

/// The pure Rust session services: the fallback-less registry dispatcher plus the native-fence
/// string embedder. Also used as the fallback for napi sessions that have no JS transport
/// (e.g. the formatter LSP, which has no Prettier callbacks).
pub fn native(dispatch_config: &Arc<ResolvedDispatchConfig>) -> SessionServices {
    // A fence dispatcher and this root's dispatcher are the same fallback-less registry,
    // so the root's doubles as the fence adapter's.
    let dispatcher = dispatch_config.root_dispatcher(None);
    let string_embedder = dispatcher.as_ref().map(|dispatcher| {
        super::jsdoc_fence::build_string_embedder(
            Arc::clone(dispatcher),
            Arc::clone(dispatch_config),
            None,
        )
    });
    SessionServices {
        dispatcher,
        string_embedder,
        tailwind_sorter: None,
    }
}

/// The pure build's default `SessionServices` for a root formatter run.
#[cfg(not(feature = "napi"))]
pub fn for_root(dispatch_config: &Arc<ResolvedDispatchConfig>) -> SessionServices {
    native(dispatch_config)
}

/// The napi build's default `SessionServices` for a root formatter run:
/// the registry dispatcher with the Prettier Doc→IR fallback,
/// the string embedder, and the Tailwind sorter.
/// The options (including `filepath`) are captured in the closures and passed to JS on each call;
/// per-language options are mapped lazily at dispatch time (see [`ResolvedDispatchConfig`]).
///
/// This is the assembly point only: closure factories live in `embed::{jsdoc_fence, dispatcher, prettier_doc}`,
/// and the napi callback `Arc`s come from the transport ([`ExternalServices`]).
///
/// NOTE: Tailwind data paths
/// `sort_tailwindcss_classes` is wired into THREE distinct callback slots,
/// because Tailwind class sorting has four legitimate paths depending on where the classes live and how they reach printing:
///
/// 1. Standalone JS/TS (`className` / functions / custom attributes):
///    `oxc_formatter` collects classes into `FormatElement::TailwindClass`.
///    When the entry document is finalized, the printer sorts them in one host batch via the session's `tailwind_sorter`.
/// 2. Standalone CSS / SCSS / LESS (`@apply` at top level), Markdown, and HTML / Vue (`class` and the listed attributes):
///    the root's session carries the same sorter (`CssFormatOptions::sort_tailwindcss` / `HtmlishFormatOptions::sort_tailwindcss` only switch collection);
///    classes sort once at finalize, no embedded boundary involved.
/// 3. Embedded parts (css-in-js + Angular `@Component({ styles })`, Markdown code blocks,
///    HTML / Vue `<script>` / `<style>` bodies and attribute values, Vue's `{{ }}` and directive values):
///    each child's `format_to_ir()` adds pre-sort classes into the session's class scope, shared with the parent,
///    so they ride the SAME parent batch as its root.
///    No sort closure is invoked for the child itself.
/// 4. JSDoc fenced blocks (Markdown code fence in JSDoc descriptions):
///    Goes through the string embedder's fence adapter, which returns a formatted string (not parent-integrated IR),
///    so there is no parent index space to join:
///    `FormatSession::dispatch_to_string` collects the child's classes in a fresh scope and sorts it before printing.
///    The `jsdoc_fence::build_string_embedder` factory receives the sorter for this case.
///
/// All four paths use the SAME `sort_tailwindcss_classes` napi callback; only the wiring differs.
/// Moving sorting to the wrong layer (e.g. sorting inside embedded CSS instead of at the root) would double-sort or drop classes.
#[cfg(feature = "napi")]
pub fn for_root(
    external_services: &ExternalServices,
    dispatch_config: &Arc<ResolvedDispatchConfig>,
) -> SessionServices {
    let needs_embedded = dispatch_config.is_embedded_formatting_enabled();
    // Built only when the off-gate passes;
    // `root_dispatcher` re-checks the same predicate, so an off run allocates neither fallback nor dispatcher.
    let fallback = needs_embedded.then(|| {
        super::prettier_doc::build_prettier_fallback(
            Arc::clone(dispatch_config),
            Arc::clone(&external_services.format_embedded_doc),
        )
    });

    // The ONE pre-bound Tailwind sorter (options JSON applied here, once);
    // the string embedder receives a clone of the same sorter for fence-collected classes.
    let sorter = dispatch_config.is_tailwind_enabled().then(|| {
        let sort = Arc::clone(&external_services.sort_tailwindcss_classes);
        let dispatch_config = Arc::clone(dispatch_config);
        Arc::new(move |classes: Vec<String>| {
            debug_span!(
                "oxfmt::external::sort_tailwind",
                classes_count = classes.len()
            )
            .in_scope(|| (sort)(dispatch_config.prettier_options(), classes))
        }) as TailwindSorter
    });

    // Fences route native languages to the registry and Prettier-served languages through
    // the JS string callback (never through the Doc fallback).
    let string_embedder = needs_embedded.then(|| {
        super::prettier_string::build_string_embedder(
            Arc::clone(&external_services.format_embedded),
            sorter.clone(),
            Arc::clone(dispatch_config),
        )
    });

    SessionServices {
        dispatcher: dispatch_config.root_dispatcher(fallback),
        string_embedder,
        tailwind_sorter: sorter,
    }
}
