#!/usr/bin/env node
// Experimental reference CLI; production compiler/runtime entry is not yet wired.
import { readFile } from 'node:fs/promises'
import { runNqd, namedNqd, verifyNqd, migrateLegacyNoedb } from './noesql-store.mjs'
try {
  const [verb, database, input, ...args]=process.argv.slice(2)
  if(verb==='run' && database && input && args.length===0) {
    if(!input.endsWith('.nqds')&&!input.endsWith('.nqd')&&!input.endsWith('.noe')) throw Error('source must be .nqds (legacy .nqd/.noe supported)')
    const text=await readFile(input,'utf8')
    console.log(JSON.stringify(await runNqd(database,text),null,2))
  } else if(verb==='migrate' && database && input && args.length===0) {
    console.log(JSON.stringify(await migrateLegacyNoedb(database,input),null,2))
  } else if(verb==='verify' && database && !input && args.length===0) {
    console.log(JSON.stringify(await verifyNqd(database),null,2))
  } else if(verb==='query' && database && input) {
    const params=args.length===1?JSON.parse(args[0]):{}
    console.log(JSON.stringify(await namedNqd(database,input,params),null,2))
  } else {
    console.error('Usage: node Runtime/noesql-cli.mjs run <data.noedb> <source.nqds>\n       node Runtime/noesql-cli.mjs query <data.noedb> <query_name> [json_parameters]\n       node Runtime/noesql-cli.mjs verify <data.noedb>\n       node Runtime/noesql-cli.mjs migrate <legacy.nqdb> <new.noedb>')
    process.exitCode=2
  }
} catch(e) { console.error(e.message);process.exitCode=1 }
