import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import {encodeRow,decodeRow} from '../Runtime/noesql-row.mjs'
const schema=[{type:'int',primary:true},{type:'text',required:true},{type:'bool'},{type:'int'}]
test('ROW1 signed integer, UTF-8, booleans and null roundtrip',()=>{
  const input=[-9223372036854775808n,'Nairobi ☀',true,null]
  const bytes=encodeRow(schema,input,7)
  assert.equal(bytes.toString('ascii',0,4),'NROW')
  assert.deepEqual(decodeRow(bytes,schema,7).row,input)
  assert.equal(decodeRow(bytes,schema,7).consumed,bytes.length)
})
test('ROW1 fails closed on schema mismatch, truncation, malformed flags and booleans',()=>{
  const bytes=encodeRow(schema,[1n,'Ada',false,0n],2)
  assert.throws(()=>decodeRow(bytes,schema,3),/schema mismatch/)
  assert.throws(()=>decodeRow(bytes.subarray(0,bytes.length-1),schema,2),/truncated/)
  const bad=Buffer.from(bytes);bad[15]=2;assert.throws(()=>decodeRow(bad,schema,2),/marker/)
  const notBool=Buffer.from(bytes);const third=14+6+8+6+3+6;notBool[third]=2
  assert.throws(()=>decodeRow(notBool,schema,2),/boolean/)
})
test('ROW1 rejects null primary key and out-of-range scalar',()=>{
  assert.throws(()=>encodeRow(schema,[null,'Ada',true,null]),/required/)
  assert.throws(()=>encodeRow(schema,[2n**64n,'Ada',true,null]),RangeError)
})
