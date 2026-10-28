/* Local-only RPC fault injector for trace availability tests. */
const http = require('node:http');
const UPSTREAM = 'http://127.0.0.1:18545';
const mode = process.env.DEOPT_TRACE_PROXY_MODE;
if (!['missing', 'malformed'].includes(mode)) throw new Error('select missing or malformed trace mode');

async function main() {
  const probe = await fetch(UPSTREAM, {method:'POST',headers:{'content-type':'application/json'},
    body:JSON.stringify({jsonrpc:'2.0',id:1,method:'web3_clientVersion',params:[]})}).then(x=>x.json());
  if (!probe.result?.toLowerCase().includes('anvil')) throw new Error('local Anvil upstream required');
  http.createServer(async (request,response) => {
    try {
      if (request.method !== 'POST') {response.writeHead(405);response.end();return;}
      let text='';
      for await (const chunk of request) {text+=chunk;if(text.length>1024*1024) throw new Error('RPC body too large');}
      const payload=JSON.parse(text);
      let result;
      if (payload.method === 'debug_traceTransaction') {
        result = mode === 'missing'
          ? {jsonrpc:'2.0',id:payload.id,error:{code:-32601,message:'Method not found'}}
          : {jsonrpc:'2.0',id:payload.id,result:{type:'CALL',calls:[]}};
      } else {
        result=await fetch(UPSTREAM,{method:'POST',headers:{'content-type':'application/json'},body:text}).then(x=>x.json());
      }
      response.writeHead(200,{'content-type':'application/json'});response.end(JSON.stringify(result));
    } catch {
      response.writeHead(502,{'content-type':'application/json'});
      response.end(JSON.stringify({jsonrpc:'2.0',id:null,error:{code:-32000,message:'local proxy failure'}}));
    }
  }).listen(18546,'127.0.0.1',()=>process.stdout.write(`LOCAL_TRACE_PROXY_READY mode=${mode}\n`));
}
main().catch(error=>{console.error(error.message);process.exitCode=1});
