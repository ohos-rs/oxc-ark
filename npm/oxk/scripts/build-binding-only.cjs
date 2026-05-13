#!/usr/bin/env node

const { spawnSync } = require('node:child_process')
const { readFileSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')

const root = join(__dirname, '..')
const generatedWrappers = ['index.js', 'index.d.ts', 'oxk.wasi.cjs', 'oxk.wasi-browser.js']
const snapshots = new Map()

for (const file of generatedWrappers) {
  const path = join(root, file)
  try {
    snapshots.set(path, readFileSync(path))
  } catch {}
}

const command = process.platform === 'win32' ? 'napi.cmd' : 'napi'
const args = ['build', '--platform', '--no-js', '--dts', '.oxk-napi.d.ts', '--no-dts-cache', ...process.argv.slice(2)]
const result = spawnSync(command, args, {
  cwd: root,
  stdio: 'inherit',
  shell: process.platform === 'win32',
})

try {
  for (const [path, content] of snapshots) {
    writeFileSync(path, content)
  }
} catch (error) {
  console.error(`Failed to restore generated wrapper after binding build: ${error.message}`)
  process.exit(1)
}

if (result.error) {
  console.error(result.error.message)
  process.exit(1)
}

if (result.signal) {
  console.error(`napi build exited from signal ${result.signal}`)
  process.exit(1)
}

process.exit(result.status ?? 0)
