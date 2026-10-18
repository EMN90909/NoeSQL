import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {DatabaseSync} from 'node:sqlite'
import {fromSQLiteDatabase,openNoeSQLEmbedded} from '../Runtime/noesql-embedded.mjs'
import {mkdtemp,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'

test('embedded public CRUD/index API cannot mutate or read reserved metadata',()=>{
  const raw=new DatabaseSync(':memory:')
  raw.exec('CREATE TABLE _noesql_catalog(seq INTEGER PRIMARY KEY, definition TEXT NOT NULL); CREATE TABLE public_items(id INTEGER PRIMARY KEY, title TEXT);')
  const store=fromSQLiteDatabase(raw)
  try {
    assert.equal(store.backend,'sqlite-memory')
    assert.throws(()=>store.insert('_noesql_catalog',{definition:'corrupt'}),/invalid identifier/)
    assert.throws(()=>store.update('_noesql_catalog',{definition:'corrupt'},{seq:1}),/invalid identifier/)
    assert.throws(()=>store.remove('_noesql_catalog',{seq:1}),/invalid identifier/)
    assert.throws(()=>store.createIndex('_noesql_catalog',['seq']),/invalid identifier/)
    assert.equal(raw.prepare('SELECT count(*) AS n FROM _noesql_catalog').get().n,0)
    assert.equal(store.insert('public_items',{id:1,title:'safe'}),1)
  }finally{store.close()}
})
test('persistent embedded backend verifies WAL/FULL and protects catalog across restart',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'noesql-catalog-protect-'))
  const file=join(dir,'safe.sqlite')
  try{
    const first=await openNoeSQLEmbedded({file})
    try{
      assert.equal(first.backend,'sqlite-wal')
      first.run('table records {id: int key auto\n label: text required\n}')
      assert.throws(()=>first.remove('_noesql_catalog',{seq:1}),/invalid identifier/)
      assert.throws(()=>first.insert('_noesql_catalog',{definition:'bogus'}),/invalid identifier/)
    }finally{first.close()}
    const second=await openNoeSQLEmbedded({file})
    try{
      assert.equal(second.verify().ok,true)
      second.run('records add {label:"reopened"}')
      assert.deepEqual(second.query('from records\nselect label'),[{label:'reopened'}])
    }finally{second.close()}
  }finally{await rm(dir,{recursive:true,force:true})}
})
