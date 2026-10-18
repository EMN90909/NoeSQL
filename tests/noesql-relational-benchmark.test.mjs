import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtempSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {baselineTrial,nativeRequired,sqliteOnly} from '../Benchmarks/qualify-relational-benchmark.mjs'
test('native performance cannot be claimed without actual compiled Noqeri Zig evidence',()=>{
  assert.throws(()=>nativeRequired({}),/NOESQL_NATIVE_BIN/)
  const results=sqliteOnly({transactions:10,rowsPerTransaction:2,blobBytes:512,repeats:3})
  assert.equal(results.status,'sqlite-baseline-only')
  assert.equal(results.nativeComparisonQualified,false)
  assert.equal(results.trials.length,3)
  for(const run of results.trials){
    assert.equal(run.restartVerified,true)
    assert.equal(run.storedRows,20)
    assert.ok(run.p99Ms>=run.p50Ms)
  }
})
test('SQLite reference baseline verifies full durability mode and cold restart',()=>{
  const directory=mkdtempSync(join(tmpdir(),'noesql-sqlite-reference-'))
  try{
    const result=baselineTrial(directory,{transactions:32,rowsPerTransaction:1,blobBytes:4096})
    assert.equal(result.durability,'WAL/FULL')
    assert.equal(result.restartVerified,true)
    assert.equal(result.storedRows,32)
  }finally{rmSync(directory,{recursive:true,force:true})}
})
