import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
test('read-path profiler produces verified bounded workload timings',()=>{
  const script=fileURLToPath(new URL('../Benchmarks/read-path-profile.mjs',import.meta.url))
  const result=spawnSync(process.execPath,[script,'41','3'],{encoding:'utf8',timeout:30000})
  assert.equal(result.status,0,result.stderr)
  const report=JSON.parse(result.stdout)
  assert.equal(report.kind,'noesql-reference-read-path-profile')
  assert.equal(report.rowsPerTable,41)
  assert.equal(report.measurements.length,3)
  assert.equal(report.measurements[0].rowsReturned,1)
  assert.equal(report.measurements[2].rowsReturned,41)
  for(const item of report.measurements){
    assert.ok(item.medianMs>=0)
    assert.equal(item.trials,3)
  }
})
