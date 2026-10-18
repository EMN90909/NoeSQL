import {existsSync} from 'node:fs'
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
gate('trusted Noqeri compiler seed',executable(process.env.NOQERI_STAGE0),'NOQERI_STAGE0 must be a local trusted executable')
const raw=process.env.NOESQL_NATIVE_CONFORMANCE_JSON
if(!raw)gate('compiled native Noqeri/Zig relational conformance',false,'NOESQL_NATIVE_CONFORMANCE_JSON command not configured')
else{
  try{
    const argv=JSON.parse(raw)
    if(!Array.isArray(argv)||argv.length===0||!argv.every(a=>typeof a==='string'&&a.length>0))throw new Error('expected non-empty JSON argv')
    run('compiled native Noqeri/Zig relational conformance',argv[0],argv.slice(1))
  }catch(e){gate('compiled native Noqeri/Zig relational conformance',false,e.message)}
}
run('NoeSQL JavaScript/SQLite reference tests',process.execPath,['--test'])
console.log(JSON.stringify({status:results.every(x=>x.status==='pass')?'qualified':'blocked',gates:results},null,2))
if(results.some(x=>x.status==='fail'))process.exitCode=1
