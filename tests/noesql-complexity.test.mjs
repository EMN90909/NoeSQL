import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { lex, parse } from '../Runtime/noesql-reference.mjs'

test('source cap rejects oversized input before lexing', () => {
  assert.throws(() => lex(' '.repeat(1048577)), e => e.code === 'NOE005')
})
test('token budget rejects pathological source', () => {
  assert.throws(() => lex('x '.repeat(100001)), e => e.code === 'NOE006')
})
test('nesting cap rejects deeply nested expressions', () => {
  assert.throws(() => parse('table x { id: int key }\nx get id where ' + '('.repeat(65) + 'true' + ')'.repeat(65)), e => e.code === 'NOE007')
})
test('ordinary queries remain valid within limits', () => {
  const rows = parse('table x { id: int key }\nx get id where id >= 1')
  assert.equal(rows.length, 2)
})
