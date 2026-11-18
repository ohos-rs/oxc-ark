// Convenience wrapper that automatically uses Prettier for external formatter callbacks
const {
  format: napiFormat,
  formatFiles: napiFormatFiles,
  formatLsp,
  jsTextToDoc: napiJsTextToDoc,
} = require('./index.js')

// Lazy load Prettier
let prettierCache

/**
 * TODO: Plugins support
 * - Read `plugins` field
 * - Load plugins dynamically and parse `languages` field
 * - Map file extensions and filenames to Prettier parsers
 *
 * @returns {Promise<string[]>} Array of loaded plugin's `languages` info
 */
async function resolvePlugins() {
  return []
}

// ---

const TAG_TO_PARSER = {
  // CSS
  css: 'css',
  styled: 'css',
  // GraphQL
  gql: 'graphql',
  graphql: 'graphql',
  // HTML
  html: 'html',
  // Markdown
  md: 'markdown',
  markdown: 'markdown',
}

/**
 * Format xxx-in-js code snippets
 *
 * @param {Object} param
 * @param {string} param.code
 * @param {string} param.tagName
 * @param {any} param.options
 * @returns {Promise<string>} Formatted code snippet
 */
async function formatEmbeddedCode({ code, tagName, options }) {
  if (tagName === '__oxkTailwind') {
    const { createSorter } = await import('prettier-plugin-tailwindcss/sorter')
    const sorter = await createSorter({
      filepath: options.filepath,
      stylesheetPath: options.tailwindStylesheet,
      configPath: options.tailwindConfig,
      preserveWhitespace: options.tailwindPreserveWhitespace,
      preserveDuplicates: options.tailwindPreserveDuplicates,
    })
    return JSON.stringify(sorter.sortClassAttributes(JSON.parse(code)))
  }
  await setupPlugins(options)

  // TODO: This should be resolved in Rust side
  const parserName = TAG_TO_PARSER[tagName] ?? tagName

  // Unknown tag, return original code
  if (!parserName) return code

  if (!prettierCache) {
    prettierCache = await import('prettier')
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  options.parser = parserName
  if (options.__oxkDoc) {
    delete options.__oxkDoc
    const metadata = {}
    prettierCache.__internal.formatOptionsHiddenDefaults.parentParser = null
    prettierCache.__internal.formatOptionsHiddenDefaults.__onHtmlRoot = null
    options.parentParser = 'OXFMT'
    if (parserName === 'html' || parserName === 'angular') {
      options.__onHtmlRoot = (root) => {
        metadata.htmlHasMultipleRootElements = (root.children?.length ?? 0) > 1
      }
    }
    const doc = await prettierCache.__debug.printToDoc(code, options)
    const symbols = new Map()
    return JSON.stringify([doc, metadata], (_key, value) => {
      if (typeof value === 'symbol') {
        if (!symbols.has(value)) symbols.set(value, symbols.size + 1)
        return symbols.get(value)
      }
      return value === -Infinity ? '__NEGATIVE_INFINITY__' : value
    })
  }
  return prettierCache
    .format(code, options)
    .then((formatted) => formatted.trimEnd())
    .catch(() => code)
}

/**
 * Format non-js file
 *
 * @param {Object} param
 * @param {string} param.code
 * @param {string} param.parserName
 * @param {string} param.fileName
 * @param {any} param.options
 * @returns {Promise<string>} Formatted code
 */
async function setupPlugins(options) {
  options.plugins ??= []
  if (options._useSveltePlugin) options.plugins.push(await import('prettier-plugin-svelte'))
  if (options._useAstroPlugin) options.plugins.push(await import('prettier-plugin-astro'))
  if (options._useTailwindPlugin) options.plugins.push(await import('prettier-plugin-tailwindcss'))
  if (options._oxfmtPluginOptionsJson) options.plugins.push(require('./prettier-plugin-oxk/index.js'))
}

async function formatFile({ code, parserName, fileName, options }) {
  await setupPlugins(options)
  if (!prettierCache) {
    prettierCache = await import('prettier')
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  // We specify `parser` to skip parser inference for performance
  options.parser = parserName
  if (parserName === 'vue') {
    const tags = code.matchAll(/<script(?=[\s>])((?:"[^"]*"|'[^']*'|[^"'>])*)>/g)
    if ([...tags].some((tag) => /(?:^|\s)lang\s*=\s*(?:"tsx"|'tsx'|tsx(?=[\s/]|$))/.test(tag[1]))) {
      options._oxfmtVueScriptLang = 'tsx'
    }
  }
  // But some plugins rely on `filepath`, so we set it too
  options.filepath = fileName
  return prettierCache.format(code, options)
}

/**
 * Format the given source text according to the specified options.
 *
 * This is a convenience wrapper that automatically uses Prettier for external formatter callbacks.
 * For more control, use the raw `format` function from `./index.js` directly.
 *
 * @param {string} fileName - The name of the file to format
 * @param {string} sourceText - The source code to format
 * @param {Record<string, any>} [options] - Optional formatting options (compatible with Prettier options)
 * @returns {Promise<{code: string, errors: string[]}>} A promise that resolves to the formatted code and any errors
 */
async function format(fileName, sourceText, options) {
  if (typeof fileName !== 'string') throw new TypeError('`fileName` must be a string')
  if (typeof sourceText !== 'string') throw new TypeError('`sourceText` must be a string')

  return napiFormat(
    fileName,
    sourceText,
    options ?? {},
    resolvePlugins,
    (options, tagName, code) => formatEmbeddedCode({ options, tagName, code }),
    (options, parserName, fileName, code) => formatFile({ options, parserName, fileName, code }),
  )
}

async function formatFiles(args) {
  return napiFormatFiles(
    {
      patterns: args.patterns,
      excludes: args.excludes,
      ignorePaths: args.ignorePaths,
      withNodeModules: args.withNodeModules,
      threadCount: args.threadCount,
      configPath: args.configPath,
      lang: args.lang,
      cliOptions: args.cliOptions,
    },
    resolvePlugins,
    (options, tagName, code) => formatEmbeddedCode({ options, tagName, code }),
    (options, parserName, fileName, code) => formatFile({ options, parserName, fileName, code }),
  )
}

// Re-export the raw format function for advanced usage
function formatRaw(fileName, sourceText, options, initExternalFormatterCb, formatEmbeddedCb, formatFileCb) {
  const { format } = require('./index.js')
  return format(fileName, sourceText, options, initExternalFormatterCb, formatEmbeddedCb, formatFileCb)
}

// Internal Prettier plugin entry point. Keep the same callback transport as format().
function __jsTextToDoc(sourceExt, sourceText, optionsJson, parentContext) {
  return napiJsTextToDoc(
    sourceExt,
    sourceText,
    optionsJson,
    parentContext,
    resolvePlugins,
    (options, tagName, code) => formatEmbeddedCode({ options, tagName, code }),
    (options, parserName, fileName, code) => formatFile({ options, parserName, fileName, code }),
  )
}

module.exports = { format, formatFiles, formatRaw, formatLsp, __jsTextToDoc }
