// Runs against the real repository's existing reference SQLite adapter.
import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {mkdtemp,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {openNoeSQLEmbedded} from '../Runtime/noesql-embedded.mjs'

test('existing .noe table and named query catalog survives SQLite WAL restart',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'noe-adapter-integration-'))
  const path=join(dir,'app.sqlite')
  try{
    const first=await openNoeSQLEmbedded({file:path})
    try{
      first.run('table users { id: int key auto\n name: text required\n }\nquery everyone() { users get id, name }')
      first.run('users add {name:"Ada"}')
      assert.deepEqual(first.named('everyone').rows,[{id:1,name:'Ada'}])
      assert.equal(first.verify().ok,true)
      assert.ok(first.database.prepare('SELECT COUNT(*) AS n FROM "_noesql_catalog"').get().n>=2)
      await assert.rejects(openNoeSQLEmbedded({file:join(dir,'native.nqd')}),/absolute .sqlite/)
    }finally{first.close()}
    const reopened=await openNoeSQLEmbedded({file:path})
    try{
      reopened.run('users add {name:"Bea"}')
      assert.deepEqual(reopened.named('everyone').rows,[{id:1,name:'Ada'},{id:2,name:'Bea'}])
      assert.deepEqual(reopened.query('from users\nselect name\nsort name'),[{name:'Ada'},{name:'Bea'}])
      assert.equal(reopened.database.prepare('PRAGMA foreign_keys').get().foreign_keys,1)
      assert.equal(reopened.database.prepare('PRAGMA synchronous').get().synchronous,2) // FULL
      assert.equal(reopened.database.prepare('PRAGMA journal_mode').get().journal_mode,'wal')
    }finally{reopened.close()}
  }finally{await rm(dir,{recursive:true,force:true})}
})
