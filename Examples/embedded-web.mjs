// Example of a private NoeSQL API behind your application's TLS reverse proxy.
// Never ship NOESQL_READER_KEY or NOESQL_WRITER_KEY to browser code.
import {createServer} from 'node:http'
import {openNoeSQLEmbedded} from '../Runtime/noesql-embedded.mjs'
import {createNoeSQLHttpHandler} from '../Runtime/noesql-web.mjs'

const location=process.env.NOESQL_DB_PATH
const reader=process.env.NOESQL_READER_KEY
const writer=process.env.NOESQL_WRITER_KEY
const tables=process.env.NOESQL_ALLOWED_TABLES?.split(',').map(x=>x.trim()).filter(Boolean)
if(!location||!reader||!writer||!tables?.length)throw Error('Set NOESQL_DB_PATH (absolute .sqlite), NOESQL_READER_KEY, NOESQL_WRITER_KEY and NOESQL_ALLOWED_TABLES')
const store=await openNoeSQLEmbedded({file:location,maxRows:100})
const handler=createNoeSQLHttpHandler({database:store,keys:[{token:reader,role:'reader',tables},{token:writer,role:'writer',tables}],requestsPerMinute:60})
const host='127.0.0.1' // Must be behind an authenticated application and TLS.
const port=Number(process.env.PORT||3030)
const server=createServer(handler)
server.listen(port,host,()=>console.log(`NoeSQL private HTTP API listening on ${host}:${port}`))
for(const sig of ['SIGINT','SIGTERM'])process.on(sig,()=>server.close(()=>{store.close();process.exit(0)}))
