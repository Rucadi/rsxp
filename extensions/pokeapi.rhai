// fanplay:id = pokeapi
// fanplay:name = PokeAPI Explorer
// fanplay:version = 0.1.0
// fanplay:capabilities = ui.webview
// The generic game/input operations are declared here so the explorer is also
// a discoverable extension surface. Their game-thread implementation remains
// in the injected Essentials adapter; this plugin supplies the API client UI.
// fanplay:operation = game_status
// fanplay:route = GET /v1/game/status game_status
// fanplay:operation = get_surroundings
// fanplay:route = GET /v1/game/surroundings get_surroundings radius
// fanplay:operation = teleport
// fanplay:route = POST /v1/game/teleport teleport map_id,x,y
// fanplay:operation = press_key
// fanplay:route = POST /v1/input/press press_key key,frames
// fanplay:operation = release_all_keys
// fanplay:route = DELETE /v1/input/held release_all_keys
// fanplay:panel = explorer,PokeAPI Explorer

fn build_ui() {
    [
        ui_button("pokeapi.open", "Open PokeAPI Explorer (HTML)"),
        ui_label("pokeapi.note", "Inspect the live game API, send requests, and copy JSON responses."),
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "pokeapi.open" {
        webview_open("pokeapi", "PokeAPI Explorer", text_block("html"));
    }
}

fn on_activate() { }
fn on_operation_result(_request_id, _result) { }
fn on_shutdown() { }

/* fanplay:text html-begin
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>PokeAPI Explorer</title>
  <style>
    :root { color-scheme: dark; --bg:#0b0d16; --panel:#15192a; --panel2:#1d2237; --line:#303754; --text:#f3f5ff; --muted:#a5adcb; --accent:#a979ff; --cyan:#68d8ff; --danger:#ff91a5; }
    * { box-sizing:border-box; }
    html,body { width:100%; height:100%; margin:0; overflow:hidden; }
    body { background:radial-gradient(circle at 90% 0,#2c2454 0,transparent 35%),var(--bg); color:var(--text); font:14px/1.45 Inter,ui-sans-serif,system-ui,sans-serif; }
    button,input,select,textarea { font:inherit; }
    button { border:1px solid var(--line); border-radius:9px; padding:9px 12px; color:var(--text); background:#222943; cursor:pointer; }
    button:hover { border-color:var(--accent); background:#2b3150; }
    button.primary { border:0; background:linear-gradient(135deg,#a264ef,#4d9ddd); font-weight:700; }
    input,select,textarea { width:100%; border:1px solid var(--line); border-radius:8px; padding:9px 10px; background:#0f1321; color:var(--text); outline:none; }
    input:focus,select:focus,textarea:focus { border-color:var(--accent); box-shadow:0 0 0 3px #a979ff20; }
    .shell { height:100%; min-height:0; display:flex; flex-direction:column; overflow:hidden; }
    header { display:flex; align-items:center; gap:14px; padding:17px 22px 14px; border-bottom:1px solid var(--line); background:#0d0f19dd; }
    .mark { width:39px; height:39px; border-radius:13px; display:grid; place-items:center; background:linear-gradient(145deg,var(--accent),var(--cyan)); color:#121426; font-size:20px; font-weight:900; }
    h1 { margin:0; font-size:18px; } h2 { margin:0 0 10px; font-size:15px; } h3 { margin:0; font-size:13px; }
    .sub,.muted { color:var(--muted); } .sub { margin:2px 0 0; font-size:12px; }
    .toolbar { display:grid; grid-template-columns:130px 1fr auto; gap:9px; padding:13px 22px; border-bottom:1px solid var(--line); }
    .toolbar button { white-space:nowrap; }
    .layout { flex:1; min-height:0; display:grid; grid-template-columns:minmax(290px,38%) 1fr; }
    .left,.right { min-width:0; min-height:0; overflow:auto; padding:18px; }
    .left { border-right:1px solid var(--line); } .right { background:#10132180; }
    .card { border:1px solid var(--line); border-radius:13px; background:#15192ae8; padding:14px; margin-bottom:12px; }
    .card > * + * { margin-top:10px; }
    .section-title { display:flex; justify-content:space-between; align-items:center; gap:8px; }
    .method { display:inline-block; border-radius:6px; padding:3px 7px; background:#6bdfad20; color:#79e6b7; font:700 11px ui-monospace,monospace; }
    .method.post { background:#a979ff22; color:#c59fff; } .method.delete { background:#ff91a522; color:#ff9bae; }
    .op { width:100%; text-align:left; display:block; margin-top:8px; background:#1a1f34; }
    .op small { display:block; margin-top:3px; color:var(--muted); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
    .grid { display:grid; grid-template-columns:1fr 1fr; gap:10px; } label { display:grid; gap:5px; color:var(--muted); font-size:12px; }
    .wide { grid-column:1/-1; } textarea { min-height:150px; resize:vertical; font:12px/1.5 ui-monospace,SFMono-Regular,monospace; }
    pre { min-height:260px; max-height:calc(100vh - 250px); overflow:auto; margin:0; white-space:pre-wrap; overflow-wrap:anywhere; color:#e7eaff; font:12px/1.55 ui-monospace,SFMono-Regular,monospace; }
    .status { min-height:20px; color:var(--muted); } .error { color:var(--danger); } .ok { color:#79e6b7; }
    @media(max-width:800px) { .layout { grid-template-columns:1fr; overflow:auto; } .left { max-height:46vh; border-right:0; border-bottom:1px solid var(--line); } .toolbar { grid-template-columns:1fr 1fr; } .toolbar input { grid-column:1/-1; grid-row:1; } }
  </style>
</head>
<body>
  <div class="shell">
    <header><div class="mark">✦</div><div><h1>PokeAPI Explorer</h1><p class="sub">Live API console for the running game and loaded plugins</p></div></header>
    <div class="toolbar"><select id="method"><option>GET</option><option>POST</option><option>DELETE</option></select><input id="path" value="/v1/game/status" placeholder="/v1/game/status?radius=5"><button id="send" class="primary">Send request</button></div>
    <div class="layout">
      <aside class="left">
        <div class="card"><div class="section-title"><h2>Common operations</h2><button id="clear">Clear</button></div><div id="operations"></div></div>
        <div class="card"><h2>Request body</h2><label>JSON body <textarea id="body" placeholder="Optional JSON body (reserved for future plugin operations)"></textarea></label><p class="muted">Most game operations use query parameters in the URL. The live route matcher is shared with the external API and browser host.</p></div>
      </aside>
      <main class="right"><div class="card"><div class="section-title"><h2>Response</h2><button id="copy">Copy JSON</button></div><div id="status" class="status">Choose an operation or enter a route.</div><pre id="response">Responses will appear here.</pre></div><div class="card"><h2>Quick reference</h2><p class="muted">Game status, surroundings, teleport, input, world map, Pokémon Essentials data, movedex, Pokédex, and all loaded plugin routes are available through the same <code>/v1/</code> transport.</p></div></main>
    </div>
  </div>
  <script>
    const $ = id => document.getElementById(id);
    const operations = [
      ['GET','/v1/game/status','Current game status'],
      ['GET','/v1/game/surroundings?radius=5','Map surroundings'],
      ['GET','/v1/game/world-map','Connected world map'],
      ['GET','/v1/game/maps/1/tiles','Map tile preview'],
      ['POST','/v1/game/teleport?map_id=1&x=0&y=0','Teleport to a tile'],
      ['POST','/v1/input/press?key=action&frames=1','Press an input key'],
      ['DELETE','/v1/input/held','Release held input'],
      ['GET','/v1/pokemon-essentials/status','Essentials status'],
      ['GET','/v1/pokemon-essentials/party','Party summary'],
      ['GET','/v1/pokemon-essentials/party/0','Party slot details'],
      ['GET','/v1/pokemon-essentials/encounters/current-map','Current-map encounters'],
      ['GET','/v1/pokemon-essentials/npcs/current-map','Current-map NPCs'],
      ['GET','/v1/pokemon-essentials/battle','Battle state and options'],
      ['GET','/v1/pokemon-essentials/debug','Read Essentials debug mode'],
      ['POST','/v1/pokemon-essentials/debug?enabled=true','Set Essentials debug mode'],
      ['GET','/v1/pokemon-essentials/itemdex','Itemdex'],
      ['GET','/v1/pokemon-essentials/movedex','Movedex'],
      ['GET','/v1/pokemon-essentials/pokedex','Pokédex index'],
      ['GET','/v1/pokemon-essentials/species/PIKACHU','Pokédex species entry'],
      ['POST','/v1/pokemon-essentials/inventory/add?item=POTION&quantity=1','Add an inventory item'],
      ['POST','/v1/pokemon-essentials/wild-encounter?species=PIKACHU&level=5','Queue a wild encounter'],
      ['GET','/v1/essentials/runtime-patches?payload=state','Essentials runtime patch state'],
      ['GET','/v1/extensions/routes','All loaded plugin routes'],
    ];
    const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
    $('operations').innerHTML = operations.map(([method,path,label],i) => `<button class="op" data-op="${i}"><span class="method ${method.toLowerCase()}">${method}</span> ${esc(label)}<small>${esc(path)}</small></button>`).join('');
    document.querySelectorAll('[data-op]').forEach(button => button.onclick = () => { const [method,path] = operations[Number(button.dataset.op)]; $('method').value=method; $('path').value=path; send(); });
    async function send() {
      const method=$('method').value, path=$('path').value.trim(); if(!path) return;
      $('send').disabled=true; $('status').className='status'; $('status').textContent='Waiting for the game thread…'; $('response').textContent='';
      try {
        const options={method}; const body=$('body').value.trim(); if(body && method!=='GET' && method!=='DELETE'){ options.headers={'Content-Type':'application/json'}; options.body=body; }
        const response=await fetch(path,options), text=await response.text(); let value; try { value=text?JSON.parse(text):{}; } catch(_) { value=text; }
        $('response').textContent=typeof value==='string'?value:JSON.stringify(value,null,2); if(!response.ok || value?.error) throw Error(value?.error || `Request failed (${response.status})`); $('status').className='status ok'; $('status').textContent=`${method} ${path} · ${response.status} OK`;
      } catch(error) { $('status').className='status error'; $('status').textContent=error.message; }
      finally { $('send').disabled=false; }
    }
    $('send').onclick=send; $('path').onkeydown=event=>{if(event.key==='Enter')send();}; $('clear').onclick=()=>{ $('body').value=''; $('response').textContent='Responses will appear here.'; $('status').className='status'; $('status').textContent='Choose an operation or enter a route.'; };
    $('copy').onclick=()=>navigator.clipboard?.writeText($('response').textContent);
  </script>
</body>
</html>
fanplay:text html-end */
