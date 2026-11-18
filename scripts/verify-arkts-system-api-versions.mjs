#!/usr/bin/env node
// 独立验证脚本：用 TypeScript AST 解析 SDK 声明与注释归属，与生成的版本表全量比对。
// 与 sync-arkts-system-api-versions.mjs 的行扫描逻辑相互独立，用于防止版本归因回归。
//
// Usage: pnpm run verify:arkts-api-versions -- <api-source-path> [--output <system_api_versions.rs>]
//
// The source path can be a complete HarmonyOS SDK root (OpenHarmony + HMS),
// an OpenHarmony/HarmonyOS SDK `ets/api` directory
// (the sibling `ets/kits` directory is included automatically), a single
// declaration file, or a directory of declaration files.
//
// Exit code is 1 for any version mismatch, missing/extra/duplicate entry,
// kit alias mismatch, or declaration parse error (including ArkTS declarations).

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
  // A complete HarmonyOS SDK has both OpenHarmony and HMS declarations.
  const sdkPaths = ['openharmony', 'hms']
    .flatMap((vendor) => ['api', 'kits'].map((kind) => join(sourcePath, vendor, 'ets', kind)))
    .filter((path) => existsSync(path))
  if (sdkPaths.length > 0) return sdkPaths
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
  if (statSync(dir).isFile()) {
    out.push(dir)
    return
  }
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
      const api = match[1].slice(1, -1)
      if (table.has(api)) throw new Error(`Duplicate version entry: ${api}`)
      table.set(api, {
        since: Number(match[2]),
        removed: match[3] === 'None' ? null : Number(match[3].slice(5, -1)),
      })
    }
  }
  return table
}

// ---------- 注释版本提取 ----------

const SINCE_RE = /@since\s+([\d.]+(?:\(\d+\))?)/u
const REMOVED_RE = /@(?:removed|deleted|delete|removal|deprecated)\s+(?:since\s+)?([\d.]+(?:\(\d+\))?)/u
function apiLevel(text) {
  return Number(text.includes('(') ? text.slice(text.indexOf('(') + 1, -1) : text.split('.')[0])
}

function versionFromComments(comments) {
  const sines = []
  const removeds = []
  for (const comment of comments) {
    const since = comment.match(SINCE_RE)
    if (since) sines.push(apiLevel(since[1]))
    const removed = comment.match(REMOVED_RE)
    if (removed) removeds.push(apiLevel(removed[1]))
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
      case ts.SyntaxKind.VariableStatement: {
        const version = versionFromComments(leadingJSDocComments(sourceFile.text, node))
        for (const declaration of node.declarationList.declarations) {
          const segments = [...scopeNames, declaration.name.text]
          if (segments[0] === rootScope) segments.shift()
          const api = segments.length === 0 ? moduleName : `${moduleName}.${segments.join('.')}`
          addExpected(expected, api, version)
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
      case ts.SyntaxKind.EnumMember:
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
      const segments = [...scopeNames, name]
      if (segments[0] === rootScope) segments.shift()
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
    .replace(/^(\s*(?:export\s+)?(?:declare\s+)?(?:default\s+)?)struct\b/gmu, '$1class')
    .replace(/^(\s*(?:export\s+)?)@interface\b/gmu, '$1class')
  const declared = source.match(/declare\s+module\s+['"]([^'"]+)['"]/u)
  const moduleName = declared?.[1]?.startsWith('@') ? declared[1] : basename(file).replace(/\.d\.(m?ts|ets)$/u, '')
  if (!moduleName?.startsWith('@') || moduleName.startsWith('@kit.')) return

  const sourceFile = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  for (const diagnostic of sourceFile.parseDiagnostics) {
    const pos = sourceFile.getLineAndCharacterOfPosition(diagnostic.start)
    console.error(
      `${file}:${pos.line + 1}:${pos.character + 1}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, ' ')}`,
    )
  }
  parseErrorByModule.set(
    moduleName,
    sourceFile.parseDiagnostics.filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error).length,
  )
  let rootScope = null
  for (const node of sourceFile.statements) {
    if (ts.isExportAssignment(node) && !node.isExportEquals && ts.isIdentifier(node.expression)) {
      rootScope = node.expression.text
    } else if (node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.DefaultKeyword)) {
      rootScope = node.name?.text ?? null
    }
  }
  walk(sourceFile, sourceFile, moduleName, rootScope, [], expected)
}

// ---------- @kit 别名验证 ----------

// Read kit imports/exports through the AST, independently of the generator's regexes.
function collectKitExpected(file, expected) {
  const sourceFile = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true)
  if (sourceFile.parseDiagnostics.length) throw new Error(`Cannot parse kit declaration: ${file}`)
  const moduleName = basename(file).replace(/\.d\.(?:ts|ets)$/u, '')
  const imports = new Map()
  const aliases = []
  for (const node of sourceFile.statements) {
    if (ts.isImportDeclaration(node) && node.importClause) {
      const target = node.moduleSpecifier.text
      if (!target.startsWith('@')) continue
      if (node.importClause.name) imports.set(node.importClause.name.text, target)
      const bindings = node.importClause.namedBindings
      if (bindings && ts.isNamespaceImport(bindings)) imports.set(bindings.name.text, target)
      else if (bindings) {
        for (const element of bindings.elements) {
          imports.set(element.name.text, `${target}.${(element.propertyName ?? element.name).text}`)
        }
      }
    }
  }
  for (const node of sourceFile.statements) {
    if (ts.isExportDeclaration(node) && node.exportClause && ts.isNamedExports(node.exportClause)) {
      for (const element of node.exportClause.elements) {
        const name = (element.propertyName ?? element.name).text
        const target = node.moduleSpecifier ? `${node.moduleSpecifier.text}.${name}` : imports.get(name)
        if (target) aliases.push([`${moduleName}.${element.name.text}`, target])
      }
    }
  }
  let checked = 0
  for (const [alias, target] of aliases) {
    for (const [api, version] of [...expected]) {
      if (api === target || api.startsWith(`${target}.`)) {
        addExpected(expected, `${alias}${api.slice(target.length)}`, version)
        checked += 1
      }
    }
  }
  return checked
}

