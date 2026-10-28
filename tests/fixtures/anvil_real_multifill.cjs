/* Disposable Base Sepolia Anvil fork fixture. Never points at a public RPC. */
const fs = require('node:fs');
const path = require('node:path');
const { randomBytes } = require('node:crypto');
const { createRequire } = require('node:module');

const fromFrontend = createRequire(path.resolve(__dirname, '../../../deopt-v2-frontend/package.json'));
const { encodeFunctionData, encodeDeployData, decodeFunctionResult, hashTypedData, keccak256, parseAbi } = fromFrontend('viem');
const { privateKeyToAccount } = fromFrontend('viem/accounts');

const RPC = 'http://127.0.0.1:18545';
const D1 = '0x6B3D846536116082dC4E7227861C341Bb85Ee963';
const D2 = '0x9947F42cC29A32992bbf2030ee49E35C839EDEd5';
const D4 = '0xd0901DE8f6de72aecC716CA1465C4Cf58a0B99fB';
const D6 = '0xeeA4e5d891B90D3d99E9C4e2c61069911477110E';
const VAULT = '0x00340C360353a5AB784c5Bc5c44322A6AF0625D3';
const USDC = '0x6eAe407f5640B006faC9965182e238582A3B412E';
const TL = '0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588';
const USDC_OWNER = '0xc35F7A8A103A9A4464adfaa76B9B514093D23C27';
const matcherArtifact = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../../deopt-v2-sol/out/PerpMatchingEngineV2.sol/PerpMatchingEngineV2.json')));
const wrapperArtifact = JSON.parse(fs.readFileSync('/tmp/deopt_anvil_forge_out/AnvilIntentBatchCaller.sol/AnvilIntentBatchCaller.json'));
const oracleSourceArtifact = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../../deopt-v2-sol/out/MockPriceSource.sol/MockPriceSource.json')));
const ORACLE_ROUTER = '0xB416406F200B2Ef3D7a86A5D5877Ed41D9B1A581';
const UNDERLYING = '0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02';
const wrapperAbi = parseAbi(['function execute(address target, bytes[] calls)']);
const engineAbi = parseAbi(['function getMarkPrice(uint256 marketId) view returns (uint256)']);
const intentTypes = { PerpOrderIntent: [
  { name:'intentId', type:'bytes32' }, { name:'trader', type:'address' },
  { name:'subaccountId', type:'uint32' }, { name:'marketId', type:'uint256' },
  { name:'side', type:'uint8' }, { name:'size1e8', type:'uint128' },
  { name:'limitPrice1e8', type:'uint128' }, { name:'maxExecPrice1e8', type:'uint128' },
  { name:'minExecPrice1e8', type:'uint128' }, { name:'nonce', type:'uint256' },
  { name:'deadline', type:'uint256' },
] };
const domain = { name:'DeOptV2-PerpMatchingEngine', version:'2', chainId:84532, verifyingContract:D6 };

