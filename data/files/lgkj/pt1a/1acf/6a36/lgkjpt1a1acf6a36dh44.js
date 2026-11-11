var me = this;
var ME = $('#' + me.UUID)[0];

me.ready = function(){
  // peer '' = this instance's own share; otherwise the uuid of the peer being browsed
  var state = { path: '', peer: '', peers: [] };

  var $app    = $(ME).find('.fb');
  var $crumbs = $(ME).find('.fb-crumbs');
  var $list   = $(ME).find('.fb-list');
  var $status = $(ME).find('.fb-status');
  var $upload = $(ME).find('.fb-file');
  var $peers  = $(ME).find('.fb-peers');

  function say(msg, isErr){
    $status.text(msg || '').toggleClass('is-err', !!isErr);
  }

  // A peer's files commands are reached through the peer proxy, over the encrypted peer channel.
  function base(){
    return state.peer ? '/peer/remote/' + state.peer + '/files/' : '/files/';
  }

  function listDir(path, cb){
    if (!state.peer) return send_list_dir(path, cb);
    $.getJSON(base() + 'list_dir', { path: path }).done(cb).fail(function(){
      cb({ status: 'err', msg: 'This peer is offline, does not run the files app, or has not put you in its "files" group.' });
    });
  }

  function shareName(){
    if (!state.peer) return 'My share';
    for (var i = 0; i < state.peers.length; i++) if (state.peers[i].id === state.peer) return state.peers[i].name || state.peer;
    return state.peer;
  }

  // ---- shares: mine, then every peer ----
  function renderPeers(){
    $peers.empty();
    var row = function(id, name, on){
      var $r = $('<div class="fb-peer"></div>').toggleClass('on', id === state.peer).toggleClass('off', !on);
      $r.append($('<span class="fb-dot"></span>').toggleClass('on', on));
      $r.append($('<span class="fb-peer-name"></span>').text(name));
      if (on) $r.on('click', function(){ pick(id); });
      $peers.append($r);
    };
    row('', 'My share', true);
    state.peers.forEach(function(p){ row(p.id, p.name || p.id, !!p.connected); });
    $(ME).find('.fb-none').attr('hidden', state.peers.length ? true : null);
  }

  function loadPeers(){
    $.getJSON('/peer/peers').done(function(r){
      var peers = Array.isArray(r) ? r : (r && Array.isArray(r.data) ? r.data : []);
      peers.sort(function(a, b){
        return (!!b.connected - !!a.connected) || String(a.name || a.id).localeCompare(String(b.name || b.id));
      });
      state.peers = peers;
      renderPeers();
      renderCrumbs(state.path); // the root crumb carries the peer's name
    });
  }

  function pick(id){
    state.peer = id;
    $app.addClass('open');
    $(ME).find('.fb-upload, .fb-dav').attr('hidden', id ? true : null); // uploads and the DAV mount are for your own share
    renderPeers();
    go('');
  }

  $(ME).find('.fb-back').on('click', function(){ $app.removeClass('open'); });

  function fmtSize(n){
    if (n === null || n === undefined) return '';
    if (n < 1024) return n + ' B';
    var u = ['KB','MB','GB','TB'], i = -1, v = n;
    do { v = v / 1024; i++; } while (v >= 1024 && i < u.length - 1);
    return (v < 10 ? v.toFixed(1) : Math.round(v)) + ' ' + u[i];
  }

  function fmtTime(ms){
    if (!ms) return '';
    var d = new Date(ms);
    if (isNaN(d.getTime())) return '';
    var p = function(x){ return (x < 10 ? '0' : '') + x; };
    return d.getFullYear() + '-' + p(d.getMonth()+1) + '-' + p(d.getDate()) +
           ' ' + p(d.getHours()) + ':' + p(d.getMinutes());
  }

  function joinPath(dir, name){
    if (!dir) return name;
    return dir.replace(/\/+$/, '') + '/' + name;
  }

  // The backend answers paths like "./agent"; strip the leading "./" so the
  // breadcrumb shows "share / agent" and not "share / . / agent".
  function cleanPath(p){
    var s = String(p || '').replace(/\\/g, '/');
    while (s.indexOf('./') === 0) s = s.slice(2);
    s = s.replace(/^\/+/, '').replace(/\/+$/, '');
    return s === '.' ? '' : s;
  }

  function renderCrumbs(p){
    $crumbs.empty();
    var parts = cleanPath(p).split('/').filter(function(s){ return s.length; });
    var acc = '';
    var add = function(label, target, here){
      if ($crumbs.children().length) $crumbs.append('<span class="fb-sep">/</span>');
      var $a = $('<a class="fb-crumb"></a>').text(label);
      if (here) $a.addClass('is-here');
      else $a.on('click', function(){ go(target); });
      $crumbs.append($a);
    };
    add(shareName(), '', parts.length === 0);
    parts.forEach(function(seg, idx){
      acc = acc ? acc + '/' + seg : seg;
      add(seg, acc, idx === parts.length - 1);
    });
  }

  function renderEntries(entries){
    $list.empty();
    if (!entries || !entries.length){
      $list.append($('<div class="fb-empty"></div>').text('This folder is empty.'));
      return;
    }
    entries.forEach(function(e){
      var isDir = !!e.isdir;
      var $row = $('<div class="fb-row"></div>').toggleClass('is-dir', isDir);
      $row.append($('<span class="fb-icon"></span>').text(isDir ? '▸' : '·'));
      $row.append($('<span class="fb-name"></span>').text(e.name));
      $row.append($('<span class="fb-size"></span>').text(isDir ? '' : fmtSize(e.size)));
      $row.append($('<span class="fb-mtime"></span>').text(fmtTime(e.mtime)));
      if (isDir){
        $row.append('<span></span>');
        $row.on('click', function(){ go(e.path); });
      } else {
        $row.append($('<a class="fb-dl">download</a>')
          .attr('href', base() + 'get_file?path=' + encodeURIComponent(e.path))
          .attr('download', e.name));
        $row.append($('<a class="fb-del" title="Delete">✕</a>').on('click', function(){ removeFile(e); }));
      }
      $list.append($row);
    });
  }

  function go(p, completion){
    var peer = state.peer;
    state.path = cleanPath(p);
    renderCrumbs(state.path);
    say('Loading…');
    listDir(state.path, function(result){
      if (peer !== state.peer) return; // another share was picked while this was in flight
      if (!result || result.status !== 'ok'){
        say((result && result.msg) || 'Could not list this folder.', true);
        $list.empty().append($('<div class="fb-empty"></div>').text('Nothing to show.'));
        return;
      }
      var data = result.data || {};
      var entries = data.entries || [];
      state.path = cleanPath(data.path || state.path);
      renderCrumbs(state.path);
      renderEntries(entries);
      if (completion && completion.message) say(completion.message, completion.isErr);
      else say(entries.length + (entries.length === 1 ? ' item' : ' items'));
    });
  }

  // Delete one file from the share being browsed. A peer's share needs its "files-write" group.
  function removeFile(e){
    if (!window.confirm('Delete "' + e.name + '" from ' + shareName() + '? This cannot be undone.')) return;
    var peer = state.peer, directory = state.path;
    say('Deleting ' + e.name + '…');
    var done = function(msg, isErr){
      if (peer !== state.peer) return;
      go(directory, { message: msg, isErr: isErr });
    };
    $.getJSON(base() + 'delete_path', { path: e.path }).done(function(r){
      var d = (r && r.data) || {};
      if (!r || r.status !== 'ok' || d.status === 'err'){
        var why = d.msg || (r && r.msg) || 'refused';
        if (why === 'UNAUTHORIZED') why = peer ? 'this peer has not put you in its "files-write" group' : 'not allowed';
        done('Could not delete ' + e.name + ': ' + why, true);
      }
      else done('Deleted ' + e.name);
    }).fail(function(){
      done('Could not delete ' + e.name + ': ' + (peer ? 'the peer is offline or has not put you in its "files-write" group' : 'request failed'), true);
    });
  }

  async function uploadFile(file, directory){
    var relative = joinPath(directory, file.name);
    var url = '/files/dav/' + relative.split('/').map(encodeURIComponent).join('/');
    var response = await fetch(url, {
      method: 'PUT',
      body: file,
      credentials: 'same-origin',
      headers: { 'Content-Type': file.type || 'application/octet-stream' }
    });
    if (!response.ok){
      var message = await response.text();
      throw new Error(message || ('Upload failed: HTTP ' + response.status));
    }
  }

  $upload.on('change', async function(){
    var files = Array.prototype.slice.call(this.files || []);
    if (!files.length || state.peer) return;
    var directory = state.path;
    var errors = [];
    var uploaded = 0;
    $upload.prop('disabled', true);
    say('Uploading ' + files.length + '…');
    try {
      for (var file of files){
        try {
          await uploadFile(file, directory);
          uploaded++;
        }
        catch (err) {
          errors.push(file.name + ': ' + err.message);
        }
      }
    }
    finally {
      $upload.prop('disabled', false).val('');
    }
    var message;
    if (errors.length){
      message = uploaded + ' of ' + files.length + ' uploaded. ' + errors.join(' | ');
    }
    else {
      message = 'Uploaded ' + uploaded + (uploaded === 1 ? ' file' : ' files');
    }
    go(directory, { message: message, isErr: errors.length > 0 });
  });

  renderPeers();
  loadPeers();
  setInterval(loadPeers, 15000);
  go('');
};
