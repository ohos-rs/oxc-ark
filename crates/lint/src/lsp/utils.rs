// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use std::borrow::Cow;

use oxc_diagnostics::{OxcCode, OxcDiagnostic};
use tower_lsp_server::gen_lsp_types::Range;

/// Returns `true` if LSP ranges `a` and `b` overlap or touch (share a boundary point).
///
/// This uses non-strict comparisons (`<=`/`>=`), so adjacent ranges where the end of one
/// equals the start of the other are also considered overlapping. This is intentional for
/// code-action filtering in the LSP server (a cursor at a boundary position should match
/// actions on either side), and is used conservatively in `fix_all_text_edit` to avoid
/// applying edits that share a boundary (which is rare in practice and safe to defer).
pub(super) fn range_overlaps(a: Range, b: Range) -> bool {
    a.start <= b.end && a.end >= b.start
}

/// Converts an `OxcCode` to a full rule name string, including plugin prefix if applicable.
///
/// this conversion is a bit messy, but basically we need to reconstruct the rule name with plugin prefix
pub fn get_full_rule_name(rule_code: &OxcCode) -> Option<Cow<'_, str>> {
    let rule_name = rule_code.number.as_ref()?;
    let scope = rule_code.scope.as_ref()?;

    // eslint does not have a plugin prefix
    if scope == "eslint" || scope.is_empty() {
        Some(Cow::Borrowed(rule_name))
    } else {
        Some(Cow::Owned(format!("{scope}/{rule_name}")))
    }
}

/// Returns a full diagnostic message string, including the main message, help text, and note if present.
pub fn get_oxc_diagnostic_full_message(diagnostic: &OxcDiagnostic) -> String {
    let mut diagnostic_message = String::with_capacity(
        diagnostic.message.len()
            + diagnostic.help.as_ref().map_or(0, |h| h.len() + 7) // "help: " prefix
            + diagnostic.note.as_ref().map_or(0, |n| n.len() + 7), // "note: " prefix
    );

    diagnostic_message.push_str(&diagnostic.message);
    if let Some(help) = &diagnostic.help {
        diagnostic_message.push_str("\nhelp: ");
        diagnostic_message.push_str(help);
    }

    if let Some(note) = &diagnostic.note {
        diagnostic_message.push_str("\nnote: ");
        diagnostic_message.push_str(note);
    }
    diagnostic_message
}
