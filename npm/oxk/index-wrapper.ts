import { format as napiFormat } from './index.js'
import { resolvePlugins, formatEmbeddedCode, formatFile } from './libs/prettier.js'

/**
 * Format the given source text according to the specified options.
 *
 * This is a convenience wrapper that automatically uses Prettier for external formatter callbacks.
 * For more control, use the raw `format` function from `./index.js` directly.
 *
 * @param fileName - The name of the file to format
 * @param sourceText - The source code to format
 * @param options - Optional formatting options (compatible with Prettier options)
 * @returns A promise that resolves to the formatted code and any errors
 */
export async function format(
  fileName: string,
  sourceText: string,
  options?: Record<string, any>,
): Promise<{ code: string; errors: string[] }> {
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

// Re-export the raw format function for advanced usage
export { format as formatRaw } from './index.js'
export type { FormatResult } from './index.d.ts'
