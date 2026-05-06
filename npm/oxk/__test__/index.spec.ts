import test from 'ava'

import { format, parse } from '../index.js'

test('format function exists', (t) => {
  t.truthy(format, 'format function should be exported')
  t.is(typeof format, 'function', 'format should be a function')
})

test('parse function exists', (t) => {
  t.truthy(parse, 'parse function should be exported')
  t.is(typeof parse, 'function', 'parse should be a function')
})

test('parse TypeScript source', async (t) => {
  const result = await parse('test.ts', 'import { foo } from "bar"; const value: number = 1')

  t.is(result.errors.length, 0, 'Should not have parse errors')
  t.is(result.program.type, 'Program')
  t.is(result.program.body[0].type, 'ImportDeclaration')
  t.true(result.module.hasModuleSyntax)
  t.is(result.module.staticImports[0].moduleRequest.value, 'bar')
})

test('parse ArkTS source', async (t) => {
  const source = `@Component
struct Demo {
  build() {
    Text('hi')
  }
}`

  const result = await parse('demo.ets', source)

  t.is(result.errors.length, 0, 'Should not have parse errors')
  t.is(result.program.type, 'Program')
  t.is(result.program.body.length, 1)
})
