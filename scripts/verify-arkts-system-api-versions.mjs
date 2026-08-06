#!/usr/bin/env node
// 独立验证脚本：用 TypeScript AST 解析 SDK 声明与注释归属，与生成的版本表全量比对。
// 与 sync-arkts-system-api-versions.mjs 的行扫描逻辑相互独立，用于防止版本归因回归。
//
// Usage: pnpm run verify:arkts-api-versions -- <api-source-path> [--output <system_api_versions.rs>]
//
// The source path can be an OpenHarmony/HarmonyOS SDK `ets/api` directory
// (the sibling `ets/kits` directory is included automatically), a single
// declaration file, or a directory of declaration files.
//
// Exit code is 1 when any version mismatch, missing entry, kit alias mismatch,
// or extra entry from a parseable (non-ArkTS-only) file is found.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import ts from 'typescript'

const DEFAULT_OUTPUT_FILE = 'crates/lint/src/arkts/system_api_versions.rs'
const SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'dist', 'build'])

function usage() {
  console.error(`Usage: pnpm run verify:arkts-api-versions -- <api-source-path> [--output <system_api_versions.rs>]`)
}

function parseArgs(argv) {
  const args = [...argv]
  let sourcePath = null
  let outputFile = DEFAULT_OUTPUT_FILE

  while (args.length > 0) {
    const arg = args.shift()
    if (arg === '--output' || arg === '--rust') {
      const value = args.shift()
      if (!value) throw new Error(`${arg} requires a file path`)
      outputFile = value
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
  return { sourcePath: resolve(sourcePath), outputFile: resolve(outputFile) }
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

function collectFiles(dir, out) {
  for (const name of readdirSync(dir).sort()) {
    if (SKIP_DIRS.has(name)) continue
    const path = join(dir, name)
    const stat = statSync(path)
    if (stat.isDirectory()) {
      collectFiles(path, out)
    } else if ((name.endsWith('.d.ts') || name.endsWith('.d.ets')) && name.startsWith('@')) {
      out.push(path)
    }
  }
}

// ---------- 版本表解析 ----------

function parseVersionTable(outputFile) {
  const table = new Map()
  for (const line of readFileSync(outputFile, 'utf8').split('\n')) {
    const match = line.match(/\(("@[^"]+"), SystemApiVersion \{ since: (\d+), removed: (Some\(\d+\)|None) \}\),/)
    if (match) {
      table.set(match[1].slice(1, -1), {
        since: Number(match[2]),
        removed: match[3] === 'None' ? null : Number(match[3].slice(5, -1)),
      })
    }
  }
  return table
}

// ---------- 注释版本提取 ----------

const SINCE_RE = /@since\s+(\d+)/u
const REMOVED_RE = /@(?:removed|deleted|delete|removal)\s+(?:since\s+)?(\d+)|@deprecated\s+(?:since\s+)?(\d+)/u

function versionFromComments(comments) {
  const sines = []
  const removeds = []
  for (const comment of comments) {
    const since = comment.match(SINCE_RE)
    if (since) sines.push(Number(since[1]))
    const removed = comment.match(REMOVED_RE)
    if (removed) removeds.push(Number(removed[1] ?? removed[2]))
  }
  if (sines.length === 0) return null
  return {
    since: Math.min(...sines),
    removed: removeds.length === 0 ? null : Math.min(...removeds),
  }
}

function leadingJSDocComments(source, node) {
  const ranges = ts.getLeadingCommentRanges(source, node.pos) ?? []
  const blocks = []
  for (const range of ranges) {
    if (range.kind !== ts.SyntaxKind.MultiLineCommentTrivia) continue
    const text = source.slice(range.pos, range.end)
    if (!text.includes('*')) continue // 空 /* */
    blocks.push(text)
  }
  return blocks
}

// ---------- AST 遍历推导 ----------

function addExpected(expected, api, version) {
  if (!version) return
  const previous = expected.get(api)
  if (!previous) {
    expected.set(api, version)
    return
  }
  expected.set(api, {
    since: Math.min(previous.since, version.since),
    removed:
      previous.removed != null && version.removed != null
        ? Math.min(previous.removed, version.removed)
        : (previous.removed ?? version.removed),
  })
}

function walk(container, sourceFile, moduleName, rootScope, scopeNames, expected) {
  const childScopes = []
  const visit = (node) => {
    const kind = node.kind
    let name = null
    let isMember = false
    switch (kind) {
      case ts.SyntaxKind.FirstStatement: {
        // TS 无法解析的语句（如 ArkTS 合法的 `const x: number;` 带类型无初始化，
        // 或个别文件里带 optional 参数的方法）。
        // 注意：FirstStatement 与 VariableStatement 的 SyntaxKind 数值相同（244），
        // 必须放在 VariableStatement case 之前，否则永远不会命中。
        const text = node.getText(sourceFile)
        const match =
          text.match(/^(?:export\s+)?(?:declare\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)/u) ??
          text.match(/^[A-Za-z_$][\w$]*\??\s*\(/u)
        const name = match ? (match[1] ?? match[0].match(/^[A-Za-z_$][\w$]*/u)?.[0]) : null
        if (name) {
          const segments = [...scopeNames, name].filter((segment) => segment !== rootScope)
          const api = segments.length === 0 ? moduleName : `${moduleName}.${segments.join('.')}`
          addExpected(expected, api, versionFromComments(leadingJSDocComments(sourceFile.text, node)))
        }
        ts.forEachChild(node, visit)
        return
      }
      case ts.SyntaxKind.VariableStatement: {
        for (const declaration of node.declarationList.declarations) {
          const segments = [...scopeNames, declaration.name.text].filter((segment) => segment !== rootScope)
          const api = segments.length === 0 ? moduleName : `${moduleName}.${segments.join('.')}`
          addExpected(expected, api, versionFromComments(leadingJSDocComments(sourceFile.text, declaration)))
        }
        return
      }
      case ts.SyntaxKind.FunctionDeclaration:
      case ts.SyntaxKind.InterfaceDeclaration:
      case ts.SyntaxKind.ClassDeclaration:
      case ts.SyntaxKind.EnumDeclaration:
      case ts.SyntaxKind.TypeAliasDeclaration:
      case ts.SyntaxKind.ModuleDeclaration:
        name = node.name?.text ?? null
        break
      case ts.SyntaxKind.MethodSignature:
      case ts.SyntaxKind.MethodDeclaration:
      case ts.SyntaxKind.PropertySignature:
      case ts.SyntaxKind.PropertyDeclaration:
      case ts.SyntaxKind.GetAccessor:
      case ts.SyntaxKind.SetAccessor:
        name = node.name?.text ?? null
        isMember = true
        break
      default:
        if (kind === ts.SyntaxKind.ModuleBlock || kind === ts.SyntaxKind.SourceFile) {
          ts.forEachChild(node, visit) // 透明容器：下钻
        }
        return
    }
    if (name == null) return

    const isScope =
      kind === ts.SyntaxKind.InterfaceDeclaration ||
      kind === ts.SyntaxKind.ClassDeclaration ||
      kind === ts.SyntaxKind.EnumDeclaration ||
      kind === ts.SyntaxKind.ModuleDeclaration
    const isModuleWrapper = kind === ts.SyntaxKind.ModuleDeclaration && ts.isStringLiteral(node.name)
    if (
      !isModuleWrapper &&
      (isMember ||
        kind === ts.SyntaxKind.FunctionDeclaration ||
        kind === ts.SyntaxKind.TypeAliasDeclaration ||
        isScope ||
        kind === ts.SyntaxKind.VariableStatement)
    ) {
      const segments = [...scopeNames, name].filter((segment) => segment !== rootScope)
      const api = segments.length === 0 ? moduleName : `${moduleName}.${segments.join('.')}`
      addExpected(expected, api, versionFromComments(leadingJSDocComments(sourceFile.text, node)))
    }
    if (isScope) childScopes.push({ name, node })
  }
  ts.forEachChild(container, visit)
  for (const { name, node } of childScopes) {
    walk(node, sourceFile, moduleName, rootScope, [...scopeNames, name], expected)
  }
}

function collectExpectedFromDeclarationFile(file, expected, parseErrorByModule) {
  const source = readFileSync(file, 'utf8')
  const declared = source.match(/declare\s+module\s+['"]([^'"]+)['"]/u)
  const moduleName = declared?.[1]?.startsWith('@') ? declared[1] : basename(file).replace(/\.d\.(m?ts|ets)$/u, '')
  if (!moduleName?.startsWith('@')) return

  const sourceFile = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  parseErrorByModule.set(
    moduleName,
    sourceFile.parseDiagnostics.filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error).length,
  )
  const rootScope = moduleName.slice(moduleName.lastIndexOf('.') + 1)
  walk(sourceFile, sourceFile, moduleName, rootScope, [], expected)
}

// ---------- @kit 别名验证 ----------

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

function isSystemModuleName(source) {
  return (
    source.startsWith('@ohos.') ||
    source.startsWith('@kit.') ||
    source.startsWith('@system.') ||
    source.startsWith('@hms.')
  )
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

// 验证 @kit.MOD.exported 及其子条目与 @ohos 目标条目版本一致
function verifyKitAliases(kitFile, table) {
  const source = readFileSync(kitFile, 'utf8')
  const moduleName = basename(kitFile).replace(/\.d\.ts$/u, '')
  if (!moduleName?.startsWith('@kit.')) return { checked: 0, bad: [] }

  const importAliases = collectKitImportAliases(source)
  const issues = []
  let checked = 0
  for (const { local, exported } of collectKitExports(source)) {
    const targetApi = importAliases.get(local)
    if (targetApi == null) continue
    const aliasApi = `${moduleName}.${exported}`
    const targetVersion = table.get(targetApi)
    const aliasVersion = table.get(aliasApi)
    if (targetVersion != null && aliasVersion != null) {
      checked += 1
      if (aliasVersion.since !== targetVersion.since || aliasVersion.removed !== targetVersion.removed) {
        issues.push(
          `${aliasApi} (since=${aliasVersion.since}, removed=${aliasVersion.removed}) 与 ${targetApi} (since=${targetVersion.since}, removed=${targetVersion.removed}) 不一致`,
        )
      }
    } else if (targetVersion != null && aliasVersion == null) {
      checked += 1
      issues.push(`@kit 别名缺失: ${aliasApi}（目标 ${targetApi} 存在）`)
    } else if (targetVersion == null && aliasVersion != null) {
      checked += 1
      issues.push(`@kit 别名多余: ${aliasApi}（目标 ${targetApi} 不存在）`)
    }

    // 子条目：@kit.MOD.exported.* 应与 targetApi.* 一致
    const targetPrefix = `${targetApi}.`
    const aliasPrefix = `${aliasApi}.`
    for (const [api, version] of table) {
      if (!api.startsWith(aliasPrefix) || api === aliasApi) continue
      const targetSuffix = api.slice(aliasPrefix.length)
      const targetSub = table.get(`${targetApi}.${targetSuffix}`)
      checked += 1
      if (targetSub == null) {
        issues.push(`@kit 子条目无对应 @ohos: ${api}`)
      } else if (targetSub.since !== version.since || targetSub.removed !== version.removed) {
        issues.push(`${api} (since=${version.since}) 与 ${targetApi}.${targetSuffix} (since=${targetSub.since}) 不一致`)
      }
    }
  }
  return { checked, bad: issues }
}

// ---------- 主流程 ----------

try {
  const { sourcePath, outputFile } = parseArgs(process.argv.slice(2))
  if (!existsSync(sourcePath)) throw new Error(`Source path does not exist: ${sourcePath}`)

  const table = parseVersionTable(outputFile)
  if (table.size === 0) throw new Error(`No version entries found in ${outputFile}`)

  const sourcePaths = collectSourcePaths(sourcePath)
  const apiPath = sourcePaths.find((path) => basename(path) === 'api') ?? sourcePaths[0]

  const expected = new Map()
  const parseErrorByModule = new Map()
  const files = []
  collectFiles(apiPath, files)
  for (const file of files) collectExpectedFromDeclarationFile(file, expected, parseErrorByModule)

  // 比对
  const mismatches = []
  const missing = []
  for (const [api, version] of expected) {
    const tableVersion = table.get(api)
    if (!tableVersion) {
      missing.push(api)
      continue
    }
    if (tableVersion.since !== version.since || tableVersion.removed !== version.removed) {
      mismatches.push(
        `${api}: 表(since=${tableVersion.since}, removed=${tableVersion.removed}) vs SDK(since=${version.since}, removed=${version.removed})`,
      )
    }
  }

  // 多余条目分类：解析报错文件（ArkTS 独有语法，无法用 TS AST 验证）vs 干净文件
  const extras = [...table.keys()].filter((api) => !api.startsWith('@kit.') && !expected.has(api))
  const cleanFileExtras = []
  const arktsFileExtras = []
  for (const api of extras) {
    let module = null
    for (const candidate of parseErrorByModule.keys()) {
      if (api === candidate || api.startsWith(`${candidate}.`)) {
        if (!module || candidate.length > module.length) module = candidate
      }
    }
    if (module && parseErrorByModule.get(module) === 0) cleanFileExtras.push(api)
    else arktsFileExtras.push(api)
  }

  // @kit 别名验证
  let kitChecked = 0
  const kitIssues = []
  const kitsPath = sourcePaths.find((path) => basename(path) === 'kits')
  if (kitsPath) {
    const kitFiles = []
    collectFiles(kitsPath, kitFiles)
    for (const kitFile of kitFiles) {
      const result = verifyKitAliases(kitFile, table)
      kitChecked += result.checked
      kitIssues.push(...result.bad)
    }
  }

  console.log(
    `Verified ${expected.size} declarations from ${files.length} SDK files against ${table.size} table entries.`,
  )
  console.log(`@kit aliases checked: ${kitChecked}.`)
  console.log(`\n=== 版本不符: ${mismatches.length} ===`)
  for (const mismatch of mismatches.slice(0, 120)) console.log(`  ${mismatch}`)
  console.log(`\n=== 表中缺失: ${missing.length} ===`)
  for (const api of missing.slice(0, 60)) console.log(`  ${api}`)
  console.log(
    `\n=== 表中多余: ${extras.length}（${arktsFileExtras.length} 条来自 ArkTS 语法文件，${cleanFileExtras.length} 条来自干净文件）===`,
  )
  for (const api of cleanFileExtras.slice(0, 40)) console.log(`  [需排查] ${api}`)
  console.log(`\n=== @kit 别名问题: ${kitIssues.length} ===`)
  for (const issue of kitIssues.slice(0, 40)) console.log(`  ${issue}`)

  const failed = mismatches.length + missing.length + cleanFileExtras.length + kitIssues.length
  if (failed > 0) {
    console.error(`\nFAILED: ${failed} issue(s) found.`)
    process.exit(1)
  }
  console.log('\nOK: all declarations match the version table.')
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  usage()
  process.exit(1)
}
