#!/usr/bin/env node

import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { basename, dirname, extname, join, relative, resolve } from 'node:path'
import process from 'node:process'

const DEFAULT_OUTPUT_FILE = 'crates/lint/src/arkts/system_api_versions.rs'
const SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'dist', 'build'])

function usage() {
  console.error(`Usage: pnpm run sync:arkts-api-versions -- <api-source-path> [--output <system_api_versions.rs>] [--dry-run]

The source path can be:
  - a JSON/JSONC file containing { "apis": { "@ohos.router.back": 9 } }
  - a JSON/JSONC file containing { "apis": { "@ohos.old": { "since": 9, "removed": 12 } } }
  - a JSON/JSONC file containing { "apis": { "@ohos.legacy": { "since": 9, "deprecatedSince": 12 } } }
  - a JSON/JSONC file containing { "@ohos.router.back": 9 }
  - a directory containing JSON/JSONC files and OpenHarmony SDK .d.ts files with @since and removal tags

When the source path is an OpenHarmony ets/api or ets/kits directory, the sibling
directory is included automatically so @kit.* aliases are generated from the
underlying @ohos.* and @system.* declarations.`)
}

function parseArgs(argv) {
  const args = [...argv]
  let sourcePath = null
  let outputFile = DEFAULT_OUTPUT_FILE
  let dryRun = false

  while (args.length > 0) {
    const arg = args.shift()
    if (arg === '--output' || arg === '--rust') {
      const value = args.shift()
      if (!value) throw new Error(`${arg} requires a file path`)
      outputFile = value
    } else if (arg === '--dry-run') {
      dryRun = true
    } else if (arg === '--help' || arg === '-h') {
      usage()
      process.exit(0)
    } else if (arg === '--') {
      continue
    } else if (!sourcePath) {
      sourcePath = arg
    } else {
      throw new Error(`Unexpected argument: ${arg}`)
    }
  }

  if (!sourcePath) throw new Error('Missing API source path')

  return {
    sourcePath: resolve(sourcePath),
    outputFile: resolve(outputFile),
    dryRun,
  }
}

function stripJsonComments(source) {
  let output = ''
  let inString = false
  let quote = ''
  let escaped = false

  for (let index = 0; index < source.length; index += 1) {
    const char = source[index]
    const next = source[index + 1]

    if (inString) {
      output += char
      if (escaped) {
        escaped = false
      } else if (char === '\\') {
        escaped = true
      } else if (char === quote) {
        inString = false
      }
      continue
    }

    if (char === '"' || char === "'") {
      inString = true
      quote = char
      output += char
      continue
    }

    if (char === '/' && next === '/') {
      while (index < source.length && source[index] !== '\n') {
        output += ' '
        index += 1
      }
      if (index < source.length) output += '\n'
      continue
    }

    if (char === '/' && next === '*') {
      output += '  '
      index += 2
      while (index < source.length) {
        if (source[index] === '*' && source[index + 1] === '/') {
          output += '  '
          index += 1
          break
        }
        output += source[index] === '\n' ? '\n' : ' '
        index += 1
      }
      continue
    }

    output += char
  }

  return output
}

function parseVersion(value) {
  if (Number.isInteger(value) && value >= 0) return value
  if (typeof value === 'string' && /^\d+$/.test(value)) return Number(value)
  return null
}

function versionFromObject(value, keys) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  for (const key of keys) {
    const parsed = parseVersion(value[key])
    if (parsed != null) return parsed
  }
  return null
}

function parseApiVersion(api, value, removedValue = null) {
  const scalarSince = parseVersion(value)
  if (scalarSince != null) {
    const removed = parseVersion(removedValue)
    if (removed != null && removed < scalarSince) {
      throw new Error(`Removal version for ${api} must be greater than or equal to since version`)
    }
    return { since: scalarSince, removed }
  }

  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`API version for ${api} must be an integer or an object with "since"`)
  }

  const since = versionFromObject(value, ['since', 'version', 'apiVersion', 'apiLevel'])
  if (since == null) throw new Error(`API version object for ${api} must include integer "since"`)

  const removed = versionFromObject(value, [
    'removed',
    'removedVersion',
    'deleteVersion',
    'deletedVersion',
    'removalVersion',
    'deprecatedSince',
    'deprecatedVersion',
  ])
  if (removed != null && removed < since) {
    throw new Error(`Removal version for ${api} must be greater than or equal to since version`)
  }

  return { since, removed }
}

