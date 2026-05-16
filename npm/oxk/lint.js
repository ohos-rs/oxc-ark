const { mkdtempSync, readFileSync, rmSync, writeFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { basename, join, resolve } = require('node:path')
const { lint: lintNative, lintSync, lintWithPlugins } = require('./index.js')

let pluginRuntime = null
let jsConfigRuntime = null

function getPluginRuntime() {
  if (!pluginRuntime) pluginRuntime = require('./oxlint-runtime/plugins.cjs')
  return pluginRuntime
}

function getJsConfigRuntime() {
  if (!jsConfigRuntime) jsConfigRuntime = require('./oxlint-runtime/js_config.cjs')
  return jsConfigRuntime
}

function getErrorMessage(error) {
  return error instanceof Error ? error.message : String(error)
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

function findConfigPath(args, cwd) {
  for (let index = 0; index < args.length; index += 1) {
    const token = args[index]
    if (token === '--config' || token === '-c') {
      const value = args[index + 1]
      return value ? resolve(cwd, value) : null
    }
    if (token.startsWith('--config=')) {
      return resolve(cwd, token.slice('--config='.length))
    }
  }

  for (const name of ['.oxlintrc.json', '.oxlintrc.jsonc', 'oxlint.config.ts']) {
    const candidate = join(cwd, name)
    try {
      readFileSync(candidate)
      return candidate
    } catch {}
  }

  return null
}

function isJsonConfig(configPath) {
  return configPath.endsWith('.json') || configPath.endsWith('.jsonc')
}

async function loadJsConfig(configPath) {
  const response = JSON.parse(await getJsConfigRuntime().loadJsConfigs([configPath]))

  if (Array.isArray(response.Success)) {
    const [result] = response.Success
    if (!result || result.config == null) {
      throw new Error('Configuration file must have a default export that is an object.')
    }
    return result.config
  }

  if (Array.isArray(response.Failures)) {
    const first = response.Failures[0]
    throw new Error(first ? `${first.path}: ${first.error}` : 'Failed to load JavaScript configuration.')
  }

  throw new Error(response.Error ?? 'Failed to load JavaScript configuration.')
}

async function loadConfig(configPath) {
  if (isJsonConfig(configPath)) {
    return JSON.parse(stripJsonComments(readFileSync(configPath, 'utf8')))
  }

  return loadJsConfig(configPath)
}

function hasJsPlugins(config) {
  if (!config || typeof config !== 'object' || Array.isArray(config)) return false
  if (Array.isArray(config.jsPlugins) && config.jsPlugins.length > 0) return true
  return Array.isArray(config.overrides) && config.overrides.some(hasJsPlugins)
}

async function prepareLintConfig(args) {
  const cwd = process.cwd()
  const configPath = findConfigPath(args, cwd)
  if (!configPath) return { args, cleanup: () => {}, needsJsPlugins: false }

  let config
  try {
    config = await loadConfig(configPath)
  } catch (error) {
    throw new Error(`Failed to load oxlint configuration ${configPath}: ${getErrorMessage(error)}`)
  }

  let shouldRewrite = !isJsonConfig(configPath)
  let tempDir = null
  const ensureTempDir = () => {
    if (!tempDir) tempDir = mkdtempSync(join(tmpdir(), 'oxk-lint-config-'))
    return tempDir
  }
  const needsJsPlugins = hasJsPlugins(config)

  if (!shouldRewrite) {
    return { args, cleanup: () => {}, needsJsPlugins }
  }

  tempDir = ensureTempDir()
  const tempConfigPath = join(
    tempDir,
    isJsonConfig(configPath) && basename(configPath).endsWith('.jsonc') ? 'oxlintrc.jsonc' : 'oxlintrc.json',
  )
  writeFileSync(tempConfigPath, JSON.stringify(config, null, 2), 'utf8')

  const nextArgs = []
  let replaced = false
  for (let index = 0; index < args.length; index += 1) {
    const token = args[index]
    if (token === '--config' || token === '-c') {
      nextArgs.push(token, tempConfigPath)
      index += 1
      replaced = true
    } else if (token.startsWith('--config=')) {
      nextArgs.push(`--config=${tempConfigPath}`)
      replaced = true
    } else {
      nextArgs.push(token)
    }
  }
  if (!replaced) nextArgs.push('--config', tempConfigPath)

  return {
    args: nextArgs,
    cleanup: () => rmSync(tempDir, { recursive: true, force: true }),
    needsJsPlugins,
  }
}

async function loadPluginWrapper(pluginUrl, pluginName, pluginNameIsAlias, workspaceUri) {
  return getPluginRuntime().loadPlugin(pluginUrl, pluginName ?? null, pluginNameIsAlias, workspaceUri ?? null)
}

function setupRuleConfigsWrapper(optionsJson) {
  return getPluginRuntime().setupRuleConfigs(optionsJson)
}

async function lintFileWrapper(
  filePath,
  bufferId,
  buffer,
  ruleIds,
  optionIds,
  settingsJson,
  globalsJson,
  workspaceUri,
) {
  return getPluginRuntime().lintFile(
    filePath,
    bufferId,
    buffer ?? null,
    ruleIds,
    optionIds,
    settingsJson,
    globalsJson,
    workspaceUri ?? null,
  )
}

function createWorkspaceWrapper() {
  return Promise.resolve()
}

function destroyWorkspaceWrapper() {}

function loadJsConfigsWrapper(paths) {
  return getJsConfigRuntime().loadJsConfigs(paths)
}

async function withStandardOxlintEnv(run) {
  const vpVersion = process.env.VP_VERSION
  delete process.env.VP_VERSION
  try {
    return await run()
  } finally {
    if (vpVersion === undefined) {
      delete process.env.VP_VERSION
    } else {
      process.env.VP_VERSION = vpVersion
    }
  }
}

async function lint(args) {
  return withStandardOxlintEnv(async () => {
    if (
      args.length > 0 &&
      (args[0] === '--print-config-schema' ||
        args[0] === '--schema-json' ||
        args[0] === '--write-config-schema')
    ) {
      return typeof lintSync === 'function' ? lintSync(args) : lintNative(args)
    }

    const prepared = await prepareLintConfig(args)
    try {
      if (!prepared.needsJsPlugins) {
        return typeof lintSync === 'function' ? lintSync(prepared.args) : lintNative(prepared.args)
      }

      return await lintWithPlugins(
        prepared.args,
        loadPluginWrapper,
        setupRuleConfigsWrapper,
        lintFileWrapper,
        createWorkspaceWrapper,
        destroyWorkspaceWrapper,
        loadJsConfigsWrapper,
      )
    } finally {
      prepared.cleanup()
    }
  })
}

module.exports = { lint }
