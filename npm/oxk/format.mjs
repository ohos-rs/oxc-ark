// Convenience wrapper that automatically uses Prettier for external formatter callbacks
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const { format: napiFormat } = require("./index.js");
import { resolvePlugins, formatEmbeddedCode, formatFile } from "./libs/prettier.js";

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
export async function format(fileName, sourceText, options) {
  if (typeof fileName !== "string") throw new TypeError("`fileName` must be a string");
  if (typeof sourceText !== "string") throw new TypeError("`sourceText` must be a string");

  return napiFormat(
    fileName,
    sourceText,
    options ?? {},
    resolvePlugins,
    (options, tagName, code) => formatEmbeddedCode({ options, tagName, code }),
    (options, parserName, fileName, code) => formatFile({ options, parserName, fileName, code }),
  );
}

// Re-export the raw format function for advanced usage
export function formatRaw(fileName, sourceText, options, initExternalFormatterCb, formatEmbeddedCb, formatFileCb) {
  const { format } = require("./index.js");
  return format(fileName, sourceText, options, initExternalFormatterCb, formatEmbeddedCb, formatFileCb);
}

