var me = this;
var ME = $('#' + me.UUID)[0];

me.ready = function(){
  // peer '' = this instance's own share; otherwise the uuid of the peer being browsed
  var state = { path: '', peer: '' };

  var $crumbs = $(ME).find('.fb-crumbs');
  var $list   = $(ME).find('.fb-list');
  var $status = $(ME).find('.fb-status');
  var $upload = $(ME).find('.fb-file');
  var $peer   = $(ME).find('.fb-peer');

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

  function loadPeers(){
    $.getJSON('/peer/peers').done(function(r){
      var peers = Array.isArray(r) ? r : (r && Array.isArray(r.data) ? r.data : []);
      peers.sort(function(a, b){ return String(a.name || a.id).localeCompare(String(b.name || b.id)); });
      $peer.find('option').slice(1).remove();
      peers.forEach(function(p){
        $peer.append($('<option></option>').val(p.id)
          .text((p.name || p.id) + (p.connected ? '' : ' (offline)'))
          .prop('disabled', !p.connected && p.id !== state.peer));
      });
      $peer.val(state.peer);
    });
  }

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
  // breadcrumb shows "root / agent" and not "root / . / agent".
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
    add('root', '', parts.length === 0);
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
        $row.on('click', function(){ go(e.path); });
      } else {
        var $dl = $('<a class="fb-dl">download</a>')
          .attr('href', base() + 'get_file?path=' + encodeURIComponent(e.path))
          .attr('download', e.name);
        $row.append($dl);
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
      if (peer !== state.peer) return; // the picker moved on while this was in flight
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

  $peer.on('change', function(){
    state.peer = $peer.val() || '';
    $(ME).find('.fb-upload').toggle(!state.peer); // uploads go to your own share only
    go('');
  });

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

  loadPeers();
  setInterval(loadPeers, 15000);
  go('');
};
