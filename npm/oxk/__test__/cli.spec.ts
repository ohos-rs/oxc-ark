import test from 'ava'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const cliPath = fileURLToPath(new URL('../bin/oxk.js', import.meta.url))

function withTempDir(run: (dir: string) => void) {
  const dir = mkdtempSync(join(tmpdir(), 'oxk-cli-'))
  try {
    run(dir)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

test('npm cli preserves quoted object properties by default', (t) => {
  withTempDir((dir) => {
    const filePath = join(dir, 'input.ts')
    writeFileSync(filePath, 'const value={"quoted":1,plain:2}\n', 'utf8')

    const result = spawnSync(process.execPath, [cliPath, 'format', 'input.ts'], {
      cwd: dir,
      encoding: 'utf8',
    })

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes('"quoted": 1'), 'Should preserve explicit quotes')
    t.true(formatted.includes('plain: 2'), 'Should keep unquoted properties unchanged')
  })
})

test('npm cli loads nearest .oxfmtrc.json', (t) => {
  withTempDir((dir) => {
    writeFileSync(join(dir, '.oxfmtrc.json'), JSON.stringify({ singleQuote: true }), 'utf8')
    const filePath = join(dir, 'config.ts')
    writeFileSync(filePath, 'const message="hello"\n', 'utf8')

    const result = spawnSync(process.execPath, [cliPath, 'format', 'config.ts'], {
      cwd: dir,
      encoding: 'utf8',
    })

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes("const message = 'hello'"), 'Should honor config discovery')
  })
})

test('npm cli loads nearest .oxfmtrc.jsonc', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxfmtrc.jsonc'),
      '{\n  // JSONC config should be accepted\n  "singleQuote": true\n}\n',
      'utf8',
    )
    const filePath = join(dir, 'config.ts')
    writeFileSync(filePath, 'const message="hello"\n', 'utf8')

    const result = spawnSync(process.execPath, [cliPath, 'format', 'config.ts'], {
      cwd: dir,
      encoding: 'utf8',
    })

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes("const message = 'hello'"), 'Should honor JSONC config discovery')
  })
})

test('npm cli prefers .oxfmtrc.json over .oxfmtrc.jsonc', (t) => {
  withTempDir((dir) => {
    writeFileSync(join(dir, '.oxfmtrc.json'), JSON.stringify({ singleQuote: true }), 'utf8')
    writeFileSync(join(dir, '.oxfmtrc.jsonc'), '{\n  "singleQuote": false\n}\n', 'utf8')
    const filePath = join(dir, 'config.ts')
    writeFileSync(filePath, 'const message="hello"\n', 'utf8')

    const result = spawnSync(process.execPath, [cliPath, 'format', 'config.ts'], {
      cwd: dir,
      encoding: 'utf8',
    })

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes("const message = 'hello'"), 'Should prefer .oxfmtrc.json')
  })
})
