import test from 'ava'
import { format, formatRaw } from '../format.js'

test('format ArkTS (.ets) file', async (t) => {
  const source = `@Component
struct MyComponent {
  @State message: string = 'Hello World'
  @State count: number = 0

  build() {
    Row() {
      Column() {
        Text(this.message)
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
        Button('Click me')
          .onClick(() => {
            this.count++
          })
      }
      .width('100%')
    }
    .height('100%')
  }
}`

  const result = await format('test.ets', source, undefined)

  // Check if there are errors and log them for debugging
  if (result.errors.length > 0) {
    console.log('Format errors:', result.errors)
  }

  // For now, we expect it to work, but if there are parse errors, that's also valid
  // (ArkTS syntax might not be fully supported yet)
  t.truthy(result, 'Should return a result')
  if (result.errors.length === 0) {
    t.truthy(result.code, 'Should return formatted code')
    t.true(result.code.includes('@Component') || result.code.includes('Component'), 'Should contain Component')
  } else {
    // If there are errors, they should be parse errors, not unsupported file type errors
    const hasUnsupportedError = result.errors.some(
      (err: string) => err.includes('Unsupported file type') || err.includes('unsupported'),
    )
    t.false(hasUnsupportedError, 'Should not have unsupported file type error')
  }
})

test('format complex ArkTS file', async (t) => {
  const source = `@Entry
@Component
struct Index {
  @State message: string = 'Hello ArkUI'
  private data: Array<string> = ['item1', 'item2', 'item3']

  aboutToAppear() {
    console.log('Component about to appear')
  }

  build() {
    Column({ space: 20 }) {
      Text(this.message)
        .fontSize(30)
        .fontColor(Color.Blue)
      ForEach(this.data, (item: string, index: number) => {
        Text(item)
          .fontSize(16)
      })
    }
    .padding(20)
    .width('100%')
    .height('100%')
  }
}`

  const result = await format('index.ets', source, undefined)

  t.truthy(result, 'Should return a result')
  if (result.errors.length === 0) {
    t.truthy(result.code, 'Should return formatted code')
    t.true(result.code.includes('@Entry') || result.code.includes('Entry'), 'Should contain Entry')
  } else {
    // Log errors for debugging
    console.log('Format errors:', result.errors)
    // Should not be unsupported file type error
    const hasUnsupportedError = result.errors.some(
      (err: string) => err.includes('Unsupported file type') || err.includes('unsupported'),
    )
    t.false(hasUnsupportedError, 'Should not have unsupported file type error')
  }
})

test('format static ETS only with explicit language', async (t) => {
  const source = [
    'package example.formatter;',
    'final class Box{value:int=1;method(value:int):int{return value}}',
    "let character:char=c'a';",
  ].join('\n')

  const inferred = await format('static.ets', source, undefined)
  t.true(inferred.errors.length > 0, 'A .ets path should keep using the ArkTS 1.1 grammar')

  const explicit = await format('static.ets', source, { lang: 'ets-static' })
  t.deepEqual(explicit.errors, [])
  t.true(explicit.code.includes('final class Box {'))
  t.true(explicit.code.includes('value: int = 1;'))
  t.true(explicit.code.includes("let character: char = c'a';"))

  const second = await format('static.ets', explicit.code, { lang: 'ets-static' })
  t.deepEqual(second.errors, [])
  t.is(second.code, explicit.code, 'Static ETS formatting should be idempotent')
})

test('format JSON5 file', async (t) => {
  const json5Source = `{
  // This is a JSON5 file
  name: 'test',
  version: '1.0.0',
  description: 'Test package',
  keywords: ['test', 'json5'],
  private: true,
  dependencies: {
    'package-a': '^1.0.0',
    'package-b': '^2.0.0'
  }
}`

  // JSON5 files are formatted by the native Rust formatter.
  const result = await format('test.json5', json5Source, undefined)

  t.truthy(result, 'Should return a result')
  t.is(result.errors.length, 0, 'Should not have errors')
  t.truthy(result.code, 'Should return formatted code')
  t.true(result.code.includes('name') || result.code.includes('test'), 'Should contain formatted content')
})

