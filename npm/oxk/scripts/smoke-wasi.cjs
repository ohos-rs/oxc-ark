#!/usr/bin/env node

process.env.NAPI_RS_FORCE_WASI ||= '1'

const { readFileSync } = require('node:fs')
const { join } = require('node:path')
const oxk = require('../index.js')
const { lint } = require('../lint.js')

function assertUniqueBrowserExports() {
  const source = readFileSync(join(__dirname, '..', 'oxk.wasi-browser.js'), 'utf8')
  const names = [...source.matchAll(/\bexport const\s+([A-Za-z_$][\w$]*)/g)].map((match) => match[1])
  const duplicate = names.find((name, index) => names.indexOf(name) !== index)
  if (duplicate) {
    throw new Error(`WASI browser binding has duplicate named export: ${duplicate}`)
  }
}

async function expectUnsupported(label, callback) {
  try {
    await callback()
  } catch (error) {
    const message = error && error.message ? error.message : String(error)
    if (message.includes('not supported')) return
    throw new Error(`${label} failed with an unexpected error: ${message}`)
  }

  throw new Error(`${label} should not be supported in WASI builds`)
}

async function main() {
  assertUniqueBrowserExports()

  const parsed = await oxk.parse('input.ts', 'const value: number = 1', { lang: 'ts' })
  if (parsed.errors.length > 0) {
    throw new Error(`WASI parse returned errors: ${parsed.errors.map((error) => error.message).join(', ')}`)
  }
  if (parsed.program.type !== 'Program') {
    throw new Error(`WASI parse returned unexpected program type: ${parsed.program.type}`)
  }

  const formatted = await oxk.format(
    'input.ts',
    'const value=1',
    {},
    async () => [],
    async (_options, _tagName, code) => code,
    async (_options, _parserName, _fileName, code) => code,
  )
  if (formatted.errors.length > 0) {
    throw new Error(`WASI format returned errors: ${formatted.errors.join(', ')}`)
  }
  if (!formatted.code.includes('const value')) {
    throw new Error('WASI format output did not contain the expected source text')
  }

  for (const [filename, source, expected] of [
    ['style.css', 'a{color:red}', 'color: red;'],
    ['README.md', '# Demo\n\n```ts\nconst value={a:1}\n```', 'const value = { a: 1 };'],
    ['package.json', '{"version":"1.0.0","name":"example"}', '"name": "example",\n  "version"'],
  ]) {
    const result = await oxk.format(filename, source, {})
    if (result.errors.length || !result.code.includes(expected)) {
      throw new Error(`WASI native formatting failed for ${filename}: ${JSON.stringify(result)}`)
    }
  }

  await expectUnsupported('root lint', () => oxk.lint([]))
  await expectUnsupported('lint wrapper', () => lint([]))

  console.log('WASI smoke ok')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
