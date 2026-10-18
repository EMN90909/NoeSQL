import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {mkdtemp,rm,writeFile} from 'node:fs/promises'
import {execFile} from 'node:child_process'
import {promisify} from 'node:util'
import {join} from 'node:path'
import {tmpdir} from 'node:os'
import {readNqd,runNqd,verifyNqd} from '../Runtime/noesql-store.mjs'

const schema='table notes { id: int key auto\n title: text required\n}'
test('new .nqdb databases persist and verify without confusing .nqd scripts',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'noesql-nqdb-'))
  const data=join(dir,'notes.nqdb')
  try{
    await runNqd(data,schema)
    await runNqd(data,'notes add {title:"native extension"}')
    assert.equal((await readNqd(data)).tables.notes.rows[0].title,'native extension')
    assert.equal((await verifyNqd(data)).ok,true)
    await assert.rejects(readNqd(join(dir,'source.noe')),/extension/)
  }finally{await rm(dir,{recursive:true,force:true})}
})
test('existing legacy .nqd reference databases remain readable',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'noesql-legacy-'))
  const data=join(dir,'legacy.nqd')
  try{
    await runNqd(data,schema)
    assert.equal((await verifyNqd(data)).ok,true)
  }finally{await rm(dir,{recursive:true,force:true})}
})

test('CLI runs .nqd source into .nqdb database without renaming legacy files',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'noesql-script-'))
  const data=join(dir,'database.nqdb')
  const source=join(dir,'program.nqd')
  const cli=new URL('../Runtime/noesql-cli.mjs',import.meta.url).pathname
  try{
    await writeFile(source,schema)
    const {stdout}=await promisify(execFile)(process.execPath,[cli,'run',data,source])
    assert.equal(JSON.parse(stdout)[0].kind,'table')
    assert.equal((await verifyNqd(data)).ok,true)
  }finally{await rm(dir,{recursive:true,force:true})}
})
