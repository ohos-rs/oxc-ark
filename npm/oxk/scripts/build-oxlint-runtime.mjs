import { createRequire } from 'node:module'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import esbuild from 'esbuild'

const require = createRequire(import.meta.url)
const packageRoot = path.resolve(import.meta.dirname, '..')
const workspaceRoot = path.resolve(packageRoot, '../..')
const packageJson = require(path.join(packageRoot, 'package.json'))
const upstreamSrc = resolveUpstreamSrc()
const upstreamSharedEntry = resolveUpstreamSharedEntry(upstreamSrc)
const outDir = path.join(packageRoot, 'oxlint-runtime')
const tempDir = fs.mkdtempSync(path.join(tmpdir(), 'oxk-oxlint-runtime-build-'))

fs.mkdirSync(outDir, { recursive: true })

const pluginsEntry = path.join(tempDir, 'plugins-entry.ts')
const jsConfigEntry = path.join(tempDir, 'js-config-entry.ts')

fs.writeFileSync(
  pluginsEntry,
  `export { lintFile, loadPlugin, setupRuleConfigs } from ${JSON.stringify(
    path.join(upstreamSrc, 'plugins/index.ts'),
  )};\n`,
)
fs.writeFileSync(
  jsConfigEntry,
  `export { loadJsConfigs, loadVitePlusConfigs } from ${JSON.stringify(path.join(upstreamSrc, 'js_config.ts'))};\n`,
)

const eslintPackageJson = require.resolve('eslint/package.json')
const eslintCodePath = path.join(path.dirname(eslintPackageJson), 'lib/linter/code-path-analysis/code-path-analyzer.js')

const patchPlugin = {
  name: 'oxk-runtime-patches',
  setup(build) {
    build.onResolve(
      {
        filter: /^\.\.\/\.\.\/node_modules\/eslint\/lib\/linter\/code-path-analysis\/code-path-analyzer\.js$/,
      },
      () => ({ path: eslintCodePath }),
    )
    build.onResolve({ filter: /^\.\.\/\.\.\/package\.json$/ }, () => ({
      path: 'oxk-package-json',
      namespace: 'oxk',
    }))
    build.onLoad({ filter: /^oxk-package-json$/, namespace: 'oxk' }, () => ({
      contents: `export const version = ${JSON.stringify(packageJson.version)}; export default { version };`,
      loader: 'js',
    }))
    build.onResolve({ filter: /^@oxapps\/shared$/ }, () => ({
      path: upstreamSharedEntry,
    }))
  },
}

const commonOptions = {
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node20',
  define: {
    DEBUG: 'false',
    CONFORMANCE: 'false',
  },
  external: ['vite-plus'],
  plugins: [patchPlugin],
  nodePaths: [path.join(packageRoot, 'node_modules')],
  minify: true,
  legalComments: 'eof',
  logLevel: 'info',
}

try {
  await esbuild.build({
    ...commonOptions,
    entryPoints: [pluginsEntry],
    outfile: path.join(outDir, 'plugins.cjs'),
  })
  await esbuild.build({
    ...commonOptions,
    entryPoints: [jsConfigEntry],
    outfile: path.join(outDir, 'js_config.cjs'),
  })
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true })
}

function resolveUpstreamSrc() {
  if (process.env.OXK_OXLINT_SRC_JS) {
    return path.resolve(process.env.OXK_OXLINT_SRC_JS)
  }

  const metadata = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1'], {
      cwd: workspaceRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'inherit'],
    }),
  )
  const oxlintPackage = metadata.packages.find((pkg) => pkg.name === 'oxlint')
  if (!oxlintPackage?.manifest_path) {
    throw new Error('Unable to locate the oxlint package with cargo metadata.')
  }

  const src = path.join(path.dirname(oxlintPackage.manifest_path), 'src-js')
  if (!fs.existsSync(path.join(src, 'plugins/index.ts')) || !fs.existsSync(path.join(src, 'js_config.ts'))) {
    throw new Error(`Resolved oxlint package does not contain src-js runtime sources: ${src}`)
  }
  return src
}

function resolveUpstreamSharedEntry(src) {
  const sharedEntry = path.resolve(src, '..', '..', 'shared', 'src-js', 'index.ts')
  if (!fs.existsSync(sharedEntry)) {
    throw new Error(`Resolved OXC checkout does not contain @oxapps/shared runtime source: ${sharedEntry}`)
  }
  return sharedEntry
}
