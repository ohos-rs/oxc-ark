import test from 'ava'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'

const cliPath = fileURLToPath(new URL('../bin/oxk.js', import.meta.url))
const schemaPath = fileURLToPath(new URL('../configuration_schema.json', import.meta.url))

function canImportTypeScriptConfig() {
  const dir = mkdtempSync(join(tmpdir(), 'oxk-ts-config-'))
  try {
    const configPath = join(dir, 'oxlint.config.ts')
    writeFileSync(configPath, 'export default {}\n', 'utf8')
    const result = spawnSync(
      process.execPath,
      ['-e', 'import(process.argv[1]).then(() => {}, () => process.exit(1))', pathToFileURL(configPath).href],
      { encoding: 'utf8' },
    )
    return result.status === 0
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

const testTsConfig = canImportTypeScriptConfig() ? test : test.skip

function runCli(args: string[], cwd: string) {
  return spawnSync(process.execPath, [cliPath, ...args], {
    cwd,
    encoding: 'utf8',
  })
}

function normalizeLintOutput(value: unknown, dir: string): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => normalizeLintOutput(item, dir))
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        key === 'start_time' ? '<duration>' : normalizeLintOutput(item, dir),
      ]),
    )
  }
  if (typeof value === 'string') {
    return value.replaceAll(dir, '<tmp>').replaceAll('\\', '/')
  }
  return value
}

function lintJsonSnapshot(result: ReturnType<typeof runCli>, dir: string) {
  const output = `${result.stdout}\n${result.stderr}`.trim()
  return {
    status: result.status,
    output: normalizeLintOutput(JSON.parse(output), dir),
  }
}

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

    const result = runCli(['format', 'input.ts'], dir)

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

    const result = runCli(['format', 'config.ts'], dir)

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

    const result = runCli(['format', 'config.ts'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes("const message = 'hello'"), 'Should honor JSONC config discovery')
  })
})

test('npm cli formats external files with bundled Prettier callbacks', (t) => {
  withTempDir((dir) => {
    const filePath = join(dir, 'config.yaml')
    writeFileSync(filePath, 'name: test\nitems:\n- one\n- two\n', 'utf8')

    const result = runCli(['format', 'config.yaml'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes('items:\n  - one\n  - two'), 'Should format YAML via Prettier')
  })
})

test('npm cli formats JSON5 through native formatter', (t) => {
  withTempDir((dir) => {
    const filePath = join(dir, 'config.json5')
    writeFileSync(filePath, "{\n// keep comment\nname:'native-json5'\n}\n", 'utf8')

    const result = runCli(['format', 'config.json5'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes('// keep comment'), 'Should preserve JSON5 comments')
    t.true(formatted.includes("name: 'native-json5'"), 'Should format JSON5 content')
  })
})

test('npm cli prefers .oxfmtrc.json over .oxfmtrc.jsonc', (t) => {
  withTempDir((dir) => {
    writeFileSync(join(dir, '.oxfmtrc.json'), JSON.stringify({ singleQuote: true }), 'utf8')
    writeFileSync(join(dir, '.oxfmtrc.jsonc'), '{\n  "singleQuote": false\n}\n', 'utf8')
    const filePath = join(dir, 'config.ts')
    writeFileSync(filePath, 'const message="hello"\n', 'utf8')

    const result = runCli(['format', 'config.ts'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)

    const formatted = readFileSync(filePath, 'utf8')
    t.true(formatted.includes("const message = 'hello'"), 'Should prefer .oxfmtrc.json')
  })
})

test('npm cli lint reports oxlint diagnostics', (t) => {
  withTempDir((dir) => {
    const filePath = join(dir, 'input.ts')
    writeFileSync(filePath, 'debugger\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1', '-D', 'no-debugger'], dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.true(`${result.stdout}\n${result.stderr}`.includes('no-debugger'), 'Should report oxlint rule diagnostics')
  })
})

test('npm package carries ArkTS lint configuration schema', (t) => {
  const schema = JSON.parse(readFileSync(schemaPath, 'utf8'))
  const plugins = schema.definitions.LintPluginOptionsSchema.enum

  t.true(plugins.includes('arkts'))
  t.truthy(schema.definitions.DummyRuleMap.properties['arkts/no-symbol'])
  t.is(
    schema.definitions.DummyRuleMap.properties['arkts/system-api-version'].allOf[0].$ref,
    '#/definitions/ArktsSystemApiVersionRule',
  )
})

test('npm cli lint init writes oxk schema path', (t) => {
  withTempDir((dir) => {
    const result = runCli(['lint', '--init'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)

    const config = JSON.parse(readFileSync(join(dir, '.oxlintrc.json'), 'utf8'))
    t.is(config.$schema, './node_modules/@ohos-rs/oxk/configuration_schema.json')
    t.true(config.plugins.includes('arkts'))
  })
})

test('npm cli lint loads .oxlintrc.jsonc', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.jsonc'),
      '{\n  // JSONC config should be accepted\n  "rules": { "no-debugger": "off" }\n}\n',
      'utf8',
    )
    const filePath = join(dir, 'input.ts')
    writeFileSync(filePath, 'debugger\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)
  })
})

testTsConfig('npm cli lint loads oxlint.config.ts', (t) => {
  withTempDir((dir) => {
    writeFileSync(join(dir, 'oxlint.config.ts'), "export default { rules: { 'no-debugger': 'off' } }\n", 'utf8')
    const filePath = join(dir, 'input.ts')
    writeFileSync(filePath, 'debugger\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1'], dir)

    t.is(result.status, 0, result.stderr || result.stdout)
  })
})