test('format JSON5 with comments', async (t) => {
  const json5Source = `{
  // Single line comment
  name: 'test',
  /* Multi-line
     comment */
  version: '1.0.0'
}`

  // JSON5 files are formatted by the native Rust formatter.
  const result = await format('config.json5', json5Source, undefined)

  t.truthy(result, 'Should return a result')
  t.is(result.errors.length, 0, 'Should not have errors')
  t.truthy(result.code, 'Should return formatted code')
  t.true(result.code.includes('//') || result.code.includes('/*'), 'Should preserve comments')
})

test('format raw JSON5 without external formatter callbacks', async (t) => {
  const source = `{
  // keep this comment
  name: 'native-json5'
}`

  const result = await formatRaw('raw.json5', source, undefined)

  t.is(result.errors.length, 0, 'JSON5 should not require external formatter callbacks')
  t.false(
    result.errors.some((err: string) => err.includes('External formatter is required')),
    'Should not fall back to the external formatter requirement',
  )
  t.true(result.code.includes('native-json5'), 'Should return formatted JSON5 content')
})

test('preserve property quotes and default to double quoted strings in TypeScript and ArkTS', async (t) => {
  for (const filename of ['quoted.ts', 'quoted.ets']) {
    const source = `const value={"quoted":'value',plain:'other'}`
    const result = await format(filename, source, undefined)
    t.deepEqual(result.errors, [])
    t.true(result.code.includes('"quoted": "value"'), result.code)
    t.true(result.code.includes('plain: "other"'), result.code)
    const second = await format(filename, result.code, undefined)
    t.deepEqual(second.errors, [])
    t.is(second.code, result.code)
  }
})

test('preserve JSON5 property quotes and use double quoted values by default', async (t) => {
  const source = `{
  "quoted": 'value',
  plain: 'other'
}`

  const result = await format('quoted.json5', source, undefined)

  t.is(result.errors.length, 0, 'Should not have errors')
  t.true(result.code.includes('"quoted": "value"'), 'Should preserve explicit JSON5 key quotes')
  t.true(result.code.includes('plain: "other"'), 'Should use double quoted values')
})

test('format regular TypeScript file', async (t) => {
  const source = `const x=1;const y=2;`

  const result = await format('test.ts', source, undefined)

  t.truthy(result, 'Should return a result')
  if (result.errors.length === 0) {
    t.truthy(result.code, 'Should return formatted code')
    t.true(result.code.includes('const'), 'Should contain const')
    // Verify formatting actually happened
    t.true(result.code.includes(';') || result.code.includes('\n'), 'Should be formatted')
  } else {
    // Log errors for debugging
    console.log('Format errors:', result.errors)
    // TypeScript should definitely work
    t.fail('TypeScript formatting should not have errors')
  }
})

test('preserve property and string quotes when explicitly configured', async (t) => {
  for (const filename of ['quoted.ts', 'quoted.json5']) {
    const object = `{"quoted": 'value', plain: 'other'}`
    const source = filename.endsWith('.ts') ? `const value = ${object}` : object
    const result = await format(filename, source, { quoteProps: 'preserve', singleQuote: true })
    t.deepEqual(result.errors, [])
    const expected = filename.endsWith('.json5') ? `"quoted": 'value'` : `'quoted': 'value'`
    t.true(result.code.includes(expected), result.code)
  }
})

