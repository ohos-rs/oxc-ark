const { existsSync, readFileSync, writeFileSync } = require('node:fs')
const path = require('node:path')
const { glob } = require('glob')

const { format } = require('../index.js')

const CONFIG_FILES = ['.oxfmtrc.json', '.oxfmtrc.jsonc']
const RECOVERABLE_ERROR_PATTERNS = [
  'Unsupported file type',
  'requires external formatter support',
  'External formatter is required',
]

function stripJsonComments(text) {
  let result = ''
  let inString = false
  let stringQuote = ''
  let isEscaped = false
  let inLineComment = false
  let inBlockComment = false

  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]
    const next = text[index + 1]

    if (inLineComment) {
      if (char === '\n') {
        inLineComment = false
        result += char
      }
      continue
    }

    if (inBlockComment) {
      if (char === '*' && next === '/') {
        inBlockComment = false
        index += 1
      }
      continue
    }

    if (inString) {
      result += char
      if (isEscaped) {
        isEscaped = false
      } else if (char === '\\') {
        isEscaped = true
      } else if (char === stringQuote) {
        inString = false
        stringQuote = ''
      }
      continue
    }

    if (char === '"' || char === "'") {
      inString = true
      stringQuote = char
      result += char
      continue
    }

    if (char === '/' && next === '/') {
      inLineComment = true
      index += 1
      continue
    }

    if (char === '/' && next === '*') {
      inBlockComment = true
      index += 1
      continue
    }

    result += char
  }

  return result
}

function resolveConfigPath(cwd, explicitPath) {
  if (explicitPath) {
    const resolved = path.resolve(cwd, explicitPath)
    if (!existsSync(resolved)) {
      throw new Error(`Failed to read ${resolved}: File not found`)
    }
    return resolved
  }

  let current = cwd
  for (;;) {
    for (const filename of CONFIG_FILES) {
      const candidate = path.join(current, filename)
      if (existsSync(candidate)) {
        return candidate
      }
    }

    const parent = path.dirname(current)
    if (parent === current) {
      return undefined
    }
    current = parent
  }
}

function loadConfig(cwd, explicitPath) {
  const configPath = resolveConfigPath(cwd, explicitPath)
  if (!configPath) {
    return { config: {}, ignorePatterns: [] }
  }

  const raw = readFileSync(configPath, 'utf8')
  const parsed = JSON.parse(stripJsonComments(raw))
  const ignorePatterns = Array.isArray(parsed.ignorePatterns) ? parsed.ignorePatterns : []
  return { config: parsed, ignorePatterns }
}

function mergeConfig(baseConfig, cliOptions) {
  return { ...baseConfig, ...cliOptions }
}

async function expandPatterns(patterns, cwd) {
  const results = new Set()

  for (const pattern of patterns) {
    const absolutePattern = path.resolve(cwd, pattern)
    const matches = await glob(absolutePattern, {
      absolute: true,
      dot: true,
      nodir: true,
      follow: false,
    })

    for (const match of matches) {
      results.add(path.resolve(match))
    }
  }

  return results
}

function isRecoverableError(errors) {
  return errors.every((error) => RECOVERABLE_ERROR_PATTERNS.some((pattern) => error.includes(pattern)))
}

async function runWithConcurrency(items, limit, worker) {
  let index = 0
  const workers = Array.from({ length: Math.max(1, Math.min(limit, items.length || 1)) }, async () => {
    for (;;) {
      const current = index
      index += 1
      if (current >= items.length) {
        return
      }
      await worker(items[current])
    }
  })

  await Promise.all(workers)
}

function printErrors(filePath, errors, prefix) {
  console.error(`${prefix} ${filePath}:`)
  for (const error of errors) {
    console.error(`  ${error}`)
  }
}

async function formatFiles({ patterns, configPath, excludes, threadCount, cliOptions }) {
  const cwd = process.cwd()

  if (patterns.length === 0) {
    throw new Error('Missing file pattern')
  }

  const { config, ignorePatterns } = loadConfig(cwd, configPath)
  const mergedConfig = mergeConfig(config, cliOptions)
  const allExcludes = excludes.concat(ignorePatterns)

  const files = await expandPatterns(patterns, cwd)
  if (allExcludes.length > 0) {
    const excludedFiles = await expandPatterns(allExcludes, cwd)
    for (const excluded of excludedFiles) {
      files.delete(excluded)
    }
  }

  const sortedFiles = Array.from(files).sort()
  if (sortedFiles.length === 0) {
    throw new Error('No files matched the provided patterns (after excludes)')
  }

  let formattedCount = 0
  let hasRecoverableErrors = false
  let fatalError = null

  await runWithConcurrency(sortedFiles, threadCount, async (filePath) => {
    if (fatalError) {
      return
    }

    const sourceText = readFileSync(filePath, 'utf8')
    if (sourceText.length === 0) {
      return
    }

    const displayPath = path.relative(cwd, filePath) || filePath
    const result = await format(displayPath, sourceText, mergedConfig)

    if (result.errors.length === 0) {
      writeFileSync(filePath, result.code, 'utf8')
      console.log(`Formatted: ${displayPath}`)
      formattedCount += 1
      return
    }

    if (isRecoverableError(result.errors)) {
      printErrors(displayPath, result.errors, 'Warning formatting')
      hasRecoverableErrors = true
      return
    }

    fatalError = { filePath: displayPath, errors: result.errors }
  })

  if (fatalError) {
    printErrors(fatalError.filePath, fatalError.errors, 'Error formatting')
    process.exitCode = 1
    return
  }

  console.log(`\nFormatted ${formattedCount} file(s)`)
  if (hasRecoverableErrors) {
    process.exitCode = 1
  }
}

module.exports = { formatFiles }
