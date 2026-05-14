# Built-in Lint

`oxk lint` embeds oxlint through `oxlint::CliRunner`; it does not shell out to an
external `oxlint` binary. The Rust side also registers the built-in ArkTS
migration rules as a virtual external plugin named `arkts`.

## Configuration

Registering the `arkts` plugin only makes its rules available. It does not enable
any rule by default. Enable each rule explicitly in `rules`.

```jsonc
{
  "plugins": ["arkts"],
  "rules": {
    "arkts/no-symbol": "error",
    "arkts/no-var": "error",
    "arkts/no-any-unknown": "warn",
    "arkts/system-api-version": [
      "error",
      {
        "minApiVersion": 11,
      },
    ],
  },
}
```

ArkTS rules run only for `.ets` files. The `.arkts` extension is not supported.
Diagnostics are reported as `arkts(<rule>)`, for example `arkts(no-symbol)`.

Supported config files:

- npm CLI/NAPI: `.oxlintrc.json`, `.oxlintrc.jsonc`, `oxlint.config.ts`, oxlint
  built-in plugin config, JS plugins, settings, and globals.
- cargo CLI: `.oxlintrc.json` and `.oxlintrc.jsonc`. It keeps a pure Rust runner
  and does not load JavaScript config files or JS plugins.

Rules that require type or cross-file information are implemented
conservatively: they are registered as normal rules, but only report when the
lint runner can determine the violation without a high false-positive risk.

## System API Versions

`arkts/system-api-version` checks imported system APIs against the configured
minimum supported API version. The configured minimum version must be within the
API availability range:

```text
since <= minApiVersion < removed/deprecated
```

If `minApiVersion` is omitted, oxk scans common OpenHarmony project files such as
`AppScope/app.json5`, `app.json5`, `src/main/module.json5`, and
`entry/src/main/module.json5` for `minAPIVersion` or `minApiVersion`.

The built-in system API table is used by default. `@ohos.*`, `@system.*`,
`@hms.*`, and `@kit.*` imports are supported. `@kit.*` entries are generated from
SDK kit aliases, so code like this is checked against the underlying system API
metadata:

```ts
import { router } from '@kit.ArkUI'

router.back()
```

Optional rule options:

- `minApiVersion` / `minVersion`: project minimum API version.
- `apis` / `apiVersions`: local overrides or additions. Values can be integers
  or objects such as `{ "since": 9, "removed": 11 }`.
- `deprecatedSince` / `deprecatedVersion`: accepted in local override objects
  and treated as the first version where the API should no longer be used.
- `apiVersionFile` / `apisFile`: JSON or JSONC file with the same API mapping,
  either at the top level or under `apis`.

Example override file:

```jsonc
{
  "apis": {
    "@ohos.example.newApi": 12,
    "@ohos.example.legacyApi": {
      "since": 9,
      "deprecatedSince": 11,
    },
  },
}
```

Refresh the built-in system API version table from a JSON/JSONC mapping or an
OpenHarmony SDK declaration directory:

```bash
pnpm run sync:arkts-api-versions -- /path/to/ets/api
pnpm run sync:arkts-api-versions -- /path/to/system-api-versions.jsonc
```

The generated table is written to `crates/lint/src/arkts/system_api_versions.rs`
by default. When the source path is `ets/api`, sibling `ets/kits` is included
automatically so `@kit.*` aliases are generated together with the base APIs.

## Rules

