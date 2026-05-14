# `@ohos-rs/oxk`

ArkTS/ArkUI parser, formatter, and lint tooling based on OXC and oxlint.

## Install

```bash
npm install @ohos-rs/oxk
```

For CLI usage, install it in your project and run it through your package
manager, or install it globally:

```bash
npm install -g @ohos-rs/oxk
oxk --help
```

The npm package requires Node.js `^20.19.0 || >=22.18.0`.

## Format

```bash
oxk format src/index.ets
oxk format "src/**/*.{ts,ets}"
```

Formatter config is loaded from `.oxfmtrc.json` or `.oxfmtrc.jsonc`.

## Lint

```bash
oxk lint src --threads 1
oxk lint src/index.ets --format json
oxk lint --config .oxlintrc.jsonc "src/**/*.ets"
```

`oxk lint` embeds `oxlint::CliRunner`; it does not shell out to an external
`oxlint` binary. The npm CLI includes the oxlint JavaScript runtime for:

- `.oxlintrc.json` and `.oxlintrc.jsonc`
- `oxlint.config.ts`
- oxlint built-in plugin configuration, such as `plugins: ["react"]`
- JavaScript plugins configured with `jsPlugins`
- plugin settings and globals

The cargo CLI keeps a pure Rust runner. Use JSON or JSONC config files there;
the JavaScript runtime and `oxlint.config.ts` are npm-only.

## ArkTS Rules

ArkTS migration rules are built in as the virtual `arkts` plugin. See
[Built-in Lint](../../crates/lint/README.md) for the rule list, configuration
examples, `arkts/system-api-version`, and the SDK sync workflow.

Example:

```ts
// index.ets
const key = Symbol('id')
let marker: symbol
```

```bash
oxk lint index.ets --threads 1 --format json
```

## JavaScript API

Use the formatter wrapper:

```js
const { format } = require('@ohos-rs/oxk/format')
```

Use the lint wrapper with oxlint-compatible CLI arguments:

```js
const { lint } = require('@ohos-rs/oxk/lint')

const ok = await lint(['src/index.ets', '--threads', '1'])
process.exitCode = ok ? 0 : 1
```

The native module is also available from the package root:

```js
const oxk = require('@ohos-rs/oxk')
```

For linting from JavaScript, prefer `@ohos-rs/oxk/lint`; it wires the oxlint JS
runtime callbacks for plugins and JavaScript config files.

## WASI

Build the WASI artifact locally:

```bash
pnpm build --target wasm32-wasip1-threads
pnpm run test:wasi
```

`test:wasi` forces `NAPI_RS_FORCE_WASI=1` and verifies the generated WASI
binding can load and execute `parse` and `format`. Linting is intentionally not
available from the WASI build because the oxlint runner and JavaScript plugin
runtime are native-only; use the native npm package or cargo CLI for linting.

## Local Development

Build the local NAPI binary before running npm CLI tests:

```bash
pnpm --filter @ohos-rs/oxk run build:debug
pnpm --filter @ohos-rs/oxk test
pnpm --filter @ohos-rs/oxk run build --target wasm32-wasip1-threads
pnpm --filter @ohos-rs/oxk run test:wasi
```

Update the bundled oxlint JavaScript runtime after changing the upstream source
or dependency versions:

```bash
pnpm --filter @ohos-rs/oxk run build:oxlint-runtime
```

Run the Rust checks:

```bash
cargo check -p lint -p oxk
cargo check -p oxk-napi
cargo test -p lint -p oxk
git diff --check
```
