#!/usr/bin/env node
// Compile Noqeri source to a REAL ELF object, link the Zig pager, execute a
// durable cold-restart smoke, and retain reviewed evidence. This bounded
// feature is NOT a full native SQL CLI or a substitute for Stage-2 proof.
import {readFileSync,statSync,realpathSync,mkdtempSync,rmSync,existsSync,mkdirSync,copyFileSync,writeFileSync} from 'node:fs'
import {createHash} from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {tmpdir} from 'node:os'
import {join,isAbsolute,resolve,dirname} from 'node:path'
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
  // Never relabel an archived C++ compiler as self-hosted by pinning its hash.
  const origin=JSON.parse(readFileSync(resolve(noqeri,env.NOQERI_STAGE2_PROOF||'build/stage2-proof.json'),'utf8')).artifacts?.native_compiler
  need(origin?.built_by_stage2===true && origin?.producer_stage2_sha256===proof.stage2_sha256 &&
    origin?.sha256===digest(compiler),'no verified Stage-2 to native-compiler artifact linkage (archival C++ bootstrap is not accepted)')
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
    const exe=join(dir,'relational-smoke'),data=join(dir,'data.noedb')
    run(compiler,['build',join(root,'tests','noesql_zig_relational.nqr'),asm])
    need(existsSync(asm)&&statSync(asm).size>0,'Noqeri generated no machine-code assembly')
    run('clang',['-c','-x','assembler',asm,'-o',obj])
    need(/\bnoeZigRelationalSmoke\b/.test(run('nm',['-g','--defined-only',obj])),'Noqeri object lacks exported relational symbol')
    const unresolved=run('nm',['-u',obj])
    for(const symbol of ['noesql_zig_page_open','noesql_zig_page_get','noesql_zig_page_stage','noesql_zig_page_commit'])
      need(new RegExp('\\b'+symbol+'\\b').test(unresolved),'compiled Noqeri object does not call native Zig pager: '+symbol)
    run('clang',['-std=c11','-Wall','-Wextra','-Werror',
      join(root,'Native','link-tests','noqeri_link_smoke.c'),obj,lib,'-pthread','-o',exe])
    run(exe,['write',data])
    run(exe,['verify',data])
    need(readFileSync(data).subarray(0,8).toString('ascii')==='NQDB0002','native checkpoint bytes are not Zig NQDB0002')
    // Keep the binary AND the assembly/object/provenance for independent review.
    const out=resolve(env.NOESQL_NATIVE_OUT_DIR||join(root,'build','noesql-native-smoke'))
    need(!existsSync(out),'native artifact output already exists; choose a fresh NOESQL_NATIVE_OUT_DIR')
    mkdirSync(dirname(out),{recursive:true})
    mkdirSync(out,{mode:0o700})
    try{
      for(const [src,name] of [[asm,'relational.s'],[obj,'relational.o'],[exe,'relational-smoke']])copyFileSync(src,join(out,name))
      const evidence={kind:'noesql-noqeri-zig-native-smoke',verified:true,
        compiler_sha256:digest(compiler),stage2_sha256:proof.stage2_sha256,
        noqeri_source_sha256:proof.source_closure_sha256,
        source_sha256:digest(join(root,'tests','noesql_zig_relational.nqr')),
        assembly_sha256:digest(asm),object_sha256:digest(obj),
        zig_library_sha256:digest(lib),executable_sha256:digest(exe),
        format:'NQDB0002',cold_restart:true,
        scope:'bounded typed single-row Noqeri-generated smoke; NOT full relational SQL'}
      writeFileSync(join(out,'provenance.json'),JSON.stringify(evidence,null,2)+'\n',{flag:'wx',mode:0o600})
      return {...evidence,artifacts:out}
    }catch(e){rmSync(out,{recursive:true,force:true});throw e}
  }finally{rmSync(dir,{recursive:true,force:true})}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(buildNoqeriZigLink(),null,2))}
  catch(e){console.error('NATIVE COMPILER LINK BLOCKED: '+e.message);process.exitCode=1}
}