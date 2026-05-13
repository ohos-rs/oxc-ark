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
  },
}
```

ArkTS rules only run for `.ets` files. The other extensions are not supported.

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
