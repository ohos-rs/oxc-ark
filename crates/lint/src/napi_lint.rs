#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
use std::sync::atomic::Ordering;
use std::{
    ffi::OsString,
    future::Future,
    sync::{Arc, OnceLock, mpsc::channel},
};

use napi::{
    Status,
    bindgen_prelude::{FnArgs, Promise, Uint8Array},
    threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
};
use oxc_allocator::Allocator;
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
use oxc_allocator::free_fixed_size_allocator;
use oxc_linter::{
    ExternalLinter, ExternalLinterCreateWorkspaceCb, ExternalLinterDestroyWorkspaceCb,
    ExternalLinterLintFileCb, ExternalLinterLoadPluginCb, ExternalLinterSetupRuleConfigsCb,
    LintFileResult, LoadPluginResult,
};
use serde::Deserialize;

use oxlint::cli::{init_miette, init_tracing};

use crate::{
    arkts::{self, ExternalLinterCallbacks},
    handle_threads_once, parse_lint_command, prepare_arkts_config, resolve_from_cwd,
    run_lint_command, run_lsp_server,
};

#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
const BLOCK_ALIGN: usize = 4_294_967_296;
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
const BUFFER_SIZE: usize = 2_147_483_576;

fn block_on_napi<F: Future>(future: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to initialize lint NAPI runtime")
        })
        .block_on(future)
}

pub type JsLoadPluginCb = ThreadsafeFunction<
    FnArgs<(String, Option<String>, bool, Option<String>)>,
    Promise<String>,
    FnArgs<(String, Option<String>, bool, Option<String>)>,
    Status,
    false,
>;

pub type JsLintFileCb = ThreadsafeFunction<
    FnArgs<(
        String,
        u32,
        Option<Uint8Array>,
        Vec<u32>,
        Vec<u32>,
        String,
        String,
        Option<String>,
    )>,
    Promise<Option<String>>,
    FnArgs<(
        String,
        u32,
        Option<Uint8Array>,
        Vec<u32>,
        Vec<u32>,
        String,
        String,
        Option<String>,
    )>,
    Status,
    false,
>;

pub type JsSetupRuleConfigsCb = ThreadsafeFunction<String, Option<String>, String, Status, false>;

pub type JsCreateWorkspaceCb = ThreadsafeFunction<String, Promise<()>, String, Status, false>;

pub type JsDestroyWorkspaceCb = ThreadsafeFunction<String, (), String, Status, false>;

pub type JsLoadJsConfigsCb =
    ThreadsafeFunction<Vec<String>, Promise<String>, Vec<String>, Status, false>;

pub async fn lint_args_with_plugins(
    args: Vec<String>,
    load_plugin: JsLoadPluginCb,
    setup_rule_configs: JsSetupRuleConfigsCb,
    lint_file: JsLintFileCb,
    create_workspace: JsCreateWorkspaceCb,
    destroy_workspace: JsDestroyWorkspaceCb,
    _load_js_configs: JsLoadJsConfigsCb,
) -> bool {
    let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
    let command = match parse_lint_command(&args) {
        Ok(command) => command,
        Err(success) => return success,
    };

    init_tracing();

    let external_linter = create_external_linter(
        load_plugin,
        setup_rule_configs,
        lint_file,
        create_workspace,
        destroy_workspace,
    );

    if command.lsp {
        let config_path = command.basic_options.config.clone().map(resolve_from_cwd);
        return run_lsp_server(Some(external_linter), config_path);
    }

    let prepared = match prepare_arkts_config(args) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("{err}");
            return false;
        }
    };

    let command = match parse_lint_command(&prepared.args) {
        Ok(command) => command,
        Err(success) => return success,
    };

    init_miette();
    handle_threads_once(&command);

    run_lint_command(command, prepared.arkts, Some(external_linter))
}

fn create_external_linter(
    load_plugin: JsLoadPluginCb,
    setup_rule_configs: JsSetupRuleConfigsCb,
    lint_file: JsLintFileCb,
    create_workspace: JsCreateWorkspaceCb,
    destroy_workspace: JsDestroyWorkspaceCb,
) -> ExternalLinter {
    arkts::create_external_linter(Some(ExternalLinterCallbacks {
        load_plugin: wrap_load_plugin(load_plugin),
        setup_rule_configs: wrap_setup_rule_configs(setup_rule_configs),
        lint_file: wrap_lint_file(lint_file),
        create_workspace: wrap_create_workspace(create_workspace),
        destroy_workspace: wrap_destroy_workspace(destroy_workspace),
    }))
}

#[derive(Clone, Debug, Deserialize)]
enum LoadPluginReturnValue {
    Success(LoadPluginResult),
    Failure(String),
}

fn wrap_load_plugin(cb: JsLoadPluginCb) -> ExternalLinterLoadPluginCb {
    Arc::new(Box::new(
        move |plugin_url, plugin_name, plugin_name_is_alias, workspace_uri| {
            let cb = &cb;
            let res = block_on_napi(async move {
                cb.call_async(FnArgs::from((
                    plugin_url,
                    plugin_name,
                    plugin_name_is_alias,
                    workspace_uri,
                )))
                .await?
                .into_future()
                .await
            });

            match res {
                Ok(json) => match serde_json::from_str(&json) {
                    Ok(LoadPluginReturnValue::Success(result)) => Ok(result),
                    Ok(LoadPluginReturnValue::Failure(err)) => Err(err),
                    Err(err) => Err(format!(
                        "Failed to deserialize JSON returned by `loadPlugin`: {err}"
                    )),
                },
                Err(err) => Err(format!("`loadPlugin` threw an error: {err}")),
            }
        },
    ))
}

