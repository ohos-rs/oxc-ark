// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.
'use strict'
/**
 * Prettier plugin that uses `oxc_formatter` for (j|t)s-in-xxx part.
 *
 * When Prettier formats Vue/HTML (which can embed JS/TS code inside) files,
 * it calls the `embed()` function for each block.
 *
 * By default, it uses the `babel` or `typescript` parser and `estree` printer.
 * Therefore, by overriding these internally, we can use `oxc_formatter` instead.
 * e.g. Now it's possible to apply our builtin sort-imports for JS/TS code inside Vue `<script>`.
 */
Object.defineProperty(exports, '__esModule', { value: true })
exports.printers = exports.parsers = exports.options = void 0
const text_to_doc_1 = require('./text-to-doc')
// NOTE: Custom options must be declared here,
// or Prettier's normalization drops them before they reach `textToDoc()`.
exports.options = {
  _oxfmtPluginOptionsJson: {
    category: 'JavaScript',
    type: 'string',
    default: '{}',
    description: 'Bundled JSON string for oxfmt-plugin options',
  },
  _oxfmtVueScriptLang: {
    category: 'JavaScript',
    type: 'string',
    default: '',
    description:
      'Effective `lang` of Vue SFC `<script>` blocks when it needs disambiguation (currently only "tsx"), scanned by the host',
  },
}
const oxfmtParser = {
  parse: text_to_doc_1.textToDoc,
  astFormat: 'OXFMT',
  // Not used but required
  locStart: () => -1,
  locEnd: () => -1,
}
exports.parsers = {
  // Override default JS/TS parsers
  babel: oxfmtParser,
  'babel-ts': oxfmtParser,
  typescript: oxfmtParser,
}
exports.printers = {
  OXFMT: {
    print: ({ node }) => node,
  },
}
