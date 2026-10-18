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
  const data=join(workspace,'actual.nqdb'),oracle=join(workspace,'oracle.nqdb')
  const schema=join(workspace,'schema.nqd'),writes=join(workspace,'writes.nqd'),bad=join(workspace,'bad.nqd')
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
    return {qualified:true, rows:rows.rows?.length||0, native_sha256:env.NOESQL_NATIVE_BIN_SHA256, compiler_sha256:proof.stage2_sha256}
  }finally{rmSync(workspace,{recursive:true,force:true})}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(qualifyNative(),null,2))}
  catch(e){console.error('NATIVE INTEGRATION BLOCKED: '+e.message);process.exitCode=1}
}
