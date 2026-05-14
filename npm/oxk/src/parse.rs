#![allow(dead_code)]

use napi_derive::napi;
use oxc_allocator::Allocator;
use oxc_napi::{convert_utf8_to_utf16, Comment, OxcError};
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::{SourceType, Span as OxcSpan};
use oxc_syntax::module_record::{self, ModuleRecord};
use rustc_hash::FxHashMap;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AstType {
  JavaScript,
  TypeScript,
}

#[napi(object)]
#[derive(Default)]
pub struct ParserOptions {
  /// Treat the source text as `js`, `jsx`, `ts`, `tsx`, `dts` or `ets`.
  #[napi(ts_type = "'js' | 'jsx' | 'ts' | 'tsx' | 'dts' | 'ets'")]
  pub lang: Option<String>,

  /// Treat the source text as `script`, `module`, `commonjs` or `unambiguous` code.
  #[napi(ts_type = "'script' | 'module' | 'commonjs' | 'unambiguous' | undefined")]
  pub source_type: Option<String>,

  /// Return an AST which includes TypeScript-related properties, or excludes them.
  #[napi(ts_type = "'js' | 'ts'")]
  pub ast_type: Option<String>,

  /// Controls whether the `range` property is included on AST nodes.
  #[napi(ts_type = "boolean")]
  pub range: Option<bool>,

  /// Emit `ParenthesizedExpression` and `TSParenthesizedType` in AST.
  pub preserve_parens: Option<bool>,

  /// Produce semantic errors with an additional AST pass.
  pub show_semantic_errors: Option<bool>,
}

#[napi(object)]
pub struct ParseResult {
  /// ESTree-compatible AST object.
  #[napi(ts_type = "any")]
  pub program: Value,
  pub module: EcmaScriptModule,
  pub comments: Vec<Comment>,
  pub errors: Vec<OxcError>,
}

#[napi(object)]
#[derive(Default)]
pub struct EcmaScriptModule {
  /// Has ESM syntax, such as import/export statements or `import.meta`.
  pub has_module_syntax: bool,
  /// Import statements.
  pub static_imports: Vec<StaticImport>,
  /// Export statements.
  pub static_exports: Vec<StaticExport>,
  /// Dynamic import expressions.
  pub dynamic_imports: Vec<DynamicImport>,
  /// Span positions of `import.meta`.
  pub import_metas: Vec<Span>,
}

#[napi(object)]
pub struct Span {
  pub start: u32,
  pub end: u32,
}

#[napi(object)]
pub struct ValueSpan {
  pub value: String,
  pub start: u32,
  pub end: u32,
}

#[napi(object)]
pub struct StaticImport {
  /// Start of import statement.
  pub start: u32,
  /// End of import statement.
  pub end: u32,
  /// Import source.
  pub module_request: ValueSpan,
  /// Import specifiers. Empty for `import "mod"`.
  pub entries: Vec<StaticImportEntry>,
}

#[napi(object)]
pub struct StaticImportEntry {
  /// The name under which the desired binding is exported by the module.
  pub import_name: ImportName,
  /// The local binding name.
  pub local_name: ValueSpan,
  /// Whether this binding is for a TypeScript type-only import.
  pub is_type: bool,
}

#[napi(object)]
pub struct StaticExport {
  pub start: u32,
  pub end: u32,
  pub entries: Vec<StaticExportEntry>,
}

#[napi(object, use_nullable = true)]
pub struct StaticExportEntry {
  pub start: u32,
  pub end: u32,
  /// Re-export source, if this export comes from another module.
  pub module_request: Option<ValueSpan>,
  /// The imported name for re-exports.
  pub import_name: ExportImportName,
  /// The exported name.
  pub export_name: ExportExportName,
  /// The local name used to access the exported value.
  pub local_name: ExportLocalName,
  /// Whether the export is a TypeScript `export type`.
  pub is_type: bool,
}

