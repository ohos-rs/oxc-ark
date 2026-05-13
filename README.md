# OXK(Oxc-Ark)

An ArkTS/ArkUI tool based on OXC.

## Goal

Provide fast ArkTS/ArkUI parser, formatter, and lint tooling based on OXC and oxlint.

## Install

### Install from cargo

Install with cargo.

```bash
cargo install oxk --git https://github.com/ohos-rs/oxc-ark.git
```

### Install from npm

Install with npm

```bash
npm install @ohos-rs/oxk -g
```

## Usage

### Format

```bash
# Path support regex
oxk format xx.ets
```

### Lint

```bash
oxk lint src --threads 1
oxk lint src/index.ets --format json
```

`oxk lint` embeds oxlint instead of shelling out to an external `oxlint` binary.
It supports oxlint-compatible rules and config files. The npm package also loads
the oxlint JavaScript runtime, so `.oxlintrc.json`, `.oxlintrc.jsonc`,
`oxlint.config.ts`, built-in plugins, and JS plugins work through the npm CLI.

ArkTS migration rules are available through the built-in `arkts` plugin. The
plugin only registers rules; each rule must be enabled explicitly.

```jsonc
{
  "plugins": ["arkts"],
  "rules": {
    "arkts/no-symbol": "error",
    "arkts/no-var": "error",
    "arkts/system-api-version": [
      "error",
      {
        "minApiVersion": 11,
      },
    ],
  },
}
```

ArkTS rules only run for `.ets` files. The other extensions are not supported.
`arkts/system-api-version` checks imported system APIs against the configured
minimum supported API version. The minimum version must be within the API
availability range: `since <= minApiVersion < removed/deprecated`. If
`minApiVersion` is omitted, oxk tries to read `minAPIVersion` from common
OpenHarmony project files such as `AppScope/app.json5`. Unknown APIs are ignored
unless they are listed in `apis` or `apiVersionFile`.
The built-in table is used by default; `apis` and `apiVersionFile` are only for
local overrides or additional API metadata.
`@kit.*` imports are supported through SDK kit aliases, for example
`import { router } from "@kit.ArkUI"`.

Refresh the built-in system API version table from a JSON/JSONC mapping or an
OpenHarmony SDK declaration directory:

```bash
pnpm run sync:arkts-api-versions -- /path/to/ets/api
pnpm run sync:arkts-api-versions -- /path/to/system-api-versions.jsonc
```

The generated table is written to `crates/lint/src/arkts/system_api_versions.rs`
by default, keeping generated data separate from rule implementation code. When
the source path is `ets/api`, sibling `ets/kits` is included automatically.

### Local Development

```bash
pnpm --filter @ohos-rs/oxk run build:debug
pnpm --filter @ohos-rs/oxk test
pnpm --filter @ohos-rs/oxk run build --target wasm32-wasip1-threads
pnpm --filter @ohos-rs/oxk run test:wasi

cargo check -p lint -p oxk
cargo check -p oxk-napi
cargo test -p lint -p oxk
```

## Credits

Thanks for the following projects:

- [oxc](https://github.com/oxc-project/oxc)
- [napi-rs](https://github.com/napi-rs/napi-rs)

## License

[MIT](./LICENSE)
