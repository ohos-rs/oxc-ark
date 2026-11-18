# format for oxk

This code is forked from oxc.

The formatter core, option mapping, native language dispatch, and Prettier
document bridge are synchronized with `ohos-rs/oxc` commit
`1526b2c932d211c25d85dd172018889d0aeda776` (oxfmt 0.79.0).
CSS/SCSS/Less, GraphQL, YAML, Markdown, JSON, and TOML use native formatters,
including supported embedded languages. `sortImports`, `jsdoc`, per-file
`overrides`, and EditorConfig use the upstream options. The npm wrapper also
provides the matching Prettier, Astro, Svelte, and Tailwind plugins.

By default, string values use double quotes and `quoteProps` is `preserve`,
retaining explicit property quotes. Explicit `quoteProps` and `singleQuote`
options take precedence.

Static ETS / ArkTS 1.2 uses the explicit `ets-static` language mode. Call
`FormatFileStrategy::from_path_with_language` with
`ExplicitLanguage::EtsStatic`; ordinary `.ets` classification remains ArkTS
1.1.