#[napi(object, use_nullable = true)]
pub struct ImportName {
  pub kind: ImportNameKind,
  pub name: Option<String>,
  pub start: Option<u32>,
  pub end: Option<u32>,
}

#[napi(string_enum)]
pub enum ImportNameKind {
  Name,
  NamespaceObject,
  Default,
}

#[napi(object, use_nullable = true)]
pub struct ExportImportName {
  pub kind: ExportImportNameKind,
  pub name: Option<String>,
  pub start: Option<u32>,
  pub end: Option<u32>,
}

#[napi(string_enum)]
pub enum ExportImportNameKind {
  Name,
  All,
  AllButDefault,
  None,
}

#[napi(object, use_nullable = true)]
pub struct ExportExportName {
  pub kind: ExportExportNameKind,
  pub name: Option<String>,
  pub start: Option<u32>,
  pub end: Option<u32>,
}

#[napi(string_enum)]
pub enum ExportExportNameKind {
  Name,
  Default,
  None,
}

#[napi(object, use_nullable = true)]
pub struct ExportLocalName {
  pub kind: ExportLocalNameKind,
  pub name: Option<String>,
  pub start: Option<u32>,
  pub end: Option<u32>,
}

#[napi(string_enum)]
pub enum ExportLocalNameKind {
  Name,
  Default,
  None,
}

#[napi(object)]
pub struct DynamicImport {
  pub start: u32,
  pub end: u32,
  pub module_request: Span,
}

fn get_source_type(filename: &str, lang: Option<&str>, source_type: Option<&str>) -> SourceType {
  let source_type_from_lang = match lang {
    Some("js") => SourceType::unambiguous(),
    Some("jsx") => SourceType::unambiguous().with_jsx(true),
    Some("ts") => SourceType::unambiguous().with_typescript(true),
    Some("tsx") => SourceType::unambiguous()
      .with_typescript(true)
      .with_jsx(true),
    Some("dts") => SourceType::d_ts(),
    Some("ets") => SourceType::ets(),
    _ => SourceType::from_path(filename).unwrap_or_default(),
  };

  match source_type {
    Some("script") => source_type_from_lang.with_script(true),
    Some("module") => source_type_from_lang.with_module(true),
    Some("commonjs") => source_type_from_lang.with_commonjs(true),
    Some("unambiguous") => source_type_from_lang.with_unambiguous(true),
    _ => source_type_from_lang,
  }
}

fn get_ast_type(source_type: SourceType, options: &ParserOptions) -> AstType {
  match options.ast_type.as_deref() {
    Some("js") => AstType::JavaScript,
    Some("ts") => AstType::TypeScript,
    _ if source_type.is_javascript() => AstType::JavaScript,
    _ => AstType::TypeScript,
  }
}

fn parse_with_return(filename: &str, source_text: &str, options: &ParserOptions) -> ParseResult {
  let allocator = Allocator::default();
  let source_type = get_source_type(
    filename,
    options.lang.as_deref(),
    options.source_type.as_deref(),
  );
  let ast_type = get_ast_type(source_type, options);
  let ranges = options.range.unwrap_or(false);
  let ret = Parser::new(&allocator, source_text, source_type)
    .with_options(ParseOptions {
      preserve_parens: options.preserve_parens.unwrap_or(true),
      ..ParseOptions::default()
    })
    .parse();

  let mut program = ret.program;
  let mut module_record = ret.module_record;
  let mut diagnostics = ret.errors;

  if options.show_semantic_errors == Some(true) {
    let semantic_ret = SemanticBuilder::new()
      .with_check_syntax_error(true)
      .build(&program);
    diagnostics.extend(semantic_ret.errors);
  }

  let mut errors = OxcError::from_diagnostics(filename, source_text, diagnostics);
  let mut comments =
    convert_utf8_to_utf16(source_text, &mut program, &mut module_record, &mut errors);

  let program_json = match ast_type {
    AstType::JavaScript => {
      if let Some(hashbang) = &program.hashbang {
        comments.insert(
          0,
          Comment {
            r#type: "Line".to_string(),
            value: hashbang.value.to_string(),
            start: hashbang.span.start,
            end: hashbang.span.end,
          },
        );
      }
      program.to_estree_js_json_with_fixes(ranges)
    }
    AstType::TypeScript => program.to_estree_ts_json_with_fixes(ranges),
  };
  let program = parse_program_json(program_json);

  let module = EcmaScriptModule::from(&module_record);

  ParseResult {
    program,
    module,
    comments,
    errors,
  }
}

