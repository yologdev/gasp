/* Presentation only: public event metadata supplies labels, never private sessions. */
(function (root) {
  function clean(value, limit) {
    if (typeof value !== 'string') return '';
    var text = value.replace(/[\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim();
    if (text.length <= limit) return text;
    return text.slice(0, limit - 1).replace(/\s+\S*$/, '') + '…';
  }
  function fallback(task) {
    if (/^task_twitter_/.test(task)) return 'Twitter task';
    if (!/^task_/.test(task)) return clean(task, 140) || 'Yoyo task';
    var words = task.slice(5).replace(/_\d{8}(?:T\d+Z)?$/, '').replace(/_/g, ' ');
    if (/^[a-f0-9-]{32,}$/i.test(words)) return 'Yoyo task';
    words = clean(words, 140);
    return words ? words[0].toUpperCase() + words.slice(1) : 'Yoyo task';
  }
  function index(events) {
    var tasks = new Map(), social = new Map();
    events.forEach(function (event) {
      var p = event.payload || {};
      if (event.kind === 'task.created' && p.id) tasks.set(p.id, p);
      if (event.kind === 'state.ops_applied' && Array.isArray(p)) p.forEach(function (op) {
        var node = op.CreateNode || op.UpdateNode;
        if (node && (op.CreateNode ? node.kind === 'task' : tasks.has(node.id)))
          tasks.set(node.id, Object.assign({}, tasks.get(node.id), node.props));
        if (op.TombstoneNode) tasks.delete(op.TombstoneNode.id);
      });
      if (event.kind === 'observation.created') {
        var record = p.metadata;
        if (record && record.channel === 'twitter' && record.producer === 'yoyo-cloudflare') {
          if (record.run_id) social.set(record.run_id, record);
          if (record.parent_run_id) social.set(record.parent_run_id, record);
        }
      }
    });
    return {tasks: tasks, social: social};
  }
  function describe(run, lookup) {
    var task = String(run.task || ''), props = lookup.tasks.get(task) || {};
    var match = /^task_twitter_(.+)_\d+$/.exec(task);
    var receipt = lookup.social.get(run.id) || (match && lookup.social.get('run_' + match[1]));
    var explicit = clean(props.title, 140);
    var title = explicit && explicit !== 'Cloudflare task' && explicit !== 'Twitter interaction' && explicit !== task ? explicit : fallback(task);
    var subtitle = '', url = '';
    if (receipt) {
      var mention = receipt.input && receipt.input.mention;
      if (mention) {
        var username = mention.author && mention.author.username;
        title = 'Reply' + (/^[A-Za-z0-9_]{1,15}$/.test(username || '') ? ' to @' + username : ' to a Twitter mention');
        subtitle = clean(String(mention.text || '').replace(/@yoyoevolve\b/ig, '').replace(/^[\s.,:—-]+/, ''), 160);
      } else title = receipt.action && receipt.action.reply_to ? 'Twitter reply' : 'Twitter post';
      if (/^\d+$/.test(receipt.tweet_id || '')) url = 'https://x.com/yoyoevolve/status/' + receipt.tweet_id;
    }
    return {title: title, subtitle: subtitle, url: url,
      category: match || receipt ? 'TWITTER' : /^task_/.test(task) ? 'TASK' : null,
      outcome: run.outcome === 'checkpoint_saved' ? 'Checkpoint saved' : String(run.outcome || 'in progress').replace(/_/g, ' '),
      resumed: Boolean(run.metadata && run.metadata.parent_checkpoint_id)};
  }
  root.YoyoActivityLabels = {index: index, describe: describe};
})(typeof window === 'undefined' ? globalThis : window);
