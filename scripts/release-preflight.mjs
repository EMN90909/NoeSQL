#!/usr/bin/env node
// Fails closed when npm publish invokes prepublishOnly.
import {readFileSync} from 'node:fs'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {resolve} from 'node:path'
const root=fileURLToPath(new URL('../',import.meta.url))
function must(ok,message){if(!ok)throw Error(message)}
function command(program,args){
  const p=spawnSync(program,args,{cwd:root,encoding:'utf8',stdio:'pipe',shell:false,timeout:180000,maxBuffer:16*1024*1024})
  must(!p.error&&!p.signal&&p.status===0,
    'gate failed ('+program+'): '+String(p.stderr||p.error?.message||p.signal||p.status).slice(-500))
  return p.stdout.trim()
}
export function releasePolicy({manifest,env,licenseText}){
  must(manifest?.name==='noesql'&&manifest.version==='0.7.6','unreviewed release identity')
  must(manifest.private===false,'package marked private')
  must(manifest.license==='GPL-3.0-only'&&licenseText.includes('GNU GENERAL PUBLIC LICENSE')&&licenseText.includes('Version 3, 29 June 2007'),'GPLv3 release license missing')
  must(manifest.repository?.url==='git+https://github.com/EMN90909/NoeSQL.git','unverified repository identity')
  must(manifest.publishConfig?.registry==='https://registry.npmjs.org'&&manifest.publishConfig?.access==='public'&&manifest.publishConfig?.tag==='next','unreviewed npm registry/access/tag')
  must(manifest.scripts?.prepublishOnly==='node scripts/release-preflight.mjs','publication hook missing')
  must(env.NOESQL_APPROVE_PUBLICATION==='publish-noesql@0.7.6','explicit publication approval missing')
  must(env.NOESQL_LICENSE_REVIEWED==='GPL-3.0-only','owner legal review missing')
  must(/^[0-9a-f]{40}$/.test(env.NOESQL_APPROVED_COMMIT_SHA||''),'reviewed exact commit SHA missing')
  for(const name of ['NOQERI_ROOT','NOQERI_COMMIT_SHA','NOQERI_SOURCE_CLOSURE_SHA256','NOESQL_NATIVE_BIN','NOESQL_NATIVE_BIN_SHA256','NOQERI_NATIVE_COMPILER','NOQERI_NATIVE_COMPILER_SHA256','NOQERI_STAGE0','NOQERI_STAGE0_SHA256']){
    must(Boolean(env[name]),name+' required: native compiler and Zig relational proof not provided')
  }
  return {sha:env.NOESQL_APPROVED_COMMIT_SHA,tag:manifest.publishConfig.tag}
}
export function qualifyPublicRelease(env=process.env){
  const manifest=JSON.parse(readFileSync(resolve(root,'package.json'),'utf8'))
  const licenseText=readFileSync(resolve(root,'LICENSE'),'utf8')
  const policy=releasePolicy({manifest,licenseText,env})
  must(command('git',['rev-parse','HEAD'])===policy.sha,'Git checkout does not match reviewed commit')
  must(command('git',['status','--porcelain'])==='','Git checkout must be clean')
  // The compiled language, pager and durable SQL tests must actually run.
  command(process.execPath,['scripts/build-noqeri-zig-link.mjs'])
  command(process.execPath,['scripts/verify-native-e2e.mjs'])
  command(process.execPath,['scripts/qualify-zig.mjs'])
  command('npm',['test','--ignore-scripts'])
  command('npm',['run','test:npm-pack','--ignore-scripts'])
  return {approved:true,version:manifest.version,commit:policy.sha,tag:policy.tag}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{console.log(JSON.stringify(qualifyPublicRelease(),null,2))}
  catch(e){console.error('PUBLIC NPM RELEASE BLOCKED: '+e.message);process.exitCode=1}
}
