#!/usr/bin/env node
// No-network install smoke for the real package tarball, never a public-publish claim.
import {spawnSync} from 'node:child_process'
import {mkdtempSync,rmSync,writeFileSync,mkdirSync} from 'node:fs'
import {join} from 'node:path'
import {tmpdir} from 'node:os'
import {fileURLToPath} from 'node:url'
const root=fileURLToPath(new URL('../',import.meta.url))
function run(command,args,cwd){
  const p=spawnSync(command,args,{cwd,encoding:'utf8',shell:false,timeout:120000,maxBuffer:8*1024*1024,
    env:{...process.env,npm_config_update_notifier:'false',npm_config_audit:'false',npm_config_fund:'false'}})
  if(p.error||p.status!==0)throw Error(command+' failed: '+(p.stderr||p.error?.message||'exit '+p.status).slice(-1000))
  return p.stdout
}
export function verifyPackageContents(files){
  const names=files.map(f=>f.path)
  if(names.some(x=>/(^|\/)(\.npmrc|\.env(?:\.|$)|\.git|id_rsa|node_modules|bootstrap-local\.json)(\/|$)/i.test(x)))
    throw Error('sensitive or generated files included in npm archive')
  for(const required of ['package.json','Runtime/noesql-embedded.mjs','Runtime/noesql-adapters.mjs','Runtime/noesql-reference.mjs']){
    if(!names.includes(required))throw Error('npm package missing '+required)
  }
  return true
}
export function testLocalNpmPack(){
  const workspace=mkdtempSync(join(tmpdir(),'noesql-npm-smoke-'))
  try{
    const output=JSON.parse(run('npm',['pack','--ignore-scripts','--json','--pack-destination',workspace],root))
    if(!Array.isArray(output)||output.length!==1)throw Error('npm pack did not produce one archive')
    verifyPackageContents(output[0].files||[])
    const tar=join(workspace,output[0].filename)
    const consumer=join(workspace,'consumer')
    mkdirSync(consumer)
    writeFileSync(join(consumer,'package.json'),'{"private":true,"type":"module"}\n')
    run('npm',['install','--offline','--ignore-scripts','--no-save','--no-audit','--no-fund',tar],consumer)
    const smoke=join(consumer,'smoke.mjs')
    writeFileSync(smoke,[
      'import {openNoeSQLEmbedded} from "noesql";',
      'const db=await openNoeSQLEmbedded();',
      'try {',
      'if(db.backend!=="sqlite-memory")throw Error("unexpected backend");',
      'db.run("table people { id: int key auto\\n name: text required\\n }");',
      'db.run("people add {name:\\"Ada\\"}");',
      'const rows=db.query("from people\\nselect name");',
      'if(JSON.stringify(rows)!==JSON.stringify([{name:"Ada"}]))throw Error("package CRUD failed: "+JSON.stringify(rows));',
      'if(!db.verify().ok)throw Error("package integrity failure");',
      '} finally {db.close();}',
      'console.log("local tarball installation and SQL reference smoke passed");'
    ].join('\n'))
    const stdout=run(process.execPath,[smoke],consumer)
    return {ok:true,package:output[0].name,version:output[0].version,files:output[0].files.length,installedOffline:true,stdout:stdout.trim(),published:false}
  }finally{rmSync(workspace,{recursive:true,force:true})}
}
if(process.argv[1]===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(testLocalNpmPack(),null,2))}
  catch(e){console.error('NPM LOCAL PACK TEST FAILED: '+e.message);process.exitCode=1}
}
