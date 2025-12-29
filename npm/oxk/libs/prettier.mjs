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
export async function resolvePlugins() {
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
export async function formatEmbeddedCode({ code, tagName, options }) {
  // TODO: This should be resolved in Rust side
  const parserName = TAG_TO_PARSER[tagName]

  // Unknown tag, return original code
  if (!parserName) return code

  if (!prettierCache) {
    prettierCache = await import('prettier')
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  options.parser = parserName
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
export async function formatFile({ code, parserName, fileName, options }) {
  if (!prettierCache) {
    prettierCache = await import('prettier')
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  // We specify `parser` to skip parser inference for performance
  options.parser = parserName
  // But some plugins rely on `filepath`, so we set it too
  options.filepath = fileName
  return prettierCache.format(code, options)
}
