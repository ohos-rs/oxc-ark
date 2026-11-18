// Derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::{path::Path, sync::Arc};

use super::{
    ConfigLoader, ConfigResolver, JsConfigLoaderCb, NestedConfigCtx,
    editorconfig::{load_editorconfig, resolve_editorconfig_path},
};

/// Root config and on-demand nested configs for a project rooted at `cwd`.
///
/// Holds the state shared by every file resolved under the same entry point run,
/// so each config file (and `.editorconfig`) is loaded at most once.
///
/// Cloning is shallow, clones share the same caches.
#[derive(Clone)]
pub struct ConfigScopes {
    root: Arc<ConfigResolver>,
    /// Always present, even when nested detection is disabled,
    /// since the walker also uses it to recognize config files.
    nested_ctx: NestedConfigCtx,
    use_nested: bool,
    has_editorconfig: bool,
}

impl ConfigScopes {
    /// Load `.editorconfig` nearest to `cwd` and the root config,
    /// discovered upwards from `cwd` or given by `explicit_config`.
    ///
    /// # Errors
    /// Returns a message ready to print if loading or validation fails.
    pub fn load(
        cwd: &Path,
        explicit_config: Option<&Path>,
        use_nested: bool,
        js_config_loader: Option<&JsConfigLoaderCb>,
    ) -> Result<Self, String> {
        Self::load_impl(cwd, explicit_config, use_nested, None, js_config_loader)
    }

    /// Like [`Self::load`], but uses `fallback_config` (for example CLI flags)
    /// as the root config when no config file is found.
    pub fn load_with_fallback_config(
        cwd: &Path,
        explicit_config: Option<&Path>,
        use_nested: bool,
        fallback_config: Option<serde_json::Value>,
        js_config_loader: Option<&JsConfigLoaderCb>,
    ) -> Result<Self, String> {
        Self::load_impl(
            cwd,
            explicit_config,
            use_nested,
            fallback_config,
            js_config_loader,
        )
    }

    fn load_impl(
        cwd: &Path,
        explicit_config: Option<&Path>,
        use_nested: bool,
        fallback_config: Option<serde_json::Value>,
        js_config_loader: Option<&JsConfigLoaderCb>,
    ) -> Result<Self, String> {
        let load_err = |err: String| format!("Failed to load configuration file.\n{err}");

        let editorconfig =
            load_editorconfig(resolve_editorconfig_path(cwd).as_deref()).map_err(load_err)?;
        let loader = ConfigLoader::new(editorconfig, js_config_loader.cloned());
        let mut root = loader
            .load_root(cwd, explicit_config, fallback_config)
            .map_err(load_err)?;
        root.build_and_validate()
            .map_err(|err| format!("Failed to parse configuration.\n{err}"))?;

        Ok(Self::new(root, loader, use_nested))
    }

    /// Same as [`Self::load`], but with the default (empty) root config.
    ///
    /// For LSP, which keeps formatting with defaults when the root config is broken,
    /// while nested configs are still detected.
    #[cfg(feature = "napi")]
    pub fn with_default_root(
        cwd: &Path,
        use_nested: bool,
        js_config_loader: Option<&JsConfigLoaderCb>,
    ) -> Self {
        let mut root = ConfigResolver::from_json_config(None, None)
            .expect("Default ConfigResolver should never fail");
        root.build_and_validate()
            .expect("Default ConfigResolver validation should never fail");

        // Best effort: an unreadable `.editorconfig` is skipped instead of failing again
        let editorconfig = load_editorconfig(resolve_editorconfig_path(cwd).as_deref())
            .ok()
            .flatten();
        Self::new(
            root,
            ConfigLoader::new(editorconfig, js_config_loader.cloned()),
            use_nested,
        )
    }

    fn new(root: ConfigResolver, loader: ConfigLoader, use_nested: bool) -> Self {
        let root = Arc::new(root);
        let has_editorconfig = loader.editorconfig.is_some();
        let nested_ctx = NestedConfigCtx::new(&root, loader);
        Self {
            root,
            nested_ctx,
            use_nested,
            has_editorconfig,
        }
    }

    /// Resolve the config scope for a single file: the nearest nested config, or the root.
    ///
    /// When nested detection is enabled, the ancestor chain of `path` is walked.
    /// The root is registered in `nested_ctx`, so reaching its `config_dir()` returns it without reloading.
    ///
    /// # Errors
    /// Returns error if a nested config fails to load.
    pub fn resolve(&self, path: &Path) -> Result<Arc<ConfigResolver>, String> {
        if !self.use_nested {
            return Ok(Arc::clone(&self.root));
        }
        let Some(parent) = path.parent() else {
            return Ok(Arc::clone(&self.root));
        };

        for dir in parent.ancestors() {
            if let Some(r) = self.nested_ctx.probe_dir(dir)? {
                return Ok(r);
            }
        }

        Ok(Arc::clone(&self.root))
    }

    pub fn root(&self) -> &Arc<ConfigResolver> {
        &self.root
    }

