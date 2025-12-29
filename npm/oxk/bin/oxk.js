#!/usr/bin/env node

const VERSION = require('../package.json').version

// Import command modules
const { formatFiles } = require('./format.js')

function showHelp() {
  console.log(`
oxk - An ArkTS/ArkUI tool based on OXC

Usage:
  oxk <command> [options] [files...]

Commands:
  format    Format files
  lint      Lint files (not yet supported)

Options:
  --help, -h     Show help
  --version, -v  Show version

Examples:
  oxk format src/**/*.ets
`)
}

function showVersion() {
  console.log(`oxk v${VERSION}`)
}

// Main CLI handler
async function main() {
  const args = process.argv.slice(2)

  if (args.length === 0) {
    showHelp()
    process.exit(0)
  }

  const command = args[0]

  // Handle global options
  if (command === '--help' || command === '-h') {
    showHelp()
    process.exit(0)
  }

  if (command === '--version' || command === '-v') {
    showVersion()
    process.exit(0)
  }

  // Handle commands by calling functions directly
  const remainingArgs = args.slice(1)

  try {
    switch (command) {
      case 'format':
        await formatFiles(remainingArgs)
        break

      default:
        console.error(`Unknown command: ${command}`)
        console.error("Run 'oxk --help' for usage information")
        process.exit(1)
    }
  } catch (error) {
    console.error('Unexpected error:', error)
    process.exit(1)
  }
}

// Run the CLI
main()
