import {existsSync,readFileSync} from 'node:fs'
import {createHash} from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {resolve,join} from 'node:path'

const root=fileURLToPath(new URL('../',import.meta.url))
const results=[]
function gate(name,ok,detail=''){results.push({name,status:ok?'pass':'fail',detail});console.log(`[${ok?'PASS':'FAIL'}] ${name}${detail?' - '+detail:''}`)}
function run(name,program,args,cwd=root){
  const result=spawnSync(program,args,{cwd,encoding:'utf8',timeout:120000,stdio:'pipe',shell:false})
  const ok=!result.error&&result.status===0
  gate(name,ok,ok?(result.stdout||'').trim().slice(-240):(result.stderr||result.error?.message||'nonzero exit').trim().slice(-320))
  return ok
}
function executable(path){return typeof path==='string'&&path.length>0&&existsSync(resolve(path))}
const zig=process.env.ZIG_BIN||'zig'
const version=spawnSync(zig,['version'],{cwd:root,encoding:'utf8',timeout:10000})
const zigPresent=version.status===0
const zigVersion=(version.stdout||'').trim()
gate('Zig 0.15.2 available',zigPresent&&zigVersion==='0.15.2',zigPresent?zigVersion:'zig binary unavailable')
if(zigPresent&&zigVersion==='0.15.2')run('Zig bridge unit tests',zig,['build','test'],join(root,'Native','zig'))
const seed=process.env.NOQERI_STAGE0
const pinned=(process.env.NOQERI_STAGE0_SHA256||'').toLowerCase()
let authenticatedSeed=false
if(executable(seed)&&/^[a-f0-9]{64}$/.test(pinned)){
  const actual=createHash('sha256').update(readFileSync(resolve(seed))).digest('hex')
  authenticatedSeed=actual===pinned
}
gate('trusted Noqeri compiler seed',authenticatedSeed,'NOQERI_STAGE0 path and pinned NOQERI_STAGE0_SHA256 required; matching a hash alone is not self-hosting proof')
const raw=process.env.NOESQL_NATIVE_CONFORMANCE_JSON
if(!raw)gate('compiled native Noqeri/Zig relational conformance',false,'NOESQL_NATIVE_CONFORMANCE_JSON command not configured')
else{
  try{
    const argv=JSON.parse(raw)
    if(!Array.isArray(argv)||argv.length===0||!argv.every(a=>typeof a==='string'&&a.length>0))throw new Error('expected non-empty JSON argv')
    run('compiled native Noqeri/Zig relational conformance',argv[0],argv.slice(1))
  }catch(e){gate('compiled native Noqeri/Zig relational conformance',false,e.message)}
}
// A callback echo test and a caller-selected zero-exit command are NOT native
// parser/executor/storage conformance. Keep this gate fail-closed until the
// Zig pager, compiled Noqeri ABI and independently verified suites are wired.
// A Zig page store is now implemented, but its tests and compiled Noqeri link
// must pass before it can be selected as a relational production engine.
if(process.env.NOQERI_ROOT)run('pinned Noqeri self-hosting proof',process.execPath,['scripts/verify-noqeri-bootstrap.mjs'])
else gate('pinned Noqeri self-hosting proof',false,'NOQERI_ROOT, NOQERI_COMMIT_SHA and NOQERI_SOURCE_CLOSURE_SHA256 are required')
gate('compiled Noqeri executor linked to Zig pager with differential results',false,
  'BLOCKED: no trusted Noqeri-compiled native relational executor is linked and tested')
gate('native cross-platform durability, recovery and security release suite',false,
  'BLOCKED: Linux-only experimental Zig v2, no crash-powercut/Windows/macOS qualification')
run('NoeSQL JavaScript/SQLite reference tests',process.execPath,['--test'])
console.log(JSON.stringify({status:results.every(x=>x.status==='pass')?'qualified':'blocked',gates:results},null,2))
if(results.some(x=>x.status==='fail'))process.exitCode=1