    pub fn nested_ctx(&self) -> &NestedConfigCtx {
        &self.nested_ctx
    }

    pub fn use_nested(&self) -> bool {
        self.use_nested
    }

    /// Whether any config file or `.editorconfig` has been found so far.
    ///
    /// Nested configs are detected lazily, call this after resolving files.
    pub fn any_config_found(&self) -> bool {
        self.nested_ctx.config_found() || self.has_editorconfig
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    use super::ConfigScopes;
    use crate::ResolveOutcome;

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).expect("config should be written");
    }

    fn resolved_print_width(scopes: &ConfigScopes, path: &Path) -> Option<u16> {
        let resolver = scopes.resolve(path).expect("scope should resolve");
        let strategy = resolver
            .classify_with_explicit_language(path, None)
            .expect("file should classify");
        match resolver
            .resolve_outcome(path, strategy)
            .expect("config should resolve")
        {
            ResolveOutcome::Format(plan) => plan.config.print_width,
            ResolveOutcome::MissingPlugin(_) => panic!("unexpected missing plugin"),
        }
    }

    #[test]
    fn nested_config_wins_over_root() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("src dir");
        write(root, ".oxfmtrc.json", r#"{ "printWidth": 100 }"#);
        write(&src, ".oxfmtrc.json", r#"{ "printWidth": 60 }"#);

        let scopes = ConfigScopes::load(root, None, true, None).expect("scopes should load");

        assert_eq!(
            resolved_print_width(&scopes, &root.join("root.ts")),
            Some(100)
        );
        assert_eq!(
            resolved_print_width(&scopes, &src.join("nested.ts")),
            Some(60)
        );
    }

    #[test]
    fn root_json_is_preferred_over_jsonc() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        write(root, ".oxfmtrc.json", r#"{ "printWidth": 100 }"#);
        write(root, ".oxfmtrc.jsonc", r#"{ "printWidth": 40 }"#);

        let scopes = ConfigScopes::load(root, None, true, None).expect("scopes should load");
        assert_eq!(
            resolved_print_width(&scopes, &root.join("file.ts")),
            Some(100)
        );
    }

    #[test]
    fn fallback_config_applies_when_no_config_file_exists() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();

        let scopes = ConfigScopes::load_with_fallback_config(
            root,
            None,
            true,
            Some(serde_json::json!({ "singleQuote": true })),
            None,
        )
        .expect("scopes should load");

        let path = root.join("file.ts");
        let resolver = scopes.resolve(&path).expect("scope should resolve");
        let strategy = resolver
            .classify_with_explicit_language(&path, None)
            .expect("file should classify");
        let ResolveOutcome::Format(plan) = resolver
            .resolve_outcome(&path, strategy)
            .expect("config should resolve")
        else {
            panic!("unexpected missing plugin");
        };
        assert_eq!(plan.config.single_quote, Some(true));
    }

    #[test]
    fn config_file_wins_over_fallback_config() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        write(root, ".oxfmtrc.json", r#"{ "singleQuote": false }"#);

        let scopes = ConfigScopes::load_with_fallback_config(
            root,
            None,
            true,
            Some(serde_json::json!({ "singleQuote": true })),
            None,
        )
        .expect("scopes should load");

        let path = root.join("file.ts");
        let resolver = scopes.resolve(&path).expect("scope should resolve");
        let strategy = resolver
            .classify_with_explicit_language(&path, None)
            .expect("file should classify");
        let ResolveOutcome::Format(plan) = resolver
            .resolve_outcome(&path, strategy)
            .expect("config should resolve")
        else {
            panic!("unexpected missing plugin");
        };
        assert_eq!(plan.config.single_quote, Some(false));
    }

    #[test]
    fn nested_associations_apply_to_unclassified_extensions() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("src dir");
        write(
            &src,
            ".oxfmtrc.json",
            r#"{ "associations": [{ "files": ["*.wxml"], "language": "html" }] }"#,
        );

        let scopes = ConfigScopes::load(root, None, true, None).expect("scopes should load");
        let resolver = scopes
            .resolve(&src.join("page.wxml"))
            .expect("scope should resolve");
        let strategy = resolver
            .classify_with_explicit_language(&src.join("page.wxml"), None)
            .expect("nested association should classify");
        assert!(matches!(
            strategy,
            crate::core::support::FormatStrategy::Native(
                crate::core::support::NativeLanguage::Html(_)
            )
        ));
    }

    #[test]
    fn any_config_found_reports_root_config() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        write(root, ".oxfmtrc.json", "{}");

        let scopes = ConfigScopes::load(root, None, true, None).expect("scopes should load");
        assert!(scopes.any_config_found());
    }

    #[cfg(not(feature = "napi"))]
    #[test]
    fn js_config_requires_a_loader() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        write(root, "oxfmt.config.ts", "export default {};");

        let error = ConfigScopes::load(root, None, true, None)
            .err()
            .expect("JS config should require a loader");
        assert!(error.contains("not supported"), "unexpected error: {error}");
    }
}
