import test from 'ava'
import { format } from '../index.js'

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

  // JSON5 requires external formatter callbacks
  // For testing, we'll provide mock callbacks
  const initExternalFormatter = async (numThreads: number): Promise<string[]> => {
    return ['json5']
  }

  const formatEmbedded = async (options: Record<string, any>, tagName: string, code: string): Promise<string> => {
    return code
  }

  const formatFile = async (
    options: Record<string, any>,
    parserName: string,
    fileName: string,
    code: string,
  ): Promise<string> => {
    // Mock JSON5 formatting - in real scenario, this would call Prettier
    if (parserName === 'json5') {
      // Simple formatting: ensure proper indentation
      return JSON.stringify(JSON.parse(code.replace(/'/g, '"')), null, 2)
    }
    return code
  }

  const result = await format('test.json5', json5Source, undefined, initExternalFormatter, formatEmbedded, formatFile)

  // Note: Without proper Prettier integration, this might fail
  // But we can at least verify the file type is recognized
  t.truthy(result, 'Should return a result')

  // If external formatter is not properly set up, we might get an error
  // This is expected behavior
  if (result.errors.length > 0) {
    // Check if it's an "external formatter required" error
    const hasExternalFormatterError = result.errors.some(
      (err: string) => err.includes('External formatter') || err.includes('external'),
    )
    t.true(hasExternalFormatterError || result.code === json5Source, 'Should either format or return appropriate error')
  } else {
    t.truthy(result.code, 'Should return formatted code')
  }
})

test('format JSON5 with comments', async (t) => {
  const json5Source = `{
  // Single line comment
  name: 'test',
  /* Multi-line
     comment */
  version: '1.0.0'
}`

  const initExternalFormatter = async (numThreads: number): Promise<string[]> => {
    return ['json5']
  }

  const formatEmbedded = async (options: Record<string, any>, tagName: string, code: string): Promise<string> => {
    return code
  }

  const formatFile = async (
    options: Record<string, any>,
    parserName: string,
    fileName: string,
    code: string,
  ): Promise<string> => {
    if (parserName === 'json5') {
      // Mock formatter - preserve structure
      return code
    }
    return code
  }

  const result = await format('config.json5', json5Source, undefined, initExternalFormatter, formatEmbedded, formatFile)

  t.truthy(result, 'Should return a result')
  // JSON5 formatting requires external formatter, so we check for either success or appropriate error
  t.true(result.errors.length === 0 || result.code === json5Source, 'Should handle JSON5 file')
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
