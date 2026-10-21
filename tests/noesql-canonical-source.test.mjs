import test from 'node:test'
import assert from 'node:assert/strict'
import {readdir,readFile} from 'node:fs/promises'
import {join,resolve,dirname} from 'node:path'
const root=resolve(new URL('..',import.meta.url).pathname)
async function walk(dir){
 const output=[]
 for(const x of await readdir(join(root,dir),{withFileTypes:true})){
  const p=join(dir,x.name)
  if(x.isDirectory()) output.push(...await walk(p))
  else output.push(p)
 }
 return output
}
test('all canonical NoeSQL sources and native fixtures are .noqeri with resolved local imports',async()=>{
 for(const part of ['Database','Benchmarks','tests']){
  const files=await walk(part)
  assert.equal(files.filter(p=>p.endsWith('.nqr')).length,0,part+' retained old source extension')
  for(const p of files.filter(p=>p.endsWith('.noqeri'))){
   const code=await readFile(join(root,p),'utf8')
   for(const m of code.matchAll(/import\s+"([^"]+)"/g)){
    const target=m[1].startsWith('.')?resolve(root,dirname(p),m[1]):resolve(root,dirname(p),m[1])
    assert.ok(target.endsWith('.noqeri'),'legacy source import in '+p)
    const expected=await readFile(target,'utf8')
    assert.ok(expected.length>0,'empty import '+target)
   }
  }
 }
})
