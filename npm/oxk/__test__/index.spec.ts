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

test('parse static ETS only with explicit language', async (t) => {
  const source = "let character: char = c'a'; let value: float = 1.25f;"

  const inferred = await parse('static.ets', source)
  t.true(inferred.errors.length > 0, 'A .ets path should keep using the ArkTS 1.1 grammar')

  const explicit = await parse('static.ets', source, { lang: 'ets-static' })
  t.is(explicit.errors.length, 0, 'Explicit static ETS should parse without errors')
  t.is(explicit.program.body.length, 2)
})

test('parse static ETS metadata', async (t) => {
  const source = [
    '@interface Mark {}',
    '@Mark const value: int = 1;',
    'final class A {}',
    'native function f(): void;',
  ].join('\n')

  const result = await parse('metadata.ets', source, { lang: 'ets-static' })
  t.is(result.errors.length, 0)
  t.true(Object.hasOwn(result.program.body[1], 'decorators'))
  t.true(result.program.body[2].final)
  t.true(result.program.body[3].native)
})
