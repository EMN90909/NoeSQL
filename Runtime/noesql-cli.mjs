#!/usr/bin/env node
// Experimental reference CLI; production compiler/runtime entry is not yet wired.
import { readFile } from 'node:fs/promises'
import { runNqd, namedNqd } from './noesql-store.mjs'
try {
  const [verb, database, input, ...args]=process.argv.slice(2)
  if(verb==='run' && database && input && args.length===0) {
    const text=await readFile(input,'utf8'); if(!input.endsWith('.noe')) throw Error('source must be .noe')
    console.log(JSON.stringify(await runNqd(database,text),null,2))
  } else if(verb==='query' && database && input) {
    const params=args.length===1?JSON.parse(args[0]):{}
    console.log(JSON.stringify(await namedNqd(database,input,params),null,2))
  } else {
    console.error('Usage: node Runtime/noesql-cli.mjs run <data.nqd> <source.noe>\n       node Runtime/noesql-cli.mjs query <data.nqd> <query_name> [json_parameters]')
    process.exitCode=2
  }
} catch(e) { console.error(e.message);process.exitCode=1 }