// ---------- 主流程 ----------

try {
  const { sourcePath, outputFile } = parseArgs(process.argv.slice(2))
  if (!existsSync(sourcePath)) throw new Error(`Source path does not exist: ${sourcePath}`)

  const table = parseVersionTable(outputFile)
  if (table.size === 0) throw new Error(`No version entries found in ${outputFile}`)

  const sourcePaths = collectSourcePaths(sourcePath)
  const apiPaths = sourcePaths.filter((path) => basename(path) !== 'kits')

  const expected = new Map()
  const parseErrorByModule = new Map()
  const files = []
  for (const path of apiPaths) collectFiles(path, files)
  for (const file of files) collectExpectedFromDeclarationFile(file, expected, parseErrorByModule)

  let kitChecked = 0
  const kitFiles = []
  for (const path of sourcePaths.filter((path) => basename(path) === 'kits')) collectFiles(path, kitFiles)
  for (const file of files.filter((file) => basename(file).startsWith('@kit.'))) kitFiles.push(file)
  for (const file of [...new Set(kitFiles)].sort()) kitChecked += collectKitExpected(file, expected)

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

  const extras = [...table.keys()].filter((api) => !expected.has(api))
  const parseErrors = [...parseErrorByModule.values()].reduce((total, count) => total + count, 0)

  console.log(
    `Verified ${expected.size} declarations from ${files.length} SDK files against ${table.size} table entries.`,
  )
  console.log(`@kit aliases checked: ${kitChecked}.`)
  console.log(`\n=== 版本不符: ${mismatches.length} ===`)
  for (const mismatch of mismatches.slice(0, 120)) console.log(`  ${mismatch}`)
  console.log(`\n=== 表中缺失: ${missing.length} ===`)
  for (const api of missing.slice(0, 60)) console.log(`  ${api}`)
  console.log(`\n=== 表中多余: ${extras.length} ===`)
  for (const api of extras.slice(0, 60)) console.log(`  ${api}`)
  console.log(`\n=== 解析错误: ${parseErrors} ===`)

  const failed = mismatches.length + missing.length + extras.length + parseErrors
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
