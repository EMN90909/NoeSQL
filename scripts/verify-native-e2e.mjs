#!/usr/bin/env node
// Native qualification gate: real executable provenance + Noqeri syntax + Zig bytes.
import {createHash} from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {readFileSync, realpathSync, statSync, mkdtempSync, writeFileSync, rmSync} from 'node:fs'
import {join, isAbsolute, resolve} from 'node:path'
import {tmpdir} from 'node:os'
import {fileURLToPath} from 'node:url'
import {verifyNoqeriProof} from './verify-noqeri-bootstrap.mjs'

const sourceRoot=fileURLToPath(new URL('../', import.meta.url))
function demand(ok,message){if(!ok)throw Error(message)}
function command(program,args,cwd,shouldPass=true){
  const p=spawnSync(program,args,{cwd,encoding:'utf8',timeout:30000,maxBuffer:4*1024*1024,shell:false,stdio:'pipe'})
  demand(!p.error&&!p.signal,'native execution failed: '+(p.error?.message||p.signal))
  demand(shouldPass?p.status===0:p.status!==0, shouldPass?'execution failed: '+p.stderr.slice(-500):'malformed script accepted')
  return p.stdout
}
function decoded(program,args,cwd){return JSON.parse(command(program,args,cwd))}
export function validateExecutable(file,pin){
  demand(isAbsolute(file||''),'NOESQL_NATIVE_BIN must be an absolute path')
  demand(/^[0-9a-f]{64}$/.test(pin||''),'NOESQL_NATIVE_BIN_SHA256 must be pinned')
  const path=realpathSync(file),info=statSync(path)
  demand(info.isFile(),'native executable must be regular file')
  const bytes=readFileSync(path)
  const digest=createHash('sha256').update(bytes).digest('hex')
  demand(digest===pin,'native executable sha256 mismatch')
  const magic=bytes.subarray(0,4).toString('hex')
  const binary=magic==='7f454c46'||['feedface','feedfacf','cefaedfe','cffaedfe'].includes(magic)||magic.slice(0,4)==='4d5a'
  demand(bytes.length>4096&&binary,'native candidate must be real ELF, Mach-O or PE machine code')
  return path
}
export function qualifyNative(env=process.env){
  demand(isAbsolute(env.NOQERI_ROOT||''),'NOQERI_ROOT is required as absolute path')
  const root=realpathSync(env.NOQERI_ROOT)
  const actualCommit=command('git',['-C',root,'rev-parse','HEAD'],sourceRoot).trim()
  const proof=verifyNoqeriProof({root,actualCommit,expectedCommit:env.NOQERI_COMMIT_SHA,
    expectedClosure:env.NOQERI_SOURCE_CLOSURE_SHA256,proofFile:env.NOQERI_STAGE2_PROOF||'build/stage2-proof.json'})
  const executable=validateExecutable(env.NOESQL_NATIVE_BIN,env.NOESQL_NATIVE_BIN_SHA256)
  const workspace=mkdtempSync(join(tmpdir(),'noesql-native-e2e-'))
  const data=join(workspace,'actual.noedb'),oracle=join(workspace,'oracle.noedb')
  const schema=join(workspace,'schema.nqds'),writes=join(workspace,'writes.nqds'),bad=join(workspace,'bad.nqds')
  const cli=fileURLToPath(new URL('../Runtime/noesql-cli.mjs',import.meta.url))
  try{
    writeFileSync(schema,'table users {\n id: int key auto\n name: text required\n age: int\n}\nquery everyone() { users get id, name, age sort name }\n')
    writeFileSync(writes,'users add {name:"Ada",age:33}\nusers add {name:"Bea",age:28}\n')
    writeFileSync(bad,'users add {name:}\n')
    const identity=decoded(executable,['identity','--json'],workspace)
    demand(identity.engine==='noqeri-zig-relational'&&identity.native===true&&identity.storage_format==='NQDB0002', 'native identity contract failed')
    demand(identity.noqeri_compiler_sha256===proof.stage2_sha256,'native compiler artifact mismatch')
    for(const script of [schema,writes]){
      command(executable,['run',data,script],workspace)
      command(process.execPath,[cli,'run',oracle,script],workspace)
    }
    const rows=decoded(executable,['query',data,'everyone'],workspace)
    const expected=decoded(process.execPath,[cli,'query',oracle,'everyone'],workspace)
    demand(JSON.stringify(rows)===JSON.stringify(expected),'native relational query differs from reference')
    demand(decoded(executable,['verify',data],workspace).ok===true,'native recovery verification failed')
    command(executable,['run',data,bad],workspace,false)
    demand(JSON.stringify(decoded(executable,['query',data,'everyone'],workspace))===JSON.stringify(rows),'failed transaction changed stored data')
    // WAL-only databases have no base image until checkpoint; check its real persisted format after a checkpoint.
    command(executable,['checkpoint',data],workspace)
    demand(readFileSync(data).subarray(0,8).toString('ascii')==='NQDB0002','not a Zig NQDB0002 database')
    demand(decoded(executable,['verify',data],workspace).ok===true,'native post-checkpoint verification failed')
    // Native relational conformance must include more than single-table CRUD.
    // Every native invocation is a fresh process against a real Zig database.
    const nativeMore=join(workspace,'relational.noedb'),referenceMore=join(workspace,'relational-oracle.noedb')
    const relationalSchema=join(workspace,'relational-schema.nqds')
    const relationalWrites=join(workspace,'relational-writes.nqds')
    const invalidBatch=join(workspace,'relational-invalid.nqds')
    writeFileSync(relationalSchema,`table departments { id: int key auto
 label: text required
}
table employees { id: int key auto
 department_id: ref departments required
 salary: int required
}
query payroll() { departments get departments.label, count(employees.id) as people, sum(employees.salary) as salary join employees on departments.id == employees.department_id group departments.label having people >= 1 sort departments.label }
query coverage() { departments get departments.label, count(employees.id) as people left join employees on departments.id == employees.department_id group departments.label sort departments.label }
query paid(min: int = 1) { employees get id, salary where salary >= $min sort id }
`)
    writeFileSync(relationalWrites,`departments add {label:"Engineering"}
departments add {label:"Research"}
departments add {label:"Unstaffed"}
employees add {department_id:1,salary:60}
employees add {department_id:1,salary:40}
employees add {department_id:2,salary:30}
`)
    writeFileSync(invalidBatch,`atomic {
 employees add {department_id:1,salary:25}
 employees add {department_id:999,salary:55}
}
`)
    for(const program of [relationalSchema,relationalWrites]){
      command(executable,['run',nativeMore,program],workspace)
      command(process.execPath,[cli,'run',referenceMore,program],workspace)
    }
    const compareQuery=(name,args=[])=>{
      const nativeResult=decoded(executable,['query',nativeMore,name,...args],workspace)
      const oracleResult=decoded(process.execPath,[cli,'query',referenceMore,name,...args],workspace)
      demand(JSON.stringify(nativeResult)===JSON.stringify(oracleResult),'native relational mismatch: '+name)
      return nativeResult
    }
    demand(compareQuery('payroll').rows.length===2,'native grouped inner join returned wrong count')
    const coverage=compareQuery('coverage').rows
    demand(coverage.length===3&&coverage.some(r=>r['departments.label']==='Unstaffed'&&r.people===0),
      'native left join did not preserve unmatched NULL/COUNT semantics')
    demand(compareQuery('paid',['{"min":40}']).rows.length===2,'native parameterized filter mismatch')
    const beforeInvalid=decoded(executable,['verify',nativeMore],workspace)
    demand(Number.isSafeInteger(beforeInvalid.storageSequence)&&beforeInvalid.storageSequence>=2,
      'native verify must report durable monotonic storage generation')
    command(executable,['run',nativeMore,invalidBatch],workspace,false)
    command(process.execPath,[cli,'run',referenceMore,invalidBatch],workspace,false)
    compareQuery('payroll');compareQuery('coverage');compareQuery('paid',['{"min":40}'])
    demand(decoded(executable,['verify',nativeMore],workspace).storageSequence===beforeInvalid.storageSequence,
      'failed native transaction advanced durable generation')
    command(executable,['checkpoint',nativeMore],workspace)
    demand(readFileSync(nativeMore).subarray(0,8).toString('ascii')==='NQDB0002',
      'native relational checkpoint did not use Zig page format')
    compareQuery('coverage') // fresh-process read after checkpoint
    return {qualified:true, rows:rows.rows?.length||0, native_sha256:env.NOESQL_NATIVE_BIN_SHA256, compiler_sha256:proof.stage2_sha256}
  }finally{rmSync(workspace,{recursive:true,force:true})}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(qualifyNative(),null,2))}
  catch(e){console.error('NATIVE INTEGRATION BLOCKED: '+e.message);process.exitCode=1}
}
