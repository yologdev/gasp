const {test}=require('node:test');const assert=require('node:assert/strict');
require('../public/activity-labels.js');const {index,describe,group,groupLabel}=global.YoyoActivityLabels;
test('checkpoint labels prefer folded titles and preserve task boundaries',()=>{
 const lookup=index([{kind:'task.created',payload:{id:'task_a',title:'Cloudflare task'}},{kind:'state.ops_applied',payload:[{UpdateNode:{id:'task_a',props:{title:'Review book themes'}}}]}]);
 assert.equal(describe({task:'task_a'},lookup).title,'Review book themes');
 assert.equal(describe({task:'task_yoyo_operating_charter_20260911',outcome:'checkpoint_saved',metadata:{parent_checkpoint_id:'cp_1'}},lookup).title,'Yoyo operating charter');
 assert.equal(describe({task:'task_other'},lookup).title,'Other');
});
test('public receipt enriches only its own Twitter task, regardless of event order',()=>{
 const receipt={kind:'observation.created',payload:{metadata:{channel:'twitter',producer:'yoyo-cloudflare',run_id:'run_social',parent_run_id:'run_parent',tweet_id:'123',input:{mention:{author:{username:'yuanhao'},text:'. @yoyoevolve What changed your mind?'}}}}};
 const lookup=index([receipt]);
 const label=describe({id:'run_cli',task:'task_twitter_parent_1'},lookup);
 assert.equal(label.title,'Reply to @yuanhao');assert.equal(label.subtitle,'What changed your mind?');assert.equal(label.url,'https://x.com/yoyoevolve/status/123');
 assert.equal(describe({task:'task_twitter_unrelated_1'},lookup).title,'Twitter task');
 assert.equal(describe({id:'run_social'},lookup).title,'Reply to @yuanhao');
});
test('missing receipts remain readable and untrusted values cannot become URLs',()=>{
 const lookup=index([{kind:'observation.created',payload:{metadata:{channel:'twitter',producer:'yoyo-cloudflare',run_id:'r',tweet_id:'javascript:alert(1)',input:{mention:{author:{username:'<img>'},text:'<script> hi </script>'}}}}}]);
 assert.equal(describe({id:'r'},lookup).url,'');assert.equal(describe({id:'r'},lookup).title,'Reply to a Twitter mention');
 assert.equal(describe({task:'task_twitter_x_1',outcome:'checkpoint_saved'},index([])).outcome,'Checkpoint saved');
 assert.equal(describe({task:'Evolve day 194'},index([])).title,'Evolve day 194');
});

test('runs are grouped by declared metadata, falling back to legacy id prefixes',()=>{
 // The stream declares its own taxonomy now — read it rather than guess.
 assert.equal(group({id:'run_uuid',metadata:{domain:'social',channel:'twitter'}}),'twitter');
 assert.equal(group({id:'run_uuid',metadata:{domain:'cloudflare-task'}}),'task');
 // Runs recorded before that metadata existed: a closed, shrinking set.
 assert.equal(group({id:'run_day181_20260828T005229Z'}),'evolve');
 assert.equal(group({id:'run_social_day181_x'}),'social');
 assert.equal(group({id:'run_skill_day181_x'}),'skill');
 assert.equal(group({id:'run_dream_day181_x'}),'dream');
 assert.equal(group({id:'run_genesis'}),'genesis');
 // An unrecognised run is never dropped — it gets its own tab.
 assert.equal(group({id:'run_1a5f8793-a47c'}),'other');
 // A category nobody has written code for still appears, correctly labelled.
 assert.equal(group({id:'run_x',metadata:{domain:'treasury'}}),'treasury');
 assert.equal(groupLabel('treasury'),'TREASURY');
 assert.equal(groupLabel('evolve'),'EVOLUTION');
 // Declared values are untrusted text: they must not become arbitrary markup.
 assert.equal(group({id:'run_x',metadata:{channel:'<img src=x>'}}),'imgsrcx');
});

test('describe reports the group and whether a Twitter run was a reply',()=>{
 const receipt={kind:'observation.created',payload:{metadata:{channel:'twitter',producer:'yoyo-cloudflare',run_id:'run_r',tweet_id:'1',action:{reply_to:'999'}}}};
 const lookup=index([receipt]);
 const reply=describe({id:'run_r',metadata:{domain:'social',channel:'twitter'}},lookup);
 assert.equal(reply.group,'twitter');assert.equal(reply.isReply,true);assert.equal(reply.isPost,false);
 const evolve=describe({id:'run_day181_x',task:'evolve session day 181'},index([]));
 assert.equal(evolve.group,'evolve');assert.equal(evolve.isReply,false);
});