function addEntry(entries, api, version, removed = null) {
  if (typeof api !== 'string' || !api.startsWith('@')) return
  const nextVersion = parseApiVersion(api, version, removed)
  const previousVersion = entries.get(api)
  if (previousVersion == null) {
    entries.set(api, nextVersion)
    return
  }

  entries.set(api, combinePendingVersions([previousVersion, nextVersion]))
}

function collectFromJsonValue(value, entries) {
  if (Array.isArray(value)) {
    for (const item of value) collectFromJsonValue(item, entries)
    return
  }

  if (!value || typeof value !== 'object') return

  if (typeof value.name === 'string' || typeof value.api === 'string') {
    const api = value.name ?? value.api
    const version = value.version ?? value.since ?? value.apiVersion ?? value.apiLevel
    const removed =
      value.removed ??
      value.removedVersion ??
      value.deleteVersion ??
      value.deletedVersion ??
      value.removalVersion ??
      value.deprecatedSince ??
      value.deprecatedVersion
    if (version != null) addEntry(entries, api, version, removed)
  }

  const apiMap = value.apis ?? value.apiVersions
  if (apiMap && typeof apiMap === 'object' && !Array.isArray(apiMap)) {
    for (const [api, version] of Object.entries(apiMap)) addEntry(entries, api, version)
  }

  for (const [key, nested] of Object.entries(value)) {
    if (key.startsWith('@')) {
      addEntry(entries, key, nested)
    } else if (key !== 'apis' && key !== 'apiVersions') {
      collectFromJsonValue(nested, entries)
    }
  }
}

function collectFromJsonFile(file, entries) {
  const source = stripJsonComments(readFileSync(file, 'utf8'))
  collectFromJsonValue(JSON.parse(source), entries)
}

function moduleNameFromFile(file, source) {
  const declaredModule = source.match(/declare\s+module\s+['"]([^'"]+)['"]/)
  if (declaredModule?.[1]?.startsWith('@')) return declaredModule[1]

  const name = basename(file).replace(/\.d\.ts$/u, '')
  if (name.startsWith('@')) return name
  return null
}

function isSystemModuleName(source) {
  return (
    source.startsWith('@ohos.') ||
    source.startsWith('@kit.') ||
    source.startsWith('@system.') ||
    source.startsWith('@hms.')
  )
}

function rootNamespaceName(moduleName) {
  return moduleName.slice(moduleName.lastIndexOf('.') + 1)
}

function sinceFromComment(comment) {
  const match = comment.match(/@since\s+(\d+)/u)
  return match ? Number(match[1]) : null
}

function removedFromComment(comment) {
  const match = comment.match(
    /@(?:removed|deleted|delete|removal)\s+(?:since\s+)?(\d+)|@deprecated\s+(?:since\s+)?(\d+)/u,
  )
  return match ? Number(match[1] ?? match[2]) : null
}

function versionFromComment(comment) {
  const since = sinceFromComment(comment)
  return since == null ? null : { since, removed: removedFromComment(comment) }
}

function combinePendingVersions(pendingVersions) {
  if (pendingVersions.length === 0) return null

  return {
    since: Math.min(...pendingVersions.map((version) => version.since)),
    removed: pendingVersions
      .map((version) => version.removed)
      .filter((version) => version != null)
      .reduce((left, right) => (left == null ? right : Math.min(left, right)), null),
  }
}

