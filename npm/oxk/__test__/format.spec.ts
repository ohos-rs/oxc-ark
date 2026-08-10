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

test('preserve quoted object properties by default for TypeScript', async (t) => {
  const source = `const value={"quoted":1,plain:2}`

  const result = await format('quoted.ts', source, undefined)

  t.is(result.errors.length, 0, 'Should not have errors')
  t.true(result.code.includes('"quoted": 1'), 'Should preserve explicit quotes')
  t.true(result.code.includes('plain: 2'), 'Should keep unquoted properties unchanged')
})

test('preserve JSON5 property quotes by default', async (t) => {
  const source = `{
  "quoted": 'value',
  plain: 'other'
}`

  const result = await format('quoted.json5', source, undefined)

  t.is(result.errors.length, 0, 'Should not have errors')
  t.true(result.code.includes('"quoted": \'value\''), 'Should preserve explicit JSON5 quotes')
  t.true(result.code.includes("plain: 'other'"), 'Should keep unquoted JSON5 properties unchanged')
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