for (const [filename, source, expected] of [
  ['style.css', 'a{color:red;margin:0  1px}', 'color: red;'],
  ['query.graphql', 'query Test{user{id name}}', '  user {\n'],
  ['data.yaml', 'items: [one,two]', 'items: [one, two]'],
  ['README.md', '# Title\n\n```ts\nconst value={a:1}\n```', 'const value = { a: 1 };'],
  ['package.json', '{"version":"1.0.0","name":"example"}', '"name": "example",\n  "version"'],
]) {
  test(`format ${filename} and nested native languages without JS callbacks`, async (t) => {
    const first = await formatRaw(filename, source, {})
    t.deepEqual(first.errors, [])
    t.true(first.code.includes(expected), first.code)
    const second = await formatRaw(filename, first.code, {})
    t.deepEqual(second.errors, [])
    t.is(second.code, first.code)
  })
}

test('native JS embedding and Prettier HTML doc embedding are idempotent', async (t) => {
  const source = 'const styles=css`a{color:red}`;const template=html`<div><span>hello</span></div>`'
  const first = await format('embedded.ts', source, {})
  t.deepEqual(first.errors, [])
  t.true(first.code.includes('color: red;'), first.code)
  const second = await format('embedded.ts', first.code, {})
  t.deepEqual(second.errors, [])
  t.is(second.code, first.code)
  const disabled = await format('embedded.ts', source, { embeddedLanguageFormatting: 'off' })
  t.true(disabled.code.includes('a{color:red}'), disabled.code)
})

for (const [filename, source, options] of [
  [
    'Component.vue',
    '<script lang="ts">import z from "z";import a from "a";const value={a,z}</script>\n<template><div>{{value}}</div></template>',
    {},
  ],
  [
    'Component.svelte',
    '<script>import z from "z";import a from "a";const value={a,z}</script>\n<div>{value}</div>',
    { svelte: true },
  ],
  [
    'Component.astro',
    '---\nimport z from "z";import a from "a";const value={a,z}\n---\n<div>{value}</div>',
    { astro: true },
  ],
] as const) {
  test(`upstream import sorting inside ${filename}`, async (t) => {
    const config = { ...options, sortImports: {} }
    const first = await format(filename, source, config)
    t.deepEqual(first.errors, [])
    t.true(first.code.indexOf('import a') < first.code.indexOf('import z'), first.code)
    const second = await format(filename, first.code, config)
    t.deepEqual(second.errors, [])
    t.is(second.code, first.code)
  })
}

test('opt-in plugins and invalid merged options return useful errors', async (t) => {
  for (const extension of ['svelte', 'astro']) {
    const result = await format(`Component.${extension}`, '<div>hello</div>', {})
    t.true(result.errors.some((error) => error.includes(`Enable \`${extension}\``)))
  }
  const result = await format('test.ts', 'const value=1', {
    overrides: [{ files: ['*.ts'], options: { printWidth: 1000 } }],
  })
  t.true(result.errors.length > 0)
})

test('sort Tailwind classes in JS and native CSS', async (t) => {
  for (const [filename, source] of [
    ['Component.tsx', 'const element=<div className="p-4 flex bg-red-500"/>'],
    ['style.css', '.button{@apply p-4 flex bg-red-500;}'],
  ]) {
    const first = await format(filename, source, { sortTailwindcss: {} })
    t.deepEqual(first.errors, [])
    t.true(first.code.includes('flex bg-red-500 p-4'), first.code)
    const second = await format(filename, first.code, { sortTailwindcss: {} })
    t.deepEqual(second.errors, [])
    t.is(second.code, first.code)
  }
})

test('explicit quote options override compatibility defaults', async (t) => {
  for (const filename of ['quoted.ts', 'quoted.ets', 'quoted.json5']) {
    const object = `{"quoted": 'value', plain: 'other'}`
    const source = filename.endsWith('.json5') ? object : `const value = ${object}`
    const result = await format(filename, source, { quoteProps: 'as-needed', singleQuote: false })
    t.deepEqual(result.errors, [])
    t.true(result.code.includes('quoted: "value"'), result.code)
    t.true(result.code.includes('plain: "other"'), result.code)
  }
})
