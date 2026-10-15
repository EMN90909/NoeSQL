import { strict as assert } from 'node:assert'
import { test } from 'node:test'
import { lex, parse, execute, emptyDatabase, runNamed, NoeError } from '../Runtime/noesql-reference.mjs'
const schema = `
db school
table learners {
  id: int key auto
  name: text required
  age: int
  passed: bool = false
}
index learner_name on learners(name)
query adults(minAge: int = 18) {
  learners get id, name where age >= $minAge sort name
}
`
const apply = (source,db=emptyDatabase(),params={}) => execute(source,db,params)
test('lexer spans, comments and operator precedence',()=>{
  const tokens=lex('a get id where age >= 15 // note\n')
  assert.equal(tokens.find(x=>x.value==='>=').line,1)
  assert.equal(parse('learners get id where 2 + 3 * 4 == 14')[0].where.left.op,'+')
  assert.throws(()=>lex('"oops'),e=>e.code==='NOE002')
})
test('schema, prepared named query, CRUD and filtering',()=>{
  let {database:db}=apply(schema)
  ;({database:db}=apply('learners add {name:"Amina",age:16}\nlearners add {name:"Bea",age:21}',db))
  assert.deepEqual(runNamed(db,'adults').rows,[{id:2,name:'Bea'}])
  assert.deepEqual(runNamed(db,'adults',{minAge:15}).rows,[{id:1,name:'Amina'},{id:2,name:'Bea'}])
  let response=apply('learners where id == 1 set passed = true\nlearners get name, passed where passed == true',db)
  assert.equal(response.results[0].affected,1)
  assert.deepEqual(response.results[1].rows,[{name:'Amina',passed:true}])
  assert.equal(apply('learners where id == 2 delete',response.database).results[0].affected,1)
})
test('reject unknown fields, type mismatches, duplicates, unsafe deletion',()=>{
  let {database:db}=apply(schema)
  ;({database:db}=apply('learners add {name:"Amina",age:16}',db))
  for (const src of ['learners get missing','learners add {name:99}','learners add {name:"B", id:1}','learners where id == 1 set age = "oops"','learners get id where age == "x"','learners delete'])
    assert.throws(()=>apply(src,db), e=>e instanceof NoeError,src)
  assert.equal(db.tables.learners.rows.length,1)
})
test('atomic block rolls back on constraint failure',()=>{
  const {database:db}=apply(schema)
  assert.throws(()=>apply('atomic {\nlearners add {name:"A",id:1}\nlearners add {name:"B",id:1}\n}',db),e=>e.code==='NOE203')
  assert.equal(db.tables.learners.rows.length,0)
})
test('reference validation and restrict delete',()=>{
  const {database:db}=apply('table users { id: int key auto\n name: text required\n}\ntable orders { id: int key auto\n buyer: ref users required\n}')
  let state=apply('users add {name:"A"}\norders add {buyer:1}',db).database
  assert.throws(()=>apply('users where id == 1 delete',state),e=>e.code==='NOE208')
  assert.throws(()=>apply('orders add {buyer:99}',state),e=>e.code==='NOE205')
})
test('source and parameter diagnostics',()=>{
  assert.throws(()=>apply('table t { id: money key }'),e=>e.code==='NOE104')
  assert.throws(()=>apply('table t { id: int key\nid: bool }'),e=>e.code==='NOE103')
  assert.throws(()=>apply('table t { id: int key\n'),e=>e.code==='NOE011')
  assert.throws(()=>apply('table t { id: int key }\nquery q(x: int){t get id where id == $missing}'),e=>e.code==='NOE121')
})
