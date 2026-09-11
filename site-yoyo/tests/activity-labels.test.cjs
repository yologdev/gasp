const {test}=require('node:test');const assert=require('node:assert/strict');
require('../public/activity-labels.js');const {index,describe}=global.YoyoActivityLabels;
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