| Rule                                  | Code      | Description                                                                                  |
| ------------------------------------- | --------- | -------------------------------------------------------------------------------------------- |
| `arkts/identifiers-as-prop-names`     | 10605001  | ArkTS requires object property names to be valid identifiers.                                |
| `arkts/no-symbol`                     | 10605002  | ArkTS does not support Symbol() or the symbol type.                                          |
| `arkts/no-private-identifiers`        | 10605003  | ArkTS does not support private identifiers starting with #. Use the private keyword instead. |
| `arkts/unique-names`                  | 10605004  | ArkTS requires unique names for types, namespaces, and values.                               |
| `arkts/no-var`                        | 10605005  | ArkTS does not support var. Use let or const instead.                                        |
| `arkts/no-any-unknown`                | 10605008  | ArkTS does not support any or unknown. Specify an explicit type.                             |
| `arkts/no-call-signatures`            | 10605014  | ArkTS does not support call signatures in object types.                                      |
| `arkts/no-ctor-signatures-type`       | 10605015  | ArkTS does not support constructor signatures in object types.                               |
| `arkts/no-multiple-static-blocks`     | 10605016  | ArkTS supports only one static block per class.                                              |
| `arkts/no-indexed-signatures`         | 10605017  | ArkTS does not support index signatures.                                                     |
| `arkts/no-intersection-types`         | 10605019  | ArkTS does not support intersection types. Use inheritance instead.                          |
| `arkts/no-typing-with-this`           | 10605021  | ArkTS does not support this in type positions.                                               |
| `arkts/no-conditional-types`          | 10605022  | ArkTS does not support conditional types or infer types.                                     |
| `arkts/no-ctor-prop-decls`            | 10605025  | ArkTS does not support declaring properties in constructor parameters.                       |
| `arkts/no-ctor-signatures-iface`      | 10605027  | ArkTS does not support constructor signatures in interfaces.                                 |
| `arkts/no-aliases-by-index`           | 10605028  | ArkTS does not support indexed access types.                                                 |
| `arkts/no-props-by-index`             | 10605029  | ArkTS does not support property access by non-numeric indexes.                               |
| `arkts/no-structural-typing`          | 10605030  | ArkTS does not support structural typing.                                                    |
| `arkts/no-inferred-generic-params`    | 10605034  | ArkTS limits type inference for generic function calls.                                      |
| `arkts/no-untyped-obj-literals`       | 10605038  | ArkTS requires object literals to have inferrable or explicit types.                         |
| `arkts/no-obj-literals-as-types`      | 10605040  | ArkTS does not support object literal types.                                                 |
| `arkts/no-noninferrable-arr-literals` | 10605043  | ArkTS requires array literal element types to be inferrable.                                 |
| `arkts/no-func-expressions`           | 10605046  | ArkTS does not support function expressions. Use arrow functions instead.                    |
| `arkts/no-class-literals`             | 10605050  | ArkTS does not support class expressions.                                                    |
| `arkts/implements-only-iface`         | 10605051  | ArkTS classes may implement interfaces only.                                                 |
| `arkts/no-method-reassignment`        | 10605052  | ArkTS does not support method reassignment.                                                  |
| `arkts/as-casts`                      | 10605053  | ArkTS supports as casts only.                                                                |
| `arkts/no-jsx`                        | 10605054  | ArkTS does not support JSX.                                                                  |
| `arkts/no-polymorphic-unops`          | 10605055  | ArkTS restricts unary operator semantics.                                                    |
| `arkts/no-delete`                     | 10605059  | ArkTS does not support the delete operator.                                                  |
| `arkts/no-type-query`                 | 10605060  | ArkTS does not support typeof in type positions.                                             |
| `arkts/instanceof-ref-types`          | 10605065  | ArkTS restricts instanceof to reference types.                                               |
| `arkts/no-in`                         | 10605066  | ArkTS does not support the in operator.                                                      |
| `arkts/no-destruct-assignment`        | 10605069  | ArkTS does not support destructuring assignment.                                             |
| `arkts/no-comma-outside-loops`        | 10605071  | ArkTS restricts comma expressions outside loops.                                             |
| `arkts/no-destruct-decls`             | 10605074  | ArkTS does not support destructuring declarations.                                           |
| `arkts/no-types-in-catch`             | 10605079  | ArkTS does not support type annotations in catch clauses.                                    |
| `arkts/no-for-in`                     | 10605080  | ArkTS does not support for-in statements.                                                    |
| `arkts/no-mapped-types`               | 10605083  | ArkTS does not support mapped types.                                                         |
| `arkts/no-with`                       | 10605084  | ArkTS does not support with statements.                                                      |
| `arkts/limited-throw`                 | 10605087  | ArkTS restricts thrown values to Error-derived objects.                                      |
| `arkts/no-implicit-return-types`      | 10605090  | ArkTS requires explicit return types for functions and methods.                              |
| `arkts/no-destruct-params`            | 10605091  | ArkTS does not support destructuring parameters.                                             |
| `arkts/no-nested-funcs`               | 10605092  | ArkTS does not support nested function declarations.                                         |
| `arkts/no-standalone-this`            | 10605093  | ArkTS does not support standalone this.                                                      |
| `arkts/no-generators`                 | 10605094  | ArkTS does not support generator functions.                                                  |
| `arkts/no-is`                         | 10605096  | ArkTS does not support is type predicates.                                                   |
| `arkts/no-spread`                     | 10605099  | ArkTS restricts spread syntax.                                                               |
| `arkts/no-extend-same-prop`           | 106050102 | ArkTS interfaces cannot extend interfaces with duplicate properties.                         |
| `arkts/no-decl-merging`               | 10605103  | ArkTS does not support declaration merging.                                                  |
| `arkts/extends-only-class`            | 10605104  | ArkTS classes can extend classes only.                                                       |
| `arkts/no-ctor-signatures-funcs`      | 10605106  | ArkTS does not support constructor function types.                                           |
| `arkts/no-enum-mixed-types`           | 10605111  | ArkTS enum members must be initialized with same-type compile-time expressions.              |
| `arkts/no-enum-merging`               | 10605113  | ArkTS does not support enum declaration merging.                                             |
| `arkts/no-ns-as-obj`                  | 10605114  | ArkTS does not support using namespaces as objects.                                          |
| `arkts/no-ns-statements`              | 10605116  | ArkTS does not support non-declaration statements in namespaces.                             |
| `arkts/no-require`                    | 10605121  | ArkTS does not support require or import assignment.                                         |
| `arkts/no-export-assignment`          | 10605126  | ArkTS does not support export = syntax.                                                      |
| `arkts/no-ambient-decls`              | 10605128  | ArkTS does not support ambient module declarations.                                          |
| `arkts/no-module-wildcards`           | 10605129  | ArkTS does not support wildcards in module names.                                            |
| `arkts/no-umd`                        | 10605130  | ArkTS does not support UMD declarations.                                                     |
| `arkts/no-new-target`                 | 10605132  | ArkTS does not support new.target.                                                           |
| `arkts/no-definite-assignment`        | 10605134  | ArkTS does not support definite assignment assertions.                                       |
| `arkts/no-prototype-assignment`       | 10605136  | ArkTS does not support prototype assignment.                                                 |
| `arkts/no-globalthis`                 | 10605137  | ArkTS does not support globalThis.                                                           |
| `arkts/no-utility-types`              | 10605138  | ArkTS supports only Partial, Required, Readonly, and Record utility types.                   |
| `arkts/no-func-props`                 | 10605139  | ArkTS does not support declaring properties on functions.                                    |
| `arkts/no-func-apply-call`            | 10605152  | ArkTS does not support Function.apply or Function.call.                                      |
| `arkts/no-func-bind`                  | 10605140  | ArkTS does not support Function.bind.                                                        |
| `arkts/no-as-const`                   | 10605142  | ArkTS does not support as const assertions.                                                  |
| `arkts/no-import-assertions`          | 10605143  | ArkTS does not support import assertions.                                                    |
| `arkts/limited-stdlib`                | 10605144  | ArkTS restricts dynamic standard library APIs.                                               |
| `arkts/strict-typing-required`        | 10605146  | ArkTS does not allow disabling type checking with @ts-ignore or @ts-nocheck.                 |
| `arkts/no-ts-deps`                    | 10605147  | TypeScript and JavaScript files cannot import ETS source files.                              |
| `arkts/no-classes-as-obj`             | -         | ArkTS does not support using classes as objects.                                             |
| `arkts/no-misplaced-imports`          | -         | ArkTS requires import declarations to appear before other statements.                        |
| `arkts/limited-esobj`                 | -         | ArkTS restricts ESObject usage.                                                              |
| `arkts/system-api-version`            | -         | ArkTS system API usage must be supported by the configured minimum API version.              |
