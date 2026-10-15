import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { parse, NoeError } from '../Runtime/noesql-reference.mjs'

// Deterministic grammar fuzz: malformed scripts must never expose raw JS
// TypeError/RangeError or crash the reference parser.
test('3500 deterministic malformed inputs fail with structured diagnostics',()=>{
  let state=8997
  const next=()=>{state=(Math.imul(state,1664525)+1013904223)>>>0;return state/4294967296}
  const alphabet='{}():,.?=$+-*/!<>\\n abcdefghijklmnopqrstuvwxyz_0123456789"\\\\'
  for(let sample=0;sample<3500;sample++){
    const length=Math.floor(next()*140)
    let source=''
    for(let i=0;i<length;i++)source+=alphabet[Math.floor(next()*alphabet.length)]
    try {parse(source)} catch(e) {
      assert.ok(e instanceof NoeError,'unexpected parser error for '+JSON.stringify(source)+': '+e)
      assert.match(e.code,/^NOE[0-9]{3}$/)
    }
  }
})