fn parse_program_json(program_json: String) -> Value {
  match serde_json::from_str::<Value>(&program_json) {
    Ok(Value::Object(mut map)) => map.remove("node").unwrap_or(Value::Object(map)),
    Ok(value) => value,
    Err(_) => Value::Null,
  }
}

/// Parse JS/TS/ArkTS source text and return an ESTree-compatible AST.
#[cfg(not(target_family = "wasm"))]
#[napi]
pub async fn parse(
  filename: String,
  source_text: String,
  options: Option<ParserOptions>,
) -> ParseResult {
  let options = options.unwrap_or_default();
  parse_with_return(&filename, &source_text, &options)
}

/// Parse JS/TS/ArkTS source text and return an ESTree-compatible AST.
#[cfg(target_family = "wasm")]
#[napi]
pub async fn parse(
  filename: String,
  source_text: String,
  options: Option<ParserOptions>,
) -> ParseResult {
  let options = options.unwrap_or_default();
  parse_with_return(&filename, &source_text, &options)
}

impl From<&ModuleRecord<'_>> for EcmaScriptModule {
  fn from(record: &ModuleRecord<'_>) -> Self {
    let mut static_imports = record
      .requested_modules
      .iter()
      .flat_map(|(name, requested_modules)| {
        requested_modules.iter().filter(|m| m.is_import).map(|m| {
          let entries = record
            .import_entries
            .iter()
            .filter(|e| e.statement_span == m.statement_span)
            .map(StaticImportEntry::from)
            .collect::<Vec<_>>();
          StaticImport {
            start: m.statement_span.start,
            end: m.statement_span.end,
            module_request: ValueSpan {
              value: name.to_string(),
              start: m.span.start,
              end: m.span.end,
            },
            entries,
          }
        })
      })
      .collect::<Vec<_>>();
    static_imports.sort_unstable_by_key(|e| e.start);

    let mut static_exports = record
      .local_export_entries
      .iter()
      .chain(record.indirect_export_entries.iter())
      .chain(record.star_export_entries.iter())
      .map(|e| (e.statement_span, StaticExportEntry::from(e)))
      .fold(
        FxHashMap::<_, Vec<StaticExportEntry>>::default(),
        |mut acc, (span, entry)| {
          acc.entry(span).or_default().push(entry);
          acc
        },
      )
      .into_iter()
      .map(|(span, entries)| StaticExport {
        start: span.start,
        end: span.end,
        entries,
      })
      .collect::<Vec<_>>();
    static_exports.sort_unstable_by_key(|e| e.start);

    let dynamic_imports = record
      .dynamic_imports
      .iter()
      .map(|import| DynamicImport {
        start: import.span.start,
        end: import.span.end,
        module_request: Span::from(&import.module_request),
      })
      .collect::<Vec<_>>();

    let import_metas = record.import_metas.iter().map(Span::from).collect();

    Self {
      has_module_syntax: record.has_module_syntax,
      static_imports,
      static_exports,
      dynamic_imports,
      import_metas,
    }
  }
}

impl From<&OxcSpan> for Span {
  fn from(span: &OxcSpan) -> Self {
    Self {
      start: span.start,
      end: span.end,
    }
  }
}