async function rpc(method, params=[]) {
  const response = await fetch(RPC, { method:'POST', headers:{'content-type':'application/json'}, body:JSON.stringify({jsonrpc:'2.0',id:1,method,params}) });
  const payload = await response.json();
  if (payload.error) throw new Error(`${method}: ${JSON.stringify(payload.error)}`);
  return payload.result;
}
async function receipt(hash) {
  for (let i=0; i<60; i++) {
    const found = await rpc('eth_getTransactionReceipt',[hash]);
    if (found) return found;
    await new Promise(resolve=>setTimeout(resolve,200));
  }
  throw new Error(`missing local receipt ${hash}`);
}
async function send(from,to,data,label,gas='0x4c4b40') {
  const hash=await rpc('eth_sendTransaction',[{from,to,data,gas}]);
  const result=await receipt(hash);
  if (BigInt(result.status)!==1n) throw new Error(`${label} reverted in ${hash}`);
  process.stdout.write(`${label} ${hash}\n`);
  return result;
}
function encode(signature,args) {
  return encodeFunctionData({abi:parseAbi([signature]),functionName:signature.match(/function (\w+)/)[1],args});
}
function makeIntent(account,side,size,price,nonce,deadline) {
  return {
    intentId:`0x${randomBytes(32).toString('hex')}`, trader:account.address, subaccountId:0,
    marketId:1n, side, size1e8:size, limitPrice1e8:0n,
    maxExecPrice1e8:side===0?price:0n, minExecPrice1e8:side===1?price:0n,
    nonce:BigInt(nonce), deadline,
  };
}
async function main() {
  if (!(await rpc('web3_clientVersion')).toLowerCase().includes('anvil')) throw new Error('local Anvil required');
  if (BigInt(await rpc('eth_chainId'))!==84532n) throw new Error('wrong local chain');
  const code=await rpc('eth_getCode',[D6,'latest']);
  if (code.length<100) throw new Error('canonical D6 missing in local fork');
  // The production market intentionally has no launch execution band yet.
  // Set a wide band only on this disposable fork to reach the fee path.
  await send(TL,D1,encode('function setMaxExecutionDeviationBps(uint256,uint16)',[1n,10_000]),'local_execution_band');
  const users=[0,1,2].map(()=>privateKeyToAccount(`0x${randomBytes(32).toString('hex')}`));
  const [buyer,sellerA,sellerB]=users;
  for (const user of users) {
    await rpc('anvil_setBalance',[user.address,'0x56bc75e2d63100000']);
    await rpc('anvil_impersonateAccount',[user.address]);
    await send(USDC_OWNER,USDC,encode('function mint(address,uint256)',[user.address,10_000_000_000n]),'mint');
    await send(user.address,USDC,encode('function approve(address,uint256)',[VAULT,10_000_000_000n]),'approve');
    await send(user.address,VAULT,encode('function deposit(address,uint256)',[USDC,10_000_000_000n]),'deposit');
  }
  const deployHash=await rpc('eth_sendTransaction',[{from:TL,data:wrapperArtifact.bytecode.object,gas:'0x300000'}]);
  const deployed=await receipt(deployHash);
  if (BigInt(deployed.status)!==1n) throw new Error('local wrapper deploy reverted');
  const wrapper=deployed.contractAddress;
  await send(TL,D6,encode('function setExecutor(address,bool)',[wrapper,true]),'wrapper_executor');
  // The fork's old external oracle samples become stale as tests run. Replace
  // both sources only in this disposable Anvil state, preserving dual-source
  // validation while making the fixture independent of elapsed wall time.
  const sourceAddresses=[];
  for (let i=0;i<2;i++) {
    const latest=await rpc('eth_getBlockByNumber',['latest',false]);
    const data=encodeDeployData({abi:oracleSourceArtifact.abi,bytecode:oracleSourceArtifact.bytecode.object,args:[237330000000n,BigInt(latest.timestamp)]});
    const deployment=await receipt(await rpc('eth_sendTransaction',[{from:TL,data,gas:'0x300000'}]));
    if (BigInt(deployment.status)!==1n || !deployment.contractAddress) throw new Error('local oracle source deployment failed');
    sourceAddresses.push(deployment.contractAddress);
  }
  await send(TL,ORACLE_ROUTER,encode('function setFeed(address,address,address,address,uint32,uint16,bool)',
    [UNDERLYING,USDC,sourceAddresses[0],sourceAddresses[1],3600,500,true]),'local_fresh_dual_oracle');
  const markData=encodeFunctionData({abi:engineAbi,functionName:'getMarkPrice',args:[1n]});
  const price=decodeFunctionResult({abi:engineAbi,functionName:'getMarkPrice',data:await rpc('eth_call',[{to:D4,data:markData},'latest'])});
  const block=await rpc('eth_getBlockByNumber',['latest',false]);
  const deadline=BigInt(block.timestamp)+3600n;
  // One signed maker order is filled twice (1 + 2 units) against distinct
  // takers. D6's intentFilled mapping must end at exactly 3 units.
  const buyA=makeIntent(buyer,0,300_000_000n,price,1,deadline);
  const sellA=makeIntent(sellerA,1,100_000_000n,price,1,deadline);
  const buyB=buyA;
  const sellB=makeIntent(sellerB,1,200_000_000n,price,1,deadline);
  const calls=[];
  for (const [buy,sell,size] of [[buyA,sellA,100_000_000n],[buyB,sellB,200_000_000n]]) {
    const buySig=await users.find(x=>x.address===buy.trader).signTypedData({domain,types:intentTypes,primaryType:'PerpOrderIntent',message:buy});
    const sellSig=await users.find(x=>x.address===sell.trader).signTypedData({domain,types:intentTypes,primaryType:'PerpOrderIntent',message:sell});
    const hashCall=encodeFunctionData({abi:matcherArtifact.abi,functionName:'hashOrderIntent',args:[buy]});
    const onchainHash=await rpc('eth_call',[{to:D6,data:hashCall},'latest']);
    const signedHash=hashTypedData({domain,types:intentTypes,primaryType:'PerpOrderIntent',message:buy});
    if (onchainHash.toLowerCase()!==signedHash.toLowerCase()) throw new Error('EIP-712 hash mismatch');
    calls.push(encodeFunctionData({abi:matcherArtifact.abi,functionName:'executeTradeFromIntents',args:[buy,buySig,sell,sellSig,{marketId:1n,buyerSubaccountId:0,sellerSubaccountId:0,size1e8:size,executionPrice1e8:price,buyerIsMaker:true,buyerIntentId:buy.intentId,sellerIntentId:sell.intentId}]}));
  }
  const data=encodeFunctionData({abi:wrapperAbi,functionName:'execute',args:[D6,calls]});
  const localSender=(await rpc('eth_accounts'))[2];
  try { await rpc('eth_call',[{from:localSender,to:wrapper,data,gas:'0x989680'},'latest']); }
  catch(e) { throw new Error(`local multi-fill simulation failed: ${e.message}`); }
  const preTradeSnapshot=await rpc('evm_snapshot');
  const actual=await send(localSender,wrapper,data,'REAL_LOCAL_MULTIFILL','0x989680');
  const buyerIntentHash=hashTypedData({domain,types:intentTypes,primaryType:'PerpOrderIntent',message:buyA});
  const filledData=encode('function intentFilled(bytes32) view returns(uint128)',[buyerIntentHash]);
  if (BigInt(await rpc('eth_call',[{to:D6,data:filledData},'latest']))!==300_000_000n) throw new Error('local partial fill tally mismatch');
  const transaction=await rpc('eth_getTransactionByHash',[actual.transactionHash]);
  const trace=await rpc('debug_traceTransaction',[actual.transactionHash,{tracer:'callTracer'}]);
  if (trace.error || trace.to.toLowerCase()!==wrapper.toLowerCase() || trace.calls?.length!==calls.length ||
      trace.calls.some((child,i)=>child.error || child.to.toLowerCase()!==D6.toLowerCase() || child.input.toLowerCase()!==calls[i].toLowerCase())) {
    throw new Error('local trace does not authenticate both nested D6 calls');
  }
  // The production D2 reader deliberately pins the safe block, so move the
  // disposable fork's safe tag past this receipt before asking Rust to ingest.
  await rpc('anvil_mine',['0x40','0x1']);
  const sources={};for(const log of actual.logs) sources[log.address.toLowerCase()]=(sources[log.address.toLowerCase()]||0)+1;
  const bundle={receipt:actual,transaction,trace,fixture:{wrapper,chainId:84532,price:price.toString(),buyers:[buyer.address],sellers:[sellerA.address,sellerB.address],sizes:['100000000','200000000'],buyerIntentHash,finalBuyerIntentFilled:'300000000',callHashes:calls.map(call=>keccak256(call)),preTradeSnapshot,D1,D2,D4,D6,USDC}};
  fs.writeFileSync('/tmp/deopt_anvil_multifill_actual.json',JSON.stringify(bundle));
  process.stdout.write(`REAL_LOCAL_MULTIFILL_PROOF ${JSON.stringify({txHash:actual.transactionHash,blockHash:actual.blockHash,blockNumber:actual.blockNumber,logCount:actual.logs.length,emitterCounts:sources,price:price.toString(),wallets:[buyer.address,sellerA.address,sellerB.address]})}\n`);
}
main().catch(e=>{console.error(e.message);process.exitCode=1;});