fn wrap_setup_rule_configs(cb: JsSetupRuleConfigsCb) -> ExternalLinterSetupRuleConfigsCb {
    Arc::new(Box::new(move |options_json| {
        let (tx, rx) = channel();
        let status = cb.call_with_return_value(
            options_json,
            ThreadsafeFunctionCallMode::NonBlocking,
            move |result, _env| {
                let _ = tx.send(result);
                Ok(())
            },
        );

        if status != Status::Ok {
            return Err(format!(
                "Failed to schedule `setupRuleConfigs` callback: {status:?}"
            ));
        }

        match rx.recv() {
            Ok(Ok(None)) => Ok(()),
            Ok(Ok(Some(err))) => Err(err),
            Ok(Err(err)) => Err(format!("`setupRuleConfigs` threw an error: {err}")),
            Err(err) => Err(format!("`setupRuleConfigs` did not respond: {err}")),
        }
    }))
}

#[derive(Clone, Debug, Deserialize)]
enum LintFileReturnValue {
    Success(Vec<LintFileResult>),
    Failure(String),
}

fn wrap_lint_file(cb: JsLintFileCb) -> ExternalLinterLintFileCb {
    Arc::new(Box::new(
        move |file_path,
              rule_ids,
              options_ids,
              settings_json,
              globals_json,
              workspace_uri,
              allocator| {
            let cb = &cb;
            // SAFETY: oxlint creates fixed-size allocators when an external linter is present.
            let (buffer_id, buffer) = unsafe { get_buffer(allocator) };
            let res = block_on_napi(async move {
                cb.call_async(FnArgs::from((
                    file_path,
                    buffer_id,
                    buffer,
                    rule_ids,
                    options_ids,
                    settings_json,
                    globals_json,
                    workspace_uri,
                )))
                .await?
                .into_future()
                .await
            });

            match res {
                Ok(None) => Ok(Vec::new()),
                Ok(Some(json)) => match serde_json::from_str(&json) {
                    Ok(LintFileReturnValue::Success(diagnostics)) => Ok(diagnostics),
                    Ok(LintFileReturnValue::Failure(err)) => Err(err),
                    Err(err) => Err(format!(
                        "Failed to deserialize JSON returned by `lintFile`: {err}"
                    )),
                },
                Err(err) => Err(format!("`lintFile` threw an error: {err}")),
            }
        },
    ))
}

#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
unsafe fn get_buffer(allocator: &Allocator) -> (u32, Option<Uint8Array>) {
    // SAFETY: Caller guarantees the allocator was created by `FixedSizeAllocator`.
    let metadata_ptr = unsafe { allocator.fixed_size_metadata_ptr() };
    // SAFETY: Fixed-size allocators store valid metadata at this pointer.
    let metadata = unsafe { metadata_ptr.as_ref() };
    let buffer_id = metadata.id;

    if metadata.is_double_owned.swap(true, Ordering::SeqCst) {
        return (buffer_id, None);
    }

    // SAFETY: Fixed-size allocator chunks are aligned to `BLOCK_ALIGN`, and the
    // transfer buffer lives at the start of that chunk.
    let chunk_ptr = unsafe {
        let ptr = metadata_ptr.cast::<u8>();
        let offset = ptr.addr().get() % BLOCK_ALIGN;
        ptr.sub(offset)
    };

    // SAFETY: The JS side keeps an immutable view over the transfer buffer. The
    // finalizer releases the Rust allocator chunk once both runtimes have dropped it.
    let buffer = unsafe {
        Uint8Array::with_external_data(chunk_ptr.as_ptr(), BUFFER_SIZE, move |_ptr, _len| {
            free_fixed_size_allocator(metadata_ptr);
        })
    };

    (buffer_id, Some(buffer))
}

#[cfg(not(all(target_pointer_width = "64", target_endian = "little")))]
unsafe fn get_buffer(_allocator: &Allocator) -> (u32, Option<Uint8Array>) {
    (0, None)
}

fn wrap_create_workspace(cb: JsCreateWorkspaceCb) -> ExternalLinterCreateWorkspaceCb {
    Arc::new(Box::new(move |workspace_uri| {
        let cb = &cb;
        let res =
            block_on_napi(async move { cb.call_async(workspace_uri).await?.into_future().await });

        res.map_err(|err| format!("`createWorkspace` threw an error: {err}"))
    }))
}

fn wrap_destroy_workspace(cb: JsDestroyWorkspaceCb) -> ExternalLinterDestroyWorkspaceCb {
    Arc::new(Box::new(move |workspace_uri| {
        let (tx, rx) = channel();
        let status = cb.call_with_return_value(
            workspace_uri,
            ThreadsafeFunctionCallMode::NonBlocking,
            move |result, _env| {
                let _ = tx.send(result);
                Ok(())
            },
        );

        if status != Status::Ok {
            return Err(format!(
                "Failed to schedule `destroyWorkspace` callback: {status:?}"
            ));
        }

        match rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(format!("`destroyWorkspace` threw an error: {err}")),
            Err(err) => Err(format!("`destroyWorkspace` did not respond: {err}")),
        }
    }))
}