function declarationName(line) {
  const match = line.match(
    /^(?:export\s+)?(?:declare\s+)?(?:async\s+)?(?:function|class|interface|enum|const|let|var|type|namespace)\s+([A-Za-z_$][\w$]*)/u,
  )
  if (match) return match[1]

  const method = line.match(/^(?:static\s+)?([A-Za-z_$][\w$]*)\s*(?:<[^>]+>)?\s*\(/u)
  if (method) return method[1]

  const property = line.match(/^(?:readonly\s+)?([A-Za-z_$][\w$]*)\??\s*:/u)
  return property?.[1] ?? null
}

function declarationScope(line) {
  const match = line.match(/^(?:export\s+)?(?:declare\s+)?(class|interface|enum|namespace)\s+([A-Za-z_$][\w$]*)/u)
  if (!match) return null
  return { kind: match[1], name: match[2] }
}

function isEnumMember(line) {
  return /^[A-Za-z_$][\w$]*\s*(?:=\s*[^,]+)?[,]?$/u.test(line)
}

function parseNamedSpecifiers(source) {
  return source
    .split(',')
    .map((item) => item.trim().replace(/^type\s+/u, ''))
    .filter(Boolean)
    .map((item) => {
      const alias = item.match(/^([A-Za-z_$][\w$]*)\s+as\s+([A-Za-z_$][\w$]*)$/u)
      if (alias) return { imported: alias[1], local: alias[2] }
      return { imported: item, local: item }
    })
}

function collectKitImportAliases(source) {
  const aliases = new Map()
  const importRegex = /import\s+(?:type\s+)?([\s\S]*?)\s+from\s+['"]([^'"]+)['"]\s*;/gu

  for (const match of source.matchAll(importRegex)) {
    const specifiers = match[1].trim()
    const sourceModule = match[2]
    if (!isSystemModuleName(sourceModule)) continue

    if (specifiers.startsWith('* as ')) {
      const local = specifiers.slice(5).trim()
      if (local) aliases.set(local, sourceModule)
      continue
    }

    const namedStart = specifiers.indexOf('{')
    if (namedStart === -1) {
      aliases.set(specifiers, sourceModule)
      continue
    }

    const defaultSpecifier = specifiers.slice(0, namedStart).replace(/,$/u, '').trim()
    if (defaultSpecifier) aliases.set(defaultSpecifier, sourceModule)

    const namedEnd = specifiers.lastIndexOf('}')
    if (namedEnd !== -1) {
      const namedSpecifiers = specifiers.slice(namedStart + 1, namedEnd)
      for (const { imported, local } of parseNamedSpecifiers(namedSpecifiers)) {
        aliases.set(local, `${sourceModule}.${imported}`)
      }
    }
  }

  return aliases
}

function collectKitExports(source) {
  const exports = []
  const exportRegex = /export\s+(?:type\s+)?\{([\s\S]*?)\}\s*;/gu

  for (const match of source.matchAll(exportRegex)) {
    for (const { imported, local } of parseNamedSpecifiers(match[1])) {
      exports.push({ local: imported, exported: local })
    }
  }

  return exports
}

function addKitAliasEntries(entries, kitModule, exportedName, targetApi) {
  const aliasApi = `${kitModule}.${exportedName}`
  const targetVersion = entries.get(targetApi)
  if (targetVersion != null) entries.set(aliasApi, targetVersion)

  const targetPrefix = `${targetApi}.`
  for (const [api, version] of entries) {
    if (api.startsWith(targetPrefix)) {
      entries.set(`${aliasApi}${api.slice(targetApi.length)}`, version)
    }
  }
}

function collectFromKitDeclarationFile(file, entries) {
  const source = readFileSync(file, 'utf8')
  const moduleName = moduleNameFromFile(file, source)
  if (!moduleName?.startsWith('@kit.')) return

  const importAliases = collectKitImportAliases(source)
  for (const { local, exported } of collectKitExports(source)) {
    const targetApi = importAliases.get(local)
    if (targetApi != null) addKitAliasEntries(entries, moduleName, exported, targetApi)
  }
}

function collectFromDeclarationFile(file, entries) {
  const source = readFileSync(file, 'utf8')
  const moduleName = moduleNameFromFile(file, source)
  if (!moduleName) return

  const rootScope = rootNamespaceName(moduleName)
  const scopes = []
  let pendingVersions = []
  let comment = null

  for (const rawLine of source.split(/\r?\n/u)) {
    const line = rawLine.trim()

    if (comment != null) {
      comment += `\n${line}`
      if (line.includes('*/')) {
        const version = versionFromComment(comment)
        if (version != null) pendingVersions.push(version)
        comment = null
      }
      continue
    }

    if (line.startsWith('/**')) {
      comment = line
      if (line.includes('*/')) {
        const version = versionFromComment(comment)
        if (version != null) pendingVersions.push(version)
        comment = null
      }
      continue
    }

    if (line === '' || line.startsWith('*') || line.startsWith('//')) continue

    if (pendingVersions.length > 0) {
      const name = declarationName(line)
      if (name) {
        const segments = [...scopes.map((scope) => scope.name), name].filter((segment) => segment !== rootScope)
        const api = segments.length === 0 ? moduleName : `${moduleName}.${segments.join('.')}`
        addEntry(entries, api, combinePendingVersions(pendingVersions))
        pendingVersions = []
      } else if (scopes.at(-1)?.kind === 'enum' && isEnumMember(line)) {
        pendingVersions = []
      }
    }

    const openingScope = declarationScope(line)
    if (openingScope && line.includes('{')) scopes.push(openingScope)

    const closes = (line.match(/\}/gu) ?? []).length
    for (let index = 0; index < closes; index += 1) scopes.pop()
  }
}

