#!/usr/bin/env node
// Builds an actual Noqeri-generated machine-code object and links Zig pager.
// No fallback to C++ bootstrap, JS reference, or a fabricated zero-exit command.
import {readFileSync,statSync,realpathSync,mkdtempSync,rmSync,existsSync} from 'node:fs'
import {createHash} from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {tmpdir} from 'node:os'
import {join,isAbsolute,resolve} from 'node:path'
import {fileURLToPath} from 'node:url'
import {verifyNoqeriProof} from './verify-noqeri-bootstrap.mjs'
const root=fileURLToPath(new URL('../',import.meta.url))
const digest=p=>createHash('sha256').update(readFileSync(p)).digest('hex')
function need(ok,msg){if(!ok)throw Error(msg)}
function run(cmd,args,cwd=root){
  const r=spawnSync(cmd,args,{cwd,encoding:'utf8',shell:false,timeout:120000,maxBuffer:4*1024*1024})
  need(!r.error&&!r.signal&&r.status===0,'native build failed: '+[cmd,...args].join(' ')+' '+(r.stderr||r.error?.message||'').slice(-600))
  return (r.stdout||'').trim()
}
export function checkNativeCompiler(env=process.env){
  need(process.platform==='linux'&&process.arch==='x64','Linux x64-only native linking fixture')
  need(isAbsolute(env.NOQERI_ROOT||''),'NOQERI_ROOT checkout path required')
  need(isAbsolute(env.NOQERI_NATIVE_COMPILER||''),'NOQERI_NATIVE_COMPILER absolute executable path required')
  need(/^[a-f0-9]{64}$/.test(env.NOQERI_NATIVE_COMPILER_SHA256||''),'NOQERI_NATIVE_COMPILER_SHA256 pin required')
  const compiler=realpathSync(env.NOQERI_NATIVE_COMPILER)
  need(statSync(compiler).isFile()&&digest(compiler)===env.NOQERI_NATIVE_COMPILER_SHA256,'native compiler checksum mismatch')
  const noqeri=realpathSync(env.NOQERI_ROOT)
  const proof=verifyNoqeriProof({
    root:noqeri,actualCommit:run('git',['-C',noqeri,'rev-parse','HEAD']),
    expectedCommit:env.NOQERI_COMMIT_SHA,expectedClosure:env.NOQERI_SOURCE_CLOSURE_SHA256,
    proofFile:env.NOQERI_STAGE2_PROOF||'build/stage2-proof.json'
  })
  need(proof.verified===true,'Noqeri proof incomplete')
  // A pin alone cannot distinguish a self-hosted compiler from the archived
  // C++ seed. Require this exact native binary to be bound to the independently
  // reviewed Stage-2 proof, not simply to exit successfully.
  const proofDocument=JSON.parse(readFileSync(
    resolve(noqeri,env.NOQERI_STAGE2_PROOF||'build/stage2-proof.json'),'utf8'))
  const origin=proofDocument.artifacts?.native_compiler
  need(origin?.built_by_stage2===true&&
    origin?.producer_stage2_sha256===proof.stage2_sha256&&
    origin?.sha256===digest(compiler),
    'no verified Stage-2 to native-compiler artifact linkage (archival C++ bootstrap is not accepted)')
  return {compiler,proof}
}
export function buildNoqeriZigLink(env=process.env){
  const {compiler,proof}=checkNativeCompiler(env)
  const zig=env.ZIG_BIN||'zig'
  need(run(zig,['version'])==='0.15.2','Zig 0.15.2 required')
  const zigRoot=join(root,'Native','zig')
  run(zig,['build','test'],zigRoot)
  run(zig,['build','-Doptimize=ReleaseSafe'],zigRoot)
  const lib=join(zigRoot,'zig-out','lib','libnoesql_zig_pages.a')
  need(existsSync(lib)&&statSync(lib).size>0,'native Zig page library missing')
  const dir=mkdtempSync(join(tmpdir(),'noesql-native-link-'))
  try{
    const asm=join(dir,'relational.s'),obj=join(dir,'relational.o')
    const exe=join(dir,'relational-smoke'),data=join(dir,'data.nqdb')
    run(compiler,['build',join(root,'tests','noesql_zig_relational.nqr'),asm])
    need(existsSync(asm)&&statSync(asm).size>0,'Noqeri generated no machine-code assembly')
    run('clang',['-c','-x','assembler',asm,'-o',obj])
    need(/\bnoeZigRelationalSmoke\b/.test(run('nm',['-g','--defined-only',obj])),'Noqeri object lacks exported relational symbol')
    run('clang',['-std=c11','-Wall','-Wextra','-Werror',
      join(root,'Native','link-tests','noqeri_link_smoke.c'),obj,lib,'-pthread','-o',exe])
    run(exe,['write',data])
    run(exe,['verify',data])
    return {verified:true,compiler_sha256:digest(compiler),noqeri_source_sha256:proof.source_closure_sha256,
      object_sha256:digest(obj),zig_library_sha256:digest(lib),cold_restart:true,
      scope:'bounded typed single-row native smoke, not full relational SQL'}
  }finally{rmSync(dir,{recursive:true,force:true})}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(buildNoqeriZigLink(),null,2))}
  catch(e){console.error('NATIVE COMPILER LINK BLOCKED: '+e.message);process.exitCode=1}
}
