#!/usr/bin/env node

const VERSION = require('../package.json').version
const { formatFiles } = require('./format.js')

function parseBoolean(value, flagName) {
  if (value === 'true') {
    return true
  }
  if (value === 'false') {
    return false
  }
  throw new Error(`${flagName} must be 'true' or 'false'`)
}

function parseInteger(value, flagName) {
  const parsed = Number.parseInt(value, 10)
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${flagName} must be a positive integer`)
  }
  return parsed
}

function parseOptionValue(args, index, token) {
  if (token.includes('=')) {
    const [, inlineValue] = token.split(/=(.*)/s)
    return { value: inlineValue, nextIndex: index }
  }

  const value = args[index + 1]
  if (value == null) {
    throw new Error(`Missing value for ${token}`)
  }
  return { value, nextIndex: index + 1 }
}

function showHelp() {
  console.log(`
oxk - An ArkTS/ArkUI tool based on OXC

Usage:
  oxk <command> [options] [files...]

Commands:
  format    Format files

Global Options:
  --help, -h     Show help
  --version, -v  Show version

Format Options:
  --config PATH
  --thread, -t THREAD
  --exclude PATTERN
  --indent-style STYLE
  --indent-width WIDTH
  --line-ending ENDING
  --line-width WIDTH
  --quote-style STYLE
  --jsx-quote-style STYLE
  --trailing-commas VALUE
  --semicolons VALUE
  --arrow-parentheses VALUE
  --bracket-spacing VALUE
  --bracket-same-line VALUE
  --attribute-position VALUE
  --expand VALUE
  --experimental-operator-position VALUE
  --experimental-ternaries VALUE
  --embedded-language-formatting VALUE

Examples:
  oxk format src/**/*.ets
  oxk format --exclude dist/** --config .oxfmtrc.json "src/**/*.{ts,ets}"
`)
}

function showVersion() {
  console.log(`oxk v${VERSION}`)
}

function parseFormatArgs(args) {
  const formatArgs = {
    patterns: [],
    excludes: [],
    threadCount: 1,
    configPath: undefined,
    cliOptions: {},
  }

  for (let index = 0; index < args.length; index += 1) {
    const token = args[index]

    if (token === '--help' || token === '-h') {
      return { help: true }
    }

    if (!token.startsWith('-')) {
      formatArgs.patterns.push(token)
      continue
    }

    switch (token.split('=')[0]) {
      case '--config': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.configPath = value
        index = nextIndex
        break
      }
      case '--thread':
      case '-t': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.threadCount = parseInteger(value, '--thread')
        index = nextIndex
        break
      }
      case '--exclude': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.excludes.push(value)
        index = nextIndex
        break
      }
      case '--indent-style': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'tab' && value !== 'space') {
          throw new Error("--indent-style must be 'tab' or 'space'")
        }
        formatArgs.cliOptions.useTabs = value === 'tab'
        index = nextIndex
        break
      }
      case '--indent-width': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.tabWidth = parseInteger(value, '--indent-width')
        index = nextIndex
        break
      }
      case '--line-ending': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.endOfLine = value
        index = nextIndex
        break
      }
      case '--line-width': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.printWidth = parseInteger(value, '--line-width')
        index = nextIndex
        break
      }
      case '--quote-style': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'single' && value !== 'double') {
          throw new Error("--quote-style must be 'single' or 'double'")
        }
        formatArgs.cliOptions.singleQuote = value === 'single'
        index = nextIndex
        break
      }
      case '--jsx-quote-style': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'single' && value !== 'double') {
          throw new Error("--jsx-quote-style must be 'single' or 'double'")
        }
        formatArgs.cliOptions.jsxSingleQuote = value === 'single'
        index = nextIndex
        break
      }
      case '--trailing-commas': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.trailingComma = value
        index = nextIndex
        break
      }
      case '--semicolons': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'always' && value !== 'as-needed') {
          throw new Error("--semicolons must be 'always' or 'as-needed'")
        }
        formatArgs.cliOptions.semi = value === 'always'
        index = nextIndex
        break
      }
      case '--arrow-parentheses': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'always' && value !== 'as-needed') {
          throw new Error("--arrow-parentheses must be 'always' or 'as-needed'")
        }
        formatArgs.cliOptions.arrowParens = value === 'always' ? 'always' : 'avoid'
        index = nextIndex
        break
      }
      case '--bracket-spacing': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.bracketSpacing = parseBoolean(value, '--bracket-spacing')
        index = nextIndex
        break
      }
      case '--bracket-same-line': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.bracketSameLine = parseBoolean(value, '--bracket-same-line')
        index = nextIndex
        break
      }
      case '--attribute-position': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (value !== 'auto' && value !== 'multiline') {
          throw new Error("--attribute-position must be 'auto' or 'multiline'")
        }
        formatArgs.cliOptions.singleAttributePerLine = value === 'multiline'
        index = nextIndex
        break
      }
      case '--expand': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        if (!['auto', 'always', 'never'].includes(value)) {
          throw new Error("--expand must be 'auto', 'always', or 'never'")
        }
        formatArgs.cliOptions.objectWrap = value === 'auto' ? 'preserve' : 'collapse'
        index = nextIndex
        break
      }
      case '--experimental-operator-position': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.experimentalOperatorPosition = value
        index = nextIndex
        break
      }
      case '--experimental-ternaries': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.experimentalTernaries = parseBoolean(value, '--experimental-ternaries')
        index = nextIndex
        break
      }
      case '--embedded-language-formatting': {
        const { value, nextIndex } = parseOptionValue(args, index, token)
        formatArgs.cliOptions.embeddedLanguageFormatting = value
        index = nextIndex
        break
      }
      case '--experimental-sort-imports': {
        const { nextIndex } = parseOptionValue(args, index, token)
        index = nextIndex
        break
      }
      default:
        throw new Error(`Unknown option: ${token}`)
    }
  }

  return { help: false, formatArgs }
}

async function main() {
  const args = process.argv.slice(2)

  if (args.length === 0) {
    showHelp()
    process.exit(0)
  }

  const command = args[0]
  if (command === '--help' || command === '-h') {
    showHelp()
    process.exit(0)
  }
  if (command === '--version' || command === '-v') {
    showVersion()
    process.exit(0)
  }

  if (command !== 'format') {
    console.error(`Unknown command: ${command}`)
    console.error("Run 'oxk --help' for usage information")
    process.exit(1)
  }

  try {
    const { help, formatArgs } = parseFormatArgs(args.slice(1))
    if (help) {
      showHelp()
      process.exit(0)
    }

    await formatFiles(formatArgs)
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
}

main()
