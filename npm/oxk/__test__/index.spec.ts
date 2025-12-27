import test from 'ava'

import { format } from '../index.js'

test('format function exists', (t) => {
  t.truthy(format, 'format function should be exported')
  t.is(typeof format, 'function', 'format should be a function')
})