impl From<&module_record::ExportEntry<'_>> for StaticExportEntry {
  fn from(entry: &module_record::ExportEntry) -> Self {
    Self {
      start: entry.span.start,
      end: entry.span.end,
      module_request: entry.module_request.as_ref().map(ValueSpan::from),
      import_name: ExportImportName::from(&entry.import_name),
      export_name: ExportExportName::from(&entry.export_name),
      local_name: ExportLocalName::from(&entry.local_name),
      is_type: entry.is_type,
    }
  }
}

impl From<&module_record::ImportEntry<'_>> for StaticImportEntry {
  fn from(entry: &module_record::ImportEntry<'_>) -> Self {
    Self {
      import_name: ImportName::from(&entry.import_name),
      local_name: ValueSpan::from(&entry.local_name),
      is_type: entry.is_type,
    }
  }
}

impl From<&module_record::ImportImportName<'_>> for ImportName {
  fn from(entry: &module_record::ImportImportName<'_>) -> Self {
    let (kind, name, start, end) = match entry {
      module_record::ImportImportName::Name(name_span) => (
        ImportNameKind::Name,
        Some(name_span.name.to_string()),
        Some(name_span.span.start),
        Some(name_span.span.end),
      ),
      module_record::ImportImportName::NamespaceObject => {
        (ImportNameKind::NamespaceObject, None, None, None)
      }
      module_record::ImportImportName::Default(span) => (
        ImportNameKind::Default,
        None,
        Some(span.start),
        Some(span.end),
      ),
    };
    Self {
      kind,
      name,
      start,
      end,
    }
  }
}

impl From<&module_record::NameSpan<'_>> for ValueSpan {
  fn from(name_span: &module_record::NameSpan) -> Self {
    Self {
      value: name_span.name.to_string(),
      start: name_span.span.start,
      end: name_span.span.end,
    }
  }
}

impl From<&module_record::ExportImportName<'_>> for ExportImportName {
  fn from(entry: &module_record::ExportImportName<'_>) -> Self {
    let (kind, name, start, end) = match entry {
      module_record::ExportImportName::Name(name_span) => (
        ExportImportNameKind::Name,
        Some(name_span.name.to_string()),
        Some(name_span.span.start),
        Some(name_span.span.end),
      ),
      module_record::ExportImportName::All => (ExportImportNameKind::All, None, None, None),
      module_record::ExportImportName::AllButDefault => {
        (ExportImportNameKind::AllButDefault, None, None, None)
      }
      module_record::ExportImportName::Null => (ExportImportNameKind::None, None, None, None),
    };
    Self {
      kind,
      name,
      start,
      end,
    }
  }
}

impl From<&module_record::ExportExportName<'_>> for ExportExportName {
  fn from(entry: &module_record::ExportExportName<'_>) -> Self {
    let (kind, name, start, end) = match entry {
      module_record::ExportExportName::Name(name_span) => (
        ExportExportNameKind::Name,
        Some(name_span.name.to_string()),
        Some(name_span.span.start),
        Some(name_span.span.end),
      ),
      module_record::ExportExportName::Default(span) => (
        ExportExportNameKind::Default,
        None,
        Some(span.start),
        Some(span.end),
      ),
      module_record::ExportExportName::Null => (ExportExportNameKind::None, None, None, None),
    };
    Self {
      kind,
      name,
      start,
      end,
    }
  }
}

impl From<&module_record::ExportLocalName<'_>> for ExportLocalName {
  fn from(entry: &module_record::ExportLocalName<'_>) -> Self {
    let (kind, name, start, end) = match entry {
      module_record::ExportLocalName::Name(name_span) => (
        ExportLocalNameKind::Name,
        Some(name_span.name.to_string()),
        Some(name_span.span.start),
        Some(name_span.span.end),
      ),
      module_record::ExportLocalName::Default(name_span) => (
        ExportLocalNameKind::Default,
        Some(name_span.name.to_string()),
        Some(name_span.span.start),
        Some(name_span.span.end),
      ),
      module_record::ExportLocalName::Null => (ExportLocalNameKind::None, None, None, None),
    };
    Self {
      kind,
      name,
      start,
      end,
    }
  }
}