test('npm cli lint honors oxlint built-in plugin config', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        plugins: ['react'],
        rules: {
          'react/jsx-key': 'error',
        },
      }),
      'utf8',
    )
    const filePath = join(dir, 'input.tsx')
    writeFileSync(filePath, 'void [1, 2].map((item) => <span>{item}</span>)\n', 'utf8')

    const result = runCli(['lint', 'input.tsx', '--threads', '1'], dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.true(`${result.stdout}\n${result.stderr}`.includes('react(jsx-key)'), 'Should run configured react plugin rules')
  })
})

test('npm cli lint snapshots no-debugger diagnostics', (t) => {
  withTempDir((dir) => {
    writeFileSync(join(dir, 'input.ts'), 'debugger\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1', '--format', 'json', '-D', 'no-debugger'], dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.snapshot(lintJsonSnapshot(result, dir))
  })
})

test('npm cli lint snapshots configured react plugin diagnostics', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        plugins: ['react'],
        rules: {
          'react/jsx-key': 'error',
        },
      }),
      'utf8',
    )
    writeFileSync(join(dir, 'input.tsx'), 'void [1, 2].map((item) => <span>{item}</span>)\n', 'utf8')

    const result = runCli(['lint', 'input.tsx', '--threads', '1', '--format', 'json'], dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.snapshot(lintJsonSnapshot(result, dir))
  })
})

test('npm cli lint exposes source text to JS plugin runtime', (t) => {
  withTempDir((dir) => {
    const pluginPath = join(dir, 'source-runtime.js')
    writeFileSync(
      pluginPath,
      `module.exports = {
  meta: { name: 'source-runtime' },
  rules: {
    'source-text': {
      create(context) {
        return {
          Program(node) {
            const source = context.getSourceCode().getText()
            if (source.includes('raw buffer marker')) {
              context.report({ node, message: source.trim() })
            }
          },
        }
      },
    },
  },
}
`,
      'utf8',
    )
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        jsPlugins: [{ name: 'source-runtime', specifier: pluginPath }],
        rules: {
          'no-unused-vars': 'off',
          'source-runtime/source-text': 'error',
        },
      }),
      'utf8',
    )
    writeFileSync(join(dir, 'input.ts'), "const marker = 'raw buffer marker'\n", 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.true((report.output as any).diagnostics[0].message.includes('raw buffer marker'))
  })
})

test('npm cli lint uses oxlint JS runtime selectors and option schemas', (t) => {
  withTempDir((dir) => {
    const pluginPath = join(dir, 'selector-runtime.js')
    writeFileSync(
      pluginPath,
      `module.exports = {
  meta: { name: 'selector-runtime' },
  rules: {
    selector: {
      meta: {
        schema: [{
          type: 'object',
          properties: {
            message: { type: 'string', default: 'schema default applied' },
          },
          additionalProperties: false,
        }],
      },
      create(context) {
        return {
          'CallExpression[callee.name="target"]'(node) {
            context.report({ node, message: context.options[0].message })
          },
        }
      },
    },
  },
}
`,
      'utf8',
    )
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        jsPlugins: [{ name: 'selector-runtime', specifier: pluginPath }],
        rules: {
          'no-unused-vars': 'off',
          'selector-runtime/selector': ['error', {}],
        },
      }),
      'utf8',
    )
    writeFileSync(join(dir, 'input.ts'), 'target()\nignored()\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.deepEqual(
      (report.output as any).diagnostics.map((diagnostic: any) => diagnostic.message),
      ['schema default applied'],
    )
  })
})

