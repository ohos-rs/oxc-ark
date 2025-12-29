// Convenience wrapper that automatically uses Prettier for external formatter callbacks
const { format: napiFormat } = require("./index.js");

// Lazy load Prettier
let prettierCache;

/**
 * TODO: Plugins support
 * - Read `plugins` field
 * - Load plugins dynamically and parse `languages` field
 * - Map file extensions and filenames to Prettier parsers
 *
 * @returns {Promise<string[]>} Array of loaded plugin's `languages` info
 */
async function resolvePlugins() {
  return [];
}

// ---

const TAG_TO_PARSER = {
  // CSS
  css: "css",
  styled: "css",
  // GraphQL
  gql: "graphql",
  graphql: "graphql",
  // HTML
  html: "html",
  // Markdown
  md: "markdown",
  markdown: "markdown",
};

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
  // TODO: This should be resolved in Rust side
  const parserName = TAG_TO_PARSER[tagName];

  // Unknown tag, return original code
  if (!parserName) return code;

  if (!prettierCache) {
    prettierCache = await import("prettier");
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  options.parser = parserName;
  return prettierCache
    .format(code, options)
    .then((formatted) => formatted.trimEnd())
    .catch(() => code);
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
async function formatFile({ code, parserName, fileName, options }) {
  if (!prettierCache) {
    prettierCache = await import("prettier");
  }

  // SAFETY: `options` is created in Rust side, so it's safe to mutate here
  // We specify `parser` to skip parser inference for performance
  options.parser = parserName;
  // But some plugins rely on `filepath`, so we set it too
  options.filepath = fileName;
  return prettierCache.format(code, options);
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
  if (typeof fileName !== "string")
    throw new TypeError("`fileName` must be a string");
  if (typeof sourceText !== "string")
    throw new TypeError("`sourceText` must be a string");

  return napiFormat(
    fileName,
    sourceText,
    options ?? {},
    resolvePlugins,
    (options, tagName, code) => formatEmbeddedCode({ options, tagName, code }),
    (options, parserName, fileName, code) =>
      formatFile({ options, parserName, fileName, code }),
  );
}

// Re-export the raw format function for advanced usage
function formatRaw(
  fileName,
  sourceText,
  options,
  initExternalFormatterCb,
  formatEmbeddedCb,
  formatFileCb,
) {
  const { format } = require("./index.js");
  return format(
    fileName,
    sourceText,
    options,
    initExternalFormatterCb,
    formatEmbeddedCb,
    formatFileCb,
  );
}

module.exports = { format, formatRaw };