function isKitDeclarationFile(path) {
  return path.endsWith('.d.ts') && basename(path).startsWith('@kit.')
}

function collectSourcePaths(sourcePath) {
  const paths = [sourcePath]
  const stat = statSync(sourcePath)
  if (stat.isDirectory()) {
    const name = basename(sourcePath)
    if (name === 'api') {
      const kitsPath = join(dirname(sourcePath), 'kits')
      if (existsSync(kitsPath)) paths.push(kitsPath)
    } else if (name === 'kits') {
      const apiPath = join(dirname(sourcePath), 'api')
      if (existsSync(apiPath)) paths.unshift(apiPath)
    }
  }
  return [...new Set(paths)]
}

function collectFromPath(path, entries, kitFiles) {
  const stat = statSync(path)
  if (stat.isDirectory()) {
    for (const name of readdirSync(path).sort()) {
      if (SKIP_DIRS.has(name)) continue
      collectFromPath(join(path, name), entries, kitFiles)
    }
    return
  }

  if (!stat.isFile()) return

  const extension = extname(path)
  if (extension === '.json' || extension === '.jsonc') {
    collectFromJsonFile(path, entries)
  } else if (path.endsWith('.d.ts')) {
    if (isKitDeclarationFile(path)) {
      kitFiles.push(path)
    } else {
      collectFromDeclarationFile(path, entries)
    }
  }
}

function rustString(value) {
  return JSON.stringify(value)
}

function renderVersion(version) {
  const removed = version.removed == null ? 'None' : `Some(${version.removed})`
  return `SystemApiVersion { since: ${version.since}, removed: ${removed} }`
}

function renderRustTable(entries) {
  const rows = [...entries]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([api, version]) => `    (${rustString(api)}, ${renderVersion(version)}),`)

  return `// This file is generated by scripts/sync-arkts-system-api-versions.mjs.
// Do not edit by hand.

#[derive(Clone, Copy, Debug)]
pub(super) struct SystemApiVersion {
    pub since: u32,
    pub removed: Option<u32>,
}

#[rustfmt::skip]
pub(super) static SYSTEM_API_VERSIONS: &[(&str, SystemApiVersion)] = &[
${rows.join('\n')}
];
`
}

function updateRustOutput(outputFile, entries, dryRun) {
  const nextTable = renderRustTable(entries)
  const previous = existsSync(outputFile) ? readFileSync(outputFile, 'utf8') : null
  if (!dryRun && nextTable !== previous) {
    mkdirSync(dirname(outputFile), { recursive: true })
    writeFileSync(outputFile, nextTable, 'utf8')
  }
  return nextTable !== previous
}

try {
  const { sourcePath, outputFile, dryRun } = parseArgs(process.argv.slice(2))
  if (!existsSync(sourcePath)) throw new Error(`Source path does not exist: ${sourcePath}`)

  const entries = new Map()
  const kitFiles = []
  const sourcePaths = collectSourcePaths(sourcePath)
  for (const path of sourcePaths) collectFromPath(path, entries, kitFiles)
  for (const file of kitFiles.sort()) collectFromKitDeclarationFile(file, entries)
  if (entries.size === 0) {
    throw new Error(`No system API version entries found in ${sourcePath}`)
  }

  const changed = updateRustOutput(outputFile, entries, dryRun)
  const relativeSource = sourcePaths.map((path) => relative(process.cwd(), path) || '.').join(', ')
  const relativeOutput = relative(process.cwd(), outputFile)
  console.log(
    `${dryRun ? 'Checked' : 'Synced'} ${entries.size} ArkTS system API version entries from ${relativeSource} to ${relativeOutput}${changed ? '' : ' (unchanged)'}.`,
  )
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  usage()
  process.exit(1)
}
