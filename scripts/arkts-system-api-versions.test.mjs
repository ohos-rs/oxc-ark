import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

const sync = resolve('scripts/sync-arkts-system-api-versions.mjs')
const verify = resolve('scripts/verify-arkts-system-api-versions.mjs')

test('API26 SDK generation and independent verification reject incomplete or incorrect tables', () => {
  const root = mkdtempSync(join(tmpdir(), 'oxk-api26-'))
  const output = join(root, 'versions.rs')
  const write = (path, text) => writeFileSync(join(root, path), text)
  const run = (script, ...args) =>
    spawnSync(process.execPath, [script, root, '--output', output, ...args], { encoding: 'utf8' })
  const success = (result) => assert.equal(result.status, 0, result.stdout + result.stderr)
  try {
    for (const vendor of ['openharmony', 'hms']) {
      for (const kind of ['api', 'kits']) mkdirSync(join(root, vendor, 'ets', kind), { recursive: true })
    }
    write('sdk-pkg.json', JSON.stringify({ data: { version: '26.0.0.105', apiVersion: '26' } }))
    write(
      'openharmony/ets/api/@ohos.demo.d.ts',
      `
/** @since 9 */
declare namespace demo {
  /** @since 9 */
  enum Mode {
    /** @since 26.0.0 */
    NEW = 1,
  }
  /** @since 10 @deprecated since 26.0.0 */
  function legacy(): void;
}
export default demo;
`,
    )
    write(
      'hms/ets/api/@hms.Sample.d.ets',
      `
/** @since 5.0.0(12) */
export default struct Sample {
  /** @since 26.0.0 */
  value: string;
}
/** @since 6.0.0(20) */
export interface Options {
  /** @since 6.0.0(20) @deprecated since 6.1.0(23) */
  old: string;
}
`,
    )
    write(
      'hms/ets/kits/@kit.Demo.d.ts',
      `
import demo from '@ohos.demo';
export { demo, Sample, Settings };
import Sample, { Options as Settings } from '@hms.Sample';
`,
    )
    success(run(sync, '--api-version', '26'))
    success(run(verify))
    success(run(sync, '--api-version', '26', '--check'))
    assert.notEqual(run(sync, '--api-version', '25').status, 0)
    const original = readFileSync(output, 'utf8')
    for (const [api, since, removed] of [
      ['@ohos.demo.Mode.NEW', 26, 'None'],
      ['@kit.Demo.demo.legacy', 10, 'Some(26)'],
      ['@hms.Sample', 12, 'None'],
      ['@kit.Demo.Sample.value', 26, 'None'],
      ['@kit.Demo.Settings.old', 20, 'Some(23)'],
    ]) {
      assert.ok(original.includes(`("${api}", SystemApiVersion { since: ${since}, removed: ${removed} })`), api)
    }
    const mutations = [
      original.replace(/.*"@kit.Demo.Sample.value".*\n/u, ''),
      original.replace(
        '"@ohos.demo.Mode.NEW", SystemApiVersion { since: 26',
        '"@ohos.demo.Mode.NEW", SystemApiVersion { since: 25',
      ),
      original.replace('\n];', '\n    ("@hms.unknown", SystemApiVersion { since: 26, removed: None }),\n];'),
      original.replace('\n];', '\n    ("@hms.Sample", SystemApiVersion { since: 12, removed: None }),\n];'),
    ]
    for (const mutation of mutations) {
      writeFileSync(output, mutation)
      assert.notEqual(run(verify).status, 0, 'verification must reject damaged data')
      assert.notEqual(run(sync, '--check').status, 0, 'check must detect stale output')
    }
    writeFileSync(output, original)
    write('hms/ets/api/@hms.broken.d.ets', 'export struct Broken { value: ; }')
    assert.notEqual(run(verify).status, 0, 'ArkTS parse failures must not be silently skipped')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
