import test from 'node:test'
import assert from 'node:assert/strict'
import {createHash} from 'node:crypto'
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {validateExecutable,qualifyNative} from '../scripts/verify-native-e2e.mjs'

test('native qualification rejects absent Noqeri selfhosting proof',()=>{
  assert.throws(()=>qualifyNative({}),/NOQERI_ROOT/)
  assert.throws(()=>qualifyNative({NOQERI_ROOT:'/nonexistent-noqeri'}),/ENOENT/)
})
test('native binary identity cannot be spoofed by a JavaScript script',()=>{
  const root=mkdtempSync(join(tmpdir(),'noesql-native-candidate-'))
  try{
    const file=join(root,'fake-native')
    const data=Buffer.from('#!/usr/bin/env node\nconsole.log("{}")\n'.repeat(300))
    writeFileSync(file,data)
    const pin=createHash('sha256').update(data).digest('hex')
    assert.throws(()=>validateExecutable(file,pin),/ELF|Mach-O|PE/)
    assert.throws(()=>validateExecutable(file,'f'.repeat(64)),/sha256 mismatch/)
  }finally{rmSync(root,{recursive:true,force:true})}
})
