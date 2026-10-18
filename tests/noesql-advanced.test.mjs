import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {DatabaseSync} from 'node:sqlite'
import {compileAdvanced,createAdvancedQueryExecutor} from '../Runtime/noesql-advanced.mjs'
import {fromSQLiteDatabase} from '../Runtime/noesql-embedded.mjs'

function fixture(options={}) {
  const db=new DatabaseSync(':memory:')
  db.exec(`PRAGMA foreign_keys=ON;
    CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    CREATE TABLE users (id INTEGER PRIMARY KEY, team_id INTEGER NOT NULL REFERENCES teams(id), name TEXT NOT NULL, active INTEGER NOT NULL);
    CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), amount INTEGER NOT NULL, status TEXT NOT NULL);
    INSERT INTO teams VALUES (1,'Build'),(2,'Support');
    INSERT INTO users VALUES (1,1,'Ada',1),(2,1,'Bea',1),(3,2,'Cam',0);
    INSERT INTO orders VALUES (1,1,10,'paid'),(2,1,20,'paid'),(3,2,30,'open');
    CREATE INDEX idx_orders_user ON orders(user_id);
  `)
  return fromSQLiteDatabase(db,options)
}

test('readable left join + grouping + having + sort with bound values',()=>{
  const db=fixture()
  try{
    const result=db.query(`from users
      left join orders on users.id == orders.user_id
      select users.name as person, count(orders.id) as purchases, sum(orders.amount) as revenue
      where users.active == $active
      group users.name
      having purchases >= $minimum
      sort purchases desc, person asc
      take 10`,{active:true,minimum:1})
    assert.deepEqual(result,[{person:'Ada',purchases:2,revenue:30},{person:'Bea',purchases:1,revenue:30}])
  }finally{db.close()}
})

test('multiple joins and NULL filter have typed validation',()=>{
  const db=fixture()
  try{
    const result=db.query(`from orders
      inner join users on orders.user_id == users.id
      inner join teams on users.team_id == teams.id
      select teams.name as team, count(orders.id) as total
      where orders.status == $status
      group teams.name
      sort team
      take 3`,{status:'paid'})
    assert.deepEqual(result,[{team:'Build',total:2}])
    assert.deepEqual(db.query(`from users\nselect name\nwhere name is not null\nsort name\ntake 1`),[{name:'Ada'}])
  }finally{db.close()}
})

test('full and right joins include unmatched related rows',()=>{
  const db=fixture()
  try{
    const right=db.query(`from orders
      right join users on orders.user_id == users.id
      select users.name as person, orders.id as order_id
      sort person, order_id
      take 20`)
    assert.deepEqual(right,[{person:'Ada',order_id:1},{person:'Ada',order_id:2},{person:'Bea',order_id:3},{person:'Cam',order_id:null}])
    assert.deepEqual(db.query(`from users
      full join orders on users.id == orders.user_id
      select users.name as person, orders.id as order_id
      where users.name is null
      take 20`),[])
  }finally{db.close()}
})

test('query injection, unknown tables and ambiguous references fail closed',()=>{
  const db=fixture()
  try{
    for(const bad of [
      'from users; DROP TABLE users\nselect id',
      'from users\nselect *\n; DELETE FROM users',
      'from users\ninner join orders on users.id == orders.user_id\nselect id',
      'from unknown\nselect id',
      'from users\nselect name\nwhere name == "Ada"',
      'from users\nselect name\nwhere name == $missing',
      'from users\nselect count(id) as n, name',
      'from users\nselect name\ntake 99999'
    ])assert.throws(()=>db.query(bad),/NOE-QUERY/)
    assert.deepEqual(db.query('from users\nselect name\nwhere name == $name',{name:'Ada\' OR 1=1 --'}),[])
    assert.equal(db.database.prepare('SELECT count(*) AS n FROM users').get().n,3)
  }finally{db.close()}
})

test('transaction commits multiple writes atomically and rolls back errors',()=>{
  const db=fixture({maxMutations:2})
  try{
    assert.equal(db.transaction(tx=>{tx.insert('teams',{id:3,name:'Sales'});tx.insert('users',{id:4,team_id:3,name:'Dev',active:true});return 'ok'}),'ok')
    assert.deepEqual(db.query('from users\nselect name\nwhere name == $name',{name:'Dev'}),[{name:'Dev'}])
    assert.throws(()=>db.transaction(tx=>{tx.update('users',{name:'Broken'},{id:4});tx.insert('users',{id:5,team_id:99,name:'Oops',active:1})}),/FOREIGN KEY/)
    assert.deepEqual(db.query('from users\nselect name\nwhere id == $id',{id:4}),[{name:'Dev'}])
    assert.throws(()=>db.transaction(tx=>{tx.update('users',{name:'A'},{id:4});tx.update('users',{name:'B'},{id:4});tx.update('users',{name:'C'},{id:4})}),/mutation quota/)
    assert.equal(db.database.prepare('SELECT count(*) AS n FROM users').get().n,4)
    assert.throws(()=>db.transaction(async ()=>{}),/synchronous/)
    assert.equal(db.transaction(tx=>tx.update('users',{name:'D'},{id:4})),1)
  }finally{db.close()}
})

test('physical indexes are usable in plans and survive sqlite WAL restart',()=>{
  const db=fixture()
  try{
    db.createIndex('users',['name'],{name:'idx_users_name'})
    const plans=db.explain('from users\nselect name\nwhere name == $name',{name:'Ada'})
    assert.ok(plans.some(p=>String(p.detail).includes('idx_users_name')),JSON.stringify(plans))
    assert.deepEqual(db.query('from users\nselect name\nwhere name == $name',{name:'Ada'}),[{name:'Ada'}])
    assert.equal(db.verify().ok,true)
    assert.throws(()=>db.createIndex('users',['missing']),/unknown column/)
  }finally{db.close()}
})

test('row limits, invalid grouped columns, unknown input and unsupported expressions',()=>{
  const db=fixture({maxRows:2})
  try{
    assert.equal(db.query('from users\nselect name\nsort name').length,2)
    assert.throws(()=>db.query('from users\nselect name\ntake 3'),/invalid take\/skip/)
    assert.throws(()=>db.query('from users\nselect name, count(*) as n\ngroup id'),/must be grouped/)
    assert.throws(()=>db.query('from users\nselect name\nwhere name == $name',{name:{obj:1}}),/invalid parameter/)
    assert.throws(()=>db.query('from users\nselect name\norder by name'),/unsupported clause/)
  }finally{db.close()}
})
