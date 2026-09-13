export function project(twitter,summary,burns,funding){
 const out={asOf:Date.now(),twitter:null,treasury:null};
 if(twitter)out.twitter={asOf:twitter.asOf,replies:twitter.replies,posts:twitter.posts,pending:twitter.pending,since:twitter.since,items:(twitter.items||[]).map(p=>({tweetId:p.tweetId,text:p.text,replyTo:p.replyTo,at:p.at,kind:p.kind}))};
 if(summary&&burns&&funding)out.treasury={asOf:summary.asOf,balanceLamports:summary.balanceLamports,burnedTokens:summary.burnedTokens,confirmedBurns:summary.confirmedBurns,spentLamports:summary.spentLamports,
  burns:(burns.burns||[]).map(b=>({signature:b.signature,at:b.at,slot:b.slot,burnUnits:b.burnUnits,decimals:b.decimals,spentLamports:b.spentLamports})),
  funding:(funding.funding||[]).map(f=>({signature:f.signature,at:f.at,lamports:f.lamports,allocation:f.allocation}))};
 return out;
}
export default {async fetch(request,env,ctx){
 const url=new URL(request.url);if(url.pathname!=='/api/activity')return env.ASSETS.fetch(request);
 if(request.method!=='GET')return new Response('Method not allowed',{status:405});
 const key=new Request(url.origin+'/api/activity'),cached=await caches.default.match(key);if(cached)return cached;
 const values=await Promise.allSettled([env.TWITTER.activity(),env.TREASURY.summary(),env.TREASURY.burns(),env.TREASURY.funding()]);
 const out=project(...values.map(v=>v.status==='fulfilled'?v.value:null));
 const response=Response.json(out,{headers:{'Cache-Control':out.twitter&&out.treasury?'public, max-age=30':'no-store','X-Content-Type-Options':'nosniff'}});
 if(out.twitter&&out.treasury)ctx.waitUntil(caches.default.put(key,response.clone()));return response;
}};
