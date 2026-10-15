// Portable ROW1 codec used to independently check the native .nqr wire contract.
// This is a conformance/reference implementation, not the native runtime.
const MAGIC='NROW', HEAD=14, MAX_COLS=256
const kind={int:1,bool:2,text:3}
const check=(ok,message)=>{if(!ok)throw Error(`NOE-ROW: ${message}`)}
export function encodeRow(columns,row,version=1){
  check(columns.length>0&&columns.length<=MAX_COLS,'field count')
  check(Number.isInteger(version)&&version>0&&version<=0xffffffff,'schema version')
  const parts=[]
  for(const [i,col] of columns.entries()){
    const type=kind[col.type],value=row[i]
    check(type,'type');check(value!==null||(!col.required&&!col.primary),'required field')
    let data=Buffer.alloc(0)
    if(value!==null){
      if(type===1){check(typeof value==='bigint'||Number.isSafeInteger(value),'int type');data=Buffer.alloc(8);data.writeBigInt64LE(BigInt(value))}
      if(type===2){check(typeof value==='boolean','bool type');data=Buffer.alloc(8);data.writeBigInt64LE(value?1n:0n)}
      if(type===3){check(typeof value==='string','text type');data=Buffer.from(value,'utf8')}
    }
    const header=Buffer.alloc(6);header[0]=type;header[1]=value===null?1:0;header.writeUInt32LE(data.length,2);parts.push(header,data)
  }
  const payload=Buffer.concat(parts),head=Buffer.alloc(HEAD)
  check(payload.length<=0xffffffff,'payload too large')
  head.write(MAGIC);head.writeUInt32LE(version,4);head.writeUInt16LE(columns.length,8);head.writeUInt32LE(payload.length,10)
  return Buffer.concat([head,payload])
}
export function decodeRow(input,columns,expectedVersion=1){
  const b=Buffer.from(input)
  check(b.length>=HEAD&&b.toString('ascii',0,4)===MAGIC,'magic')
  check(b.readUInt32LE(4)===expectedVersion,'schema mismatch')
  check(b.readUInt16LE(8)===columns.length&&columns.length>0,'column mismatch')
  const finish=b.readUInt32LE(10)+HEAD
  check(finish<=b.length,'truncated row')
  const row=[];let p=HEAD
  for(const col of columns){
    check(p+6<=finish,'field header truncated')
    const type=b[p],nullBit=b[p+1],size=b.readUInt32LE(p+2);p+=6
    check(type===kind[col.type]&&(nullBit===0||nullBit===1),'field type/null marker')
    check(size<=finish-p,'field body truncated')
    if(nullBit===1){check(size===0&&!col.required&&!col.primary,'invalid NULL');row.push(null)}
    else if(type===1){check(size===8,'invalid integer width');row.push(b.readBigInt64LE(p))}
    else if(type===2){check(size===8,'invalid boolean width');const n=b.readBigInt64LE(p);check(n===0n||n===1n,'invalid boolean');row.push(n===1n)}
    else if(type===3){const text=b.toString('utf8',p,p+size);check(Buffer.from(text,'utf8').equals(b.subarray(p,p+size)),'invalid UTF-8');row.push(text)}
    p+=size
  }
  check(p===finish,'extra row data')
  return {row,consumed:finish}
}
