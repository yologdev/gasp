const {test}=require('node:test');const assert=require('node:assert/strict');require('../public/activity-labels.js');require('../public/activity-live.js');const L=global.YoyoActivityLabels,A=global.YoyoActivityLive;
const signature='4'.repeat(88);
test('checkpoints group by task domain but never count as publications',()=>{
 const idx=L.index([{kind:'observation.created',payload:{metadata:{producer:'yoyo-cloudflare',channel:'twitter',run_id:'r',tweet_id:'123',action:{reply_to:'1'}}}}]);
 const r={id:'r',task:'task_twitter_live_conversation_1',outcome:'checkpoint_saved',metadata:{domain:'cloudflare-task'}};
 const d=L.describe(r,idx);assert.equal(d.group,'twitter');assert.equal(d.isReply,false);assert.equal(d.isPost,false);assert.equal(L.group({task:'task_treasury_buyburn',metadata:{domain:'cloudflare-task'}}),'treasury');
});
test('standalone burn observations render and deduplicate against live receipts',()=>{
 const events=[{kind:'observation.created',ts_ms:100,payload:{metadata:{domain:'treasury',producer:'yoyo-treasury',receipt:{failed:false,signature,burnUnits:'1234567',spentLamports:'100000000'}}}}];
 const base=A.receipts(events);assert.equal(base.length,1);assert.equal(base[0].display.burned,1.234567);
 const merged=A.merge(base,{treasury:{burns:[{signature,burnUnits:'1234567',decimals:6,spentLamports:'100000000',at:200}],funding:[]}});
 assert.equal(merged.length,1);assert.equal(merged[0].ts,200);assert.equal(merged[0].group,'treasury');
});
test('live publications replace recorded copies without losing independent checkpoints',()=>{
 const runs=[{id:'r',outcome:'published',ts:1,display:{tweetId:'123'}},{id:'cp',outcome:'checkpoint_saved',ts:2,display:{tweetId:'123'}}];
 const merged=A.merge(runs,{twitter:{items:[{tweetId:'123',replyTo:'7',text:'Current reply',at:3,kind:'twitter'}]}});
 assert.equal(merged.length,2);assert.equal(merged[0].display.subtitle,'Current reply');assert.ok(merged.some(r=>r.id==='cp'));
});
test('live projection never exposes extra internal fields',async()=>{
 const {project}=await import('../worker.mjs');const secret='DO NOT EXPOSE';const out=project({asOf:1,items:[{tweetId:'1',text:'published',secret}],secret},{asOf:'date',secret,mint:secret},{burns:[{signature,secret,mint:secret}]},{funding:[{signature,secret,sourceWallet:secret}]});assert.ok(!JSON.stringify(out).includes(secret));
});
test('unsafe transaction and tweet URLs are rejected',()=>{
 assert.deepEqual(A.merge([],{twitter:{items:[{tweetId:'javascript:alert(1)'}]},treasury:{burns:[{signature:'javascript:alert(1)'}],funding:[]}}),[]);
});
