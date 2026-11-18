// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use oxc_formatter_core::{CoreFormatOptions, FormatOptions};
use oxc_formatter_graphql::{BracketSpacing, GraphqlFormatOptions};

use super::super::oxfmtrc::FormatConfig;

/// Convert `FormatConfig` into `GraphqlFormatOptions` for `oxc_formatter_graphql`.
///
/// Prettier's `graphql` language consumes only the shared layout options plus `bracketSpacing`.
///
/// NOTE: Pure field translation:
/// `core` comes pre-validated from the config-resolution gate (`validate()`), so this cannot fail.
pub fn to_oxc_formatter_graphql(
    config: &FormatConfig,
    core_options: CoreFormatOptions,
) -> GraphqlFormatOptions {
    let mut options = GraphqlFormatOptions::default();
    options.apply_core(core_options);

    // [Prettier] bracketSpacing: boolean
    if let Some(spacing) = config.bracket_spacing {
        options.bracket_spacing = BracketSpacing::from(spacing);
    }

    options
}