test('npm cli lint passes settings and globals to JS runtime', (t) => {
  withTempDir((dir) => {
    const pluginPath = join(dir, 'context-runtime.js')
    writeFileSync(
      pluginPath,
      `module.exports = {
  meta: { name: 'context-runtime' },
  rules: {
    context: {
      create(context) {
        return {
          Program(node) {
            const configured = context.settings.customRuntime?.message
            const globalMode = context.languageOptions.globals.configuredGlobal
            if (configured && globalMode) {
              context.report({ node, message: configured + ':' + globalMode })
            }
          },
        }
      },
    },
  },
}
`,
      'utf8',
    )
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        jsPlugins: [{ name: 'context-runtime', specifier: pluginPath }],
        settings: {
          customRuntime: { message: 'settings reached runtime' },
        },
        globals: {
          configuredGlobal: 'readonly',
        },
        rules: {
          'no-unused-expressions': 'off',
          'no-unused-vars': 'off',
          'context-runtime/context': 'error',
        },
      }),
      'utf8',
    )
    writeFileSync(join(dir, 'input.ts'), 'configuredGlobal\n', 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.deepEqual(
      (report.output as any).diagnostics.map((diagnostic: any) => diagnostic.message),
      ['settings reached runtime:readonly'],
    )
  })
})

test('npm cli lint snapshots Rust arkts no-symbol diagnostics', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.jsonc'),
      '{\n  // Registering the plugin does not enable every ArkTS rule.\n  "plugins": ["arkts"],\n  "rules": {\n    "no-unused-vars": "off",\n    "arkts/no-symbol": "error"\n  }\n}\n',
      'utf8',
    )
    writeFileSync(join(dir, 'input.ets'), "const key = Symbol('id')\nlet marker: symbol\n", 'utf8')

    const result = runCli(['lint', 'input.ets', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.snapshot(report)
  })
})

test('npm cli lint snapshots Rust arkts system API version diagnostics', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        plugins: ['arkts'],
        rules: {
          'no-unused-vars': 'off',
          'arkts/system-api-version': [
            'error',
            {
              minApiVersion: 11,
            },
          ],
        },
      }),
      'utf8',
    )
    writeFileSync(
      join(dir, 'input.ets'),
      "import { router } from '@kit.ArkUI'\nrouter.back()\nrouter.push()\nrouter.showAlertBeforeBackPage()\n",
      'utf8',
    )

    const result = runCli(['lint', 'input.ets', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.snapshot(report)
  })
})

testTsConfig('npm cli lint runs Rust arkts rules from oxlint.config.ts', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, 'oxlint.config.ts'),
      "export default { plugins: ['arkts'], rules: { 'no-unused-vars': 'off', 'arkts/no-var': 'error' } }\n",
      'utf8',
    )
    writeFileSync(join(dir, 'input.ets'), 'var value = 1\n', 'utf8')

    const result = runCli(['lint', 'input.ets', '--threads', '1', '--format', 'json'], dir)
    const report = lintJsonSnapshot(result, dir)

    t.is(result.status, 1, result.stdout || result.stderr)
    t.deepEqual(
      (report.output as any).diagnostics.map((diagnostic: any) => diagnostic.code),
      ['arkts(no-var)'],
    )
  })
})

test('npm cli lint skips Rust arkts rules for TypeScript files', (t) => {
  withTempDir((dir) => {
    writeFileSync(
      join(dir, '.oxlintrc.json'),
      JSON.stringify({
        plugins: ['arkts'],
        rules: {
          'no-unused-vars': 'off',
          'arkts/no-symbol': 'error',
        },
      }),
      'utf8',
    )
    writeFileSync(join(dir, 'input.ts'), "const key = Symbol('id')\n", 'utf8')

    const result = runCli(['lint', 'input.ts', '--threads', '1'], dir)

    t.is(result.status, 0, result.stdout || result.stderr)
  })
})
