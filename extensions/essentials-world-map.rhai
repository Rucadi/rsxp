// fanplay:id = essentials-world-map
// fanplay:name = Essentials World Map
// fanplay:cores = rgss
// fanplay:version = 0.1.0
// fanplay:capabilities = ruby.inject,ruby.unload,ui.webview
// fanplay:operation = world_map
// fanplay:operation-capabilities = world_map game.read
// fanplay:operation = map_preview
// fanplay:operation-capabilities = map_preview game.read
// fanplay:route = GET /v1/game/world-map world_map
// fanplay:route = GET /v1/game/maps/{map_id}/tiles map_preview map_id
// fanplay:panel = world-map,Connected World Map

fn world_map_html() { text_block("html") }

fn on_activate() {
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + "; FANPLAY::WorldMapPlugin.install }, revert: proc { FANPLAY::WorldMapPlugin.uninstall if defined?(FANPLAY::WorldMapPlugin) } }";
    let pending = ruby_inject("essentials-world-map", apply);
    state_set("pending", pending);
    if pending < 0 { state_set("error", last_error()); }
}

fn build_ui() {
    let error = state_get("error");
    if error != () && error != "" { return [ui_label("world-map.error", error)]; }
    if state_get("patch_handle") == () {
        return [ui_loading("world-map.installing", "Installing the world-map plugin…")];
    }
    [
        ui_button("world-map.open", "Open Connected World Map"),
        ui_label("world-map.note", "Explore connected maps, inspect actual tiles, and teleport to a selected tile.")
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "world-map.open" {
        webview_open("essentials-world-map", "Connected World Map", world_map_html());
    }
}

fn on_operation_result(request_id, result) {
    if request_id == state_get("pending") {
        state_set("pending", ());
        if result == () {
            state_set("error", last_error());
            return;
        }
        state_set("patch_handle", result["patch_id"]);
    }
}

fn on_shutdown() {
    let handle = state_get("patch_handle");
    if handle != () { ruby_unload(handle); }
}

/* fanplay:text ruby-begin
module FANPLAY
  module WorldMapPlugin
    def self.install
      @api = FANPLAY::GameAPI
      @original_world_map = @api.method(:world_map) if @api.respond_to?(:world_map)
      @original_map_preview = @api.method(:map_preview) if @api.respond_to?(:map_preview)
      @api.define_singleton_method(:world_map) { FANPLAY::WorldMapPlugin.world_map }
      @api.define_singleton_method(:map_preview) { |map_id| FANPLAY::WorldMapPlugin.map_preview(map_id) }
    end

    def self.uninstall
      return unless @api
      if @original_world_map
        original = @original_world_map
        @api.define_singleton_method(:world_map) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :world_map) rescue nil
      end
      if @original_map_preview
        original = @original_map_preview
        @api.define_singleton_method(:map_preview) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :map_preview) rescue nil
      end
      @api = nil
      @original_world_map = nil
      @original_map_preview = nil
    end

    def self.read(object, method)
      @api.read(object, method)
    end

    def self.read_alias(object, *methods)
      @api.read_alias(object, *methods)
    end

    def self.load_game_data(path)
      @api.load_game_data(path)
    end

    def self.map_data_path(map_id)
      @api.map_data_path(map_id)
    end

    # Describe every RPG Maker map and the Essentials connection offsets. The
    # HTML client owns layout and rendering; Ruby only exposes game data.
    def self.world_map
      infos = load_game_data('Data/MapInfos.rxdata')
      metadata = world_map_metadata(infos.keys)
      transfers = []
      maps = infos.each_with_object([]) do |(id, info), result|
        id = Integer(id)
        path = map_data_path(id)
        next unless File.file?(path)
        map = load_game_data(path)
        transfers.concat(map_transfer_edges(id, map))
        entry = metadata[id] || {}
        position = entry['position']
        result << {
          'id' => id, 'name' => read(info, :name).to_s,
          'width' => Integer(read(map, :width) || 0),
          'height' => Integer(read(map, :height) || 0),
          'tileset_id' => Integer(read(map, :tileset_id) || 0),
          'outdoor' => entry['outdoor'] == true,
          'region' => position && position[0],
          'world_x' => position && position[1],
          'world_y' => position && position[2]
        }
      end
      connections = if Object.const_defined?(:MapFactoryHelper) && MapFactoryHelper.respond_to?(:getMapConnections)
                      MapFactoryHelper.getMapConnections.map do |connection|
                        connection.first(6).map { |value| Integer(value) }
                      end
                    else
                      []
                    end
      player = defined?($game_map) && $game_map ? Integer(read($game_map, :map_id) || 0) : nil
      { 'maps' => maps, 'connections' => connections, 'transfers' => transfers, 'current_map_id' => player }
    end

    def self.map_transfer_edges(map_id, map)
      events = read(map, :events)
      return [] unless events.respond_to?(:values)
      events.values.flat_map do |event|
        pages = read(event, :pages)
        Array(pages).flat_map do |page|
          commands = read(page, :list)
          Array(commands).filter_map do |command|
            next unless read(command, :code).to_i == 201
            parameters = read(command, :parameters)
            next unless parameters.respond_to?(:[]) && parameters[0].to_i == 0
            destination_map = Integer(parameters[1] || 0)
            next unless destination_map.positive?
            [map_id, Integer(read(event, :x) || 0), Integer(read(event, :y) || 0),
             destination_map, Integer(parameters[2] || 0), Integer(parameters[3] || 0)]
          end
        end
      end
    end

    def self.world_map_metadata(map_ids = nil)
      result = {}
      path = File.join(Dir.pwd, 'PBS', 'metadata.txt')
      if File.file?(path)
        current_id = nil
        File.foreach(path, encoding: 'UTF-8') do |raw|
          line = raw.sub(/^\uFEFF/, '').strip
          if line =~ /^\[(\d+)\]$/
            current_id = Integer(Regexp.last_match(1), 10)
            result[current_id] ||= {}
          elsif current_id && line == 'Outdoor=true'
            result[current_id]['outdoor'] = true
          elsif current_id && line.start_with?('MapPosition=')
            values = line.split('=', 2)[1].split(',').map { |value| Integer(value, 10) rescue nil }
            result[current_id]['position'] = values if values.length == 3 && values.none?(&:nil?)
          end
        end
      end
      if map_ids && defined?(GameData::MapMetadata)
        map_ids.each do |map_id|
          record = if GameData::MapMetadata.respond_to?(:try_get)
                     GameData::MapMetadata.try_get(map_id)
                   elsif GameData::MapMetadata.respond_to?(:get)
                     GameData::MapMetadata.get(map_id)
                   end
          next unless record
          entry = result[Integer(map_id)] ||= {}
          outdoor = read_alias(record, :outdoor_map, :outdoor)
          entry['outdoor'] = outdoor == true unless outdoor.nil?
          position = read_alias(record, :town_map_position, :map_position)
          if position.respond_to?(:to_a)
            values = position.to_a.map { |value| Integer(value) rescue nil }
            entry['position'] = values if values.length == 3 && values.none?(&:nil?)
          end
        rescue StandardError
          next
        end
      end
      result
    end

    # Return the real RGSS tile layers and their source bitmap names. The
    # plugin webview composes these with the same 32px tile rules as Tilemap.
    def self.map_preview(map_id)
      map_id = Integer(map_id)
      raise ArgumentError, 'map ID must be positive' unless map_id.positive?
      map = load_game_data(map_data_path(map_id))
      tilesets = defined?($data_tilesets) && $data_tilesets ? $data_tilesets : load_game_data('Data/Tilesets.rxdata')
      tileset = tilesets[Integer(read(map, :tileset_id) || 0)]
      raise RuntimeError, 'map tileset is unavailable' unless tileset
      width = Integer(read(map, :width) || 0)
      height = Integer(read(map, :height) || 0)
      raise RuntimeError, 'map dimensions are invalid' unless width.positive? && height.positive?
      layers = (0...3).map do |z|
        Array.new(height) { |y| Array.new(width) { |x| Integer(map.data[x, y, z] || 0) } }
      end
      {
        'map_id' => map_id, 'width' => width, 'height' => height,
        'tileset' => read(tileset, :tileset_name).to_s,
        'autotiles' => Array(read(tileset, :autotile_names)).map(&:to_s),
        'autotile_parts' => (Tilemap::AUTOTILE_PARTS rescue []),
        'layers' => layers
      }
    end
  end
end
fanplay:text ruby-end */

/* fanplay:text html-begin
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Connected World Map</title>
  <style>
    :root { color-scheme: dark; --bg:#0c0e17; --panel:#15192a; --panel2:#1d2340; --line:#303754; --text:#f2f4ff; --muted:#a8b0ca; --accent:#8e9eff; --cyan:#6ee7ff; --good:#73e0b0; --danger:#ff8d9e; }
    * { box-sizing:border-box; }
    html,body { width:100%; height:100%; margin:0; overflow:hidden; }
    body { background:radial-gradient(circle at 85% 0,#29254e 0,transparent 38%),var(--bg); color:var(--text); font:14px/1.4 Inter,ui-sans-serif,system-ui,sans-serif; }
    button,select,input { font:inherit; }
    button { border:1px solid var(--line); border-radius:9px; background:#222943; color:var(--text); padding:8px 11px; cursor:pointer; }
    button:hover { border-color:var(--accent); background:#2b3458; }
    button.primary { border:0; background:linear-gradient(135deg,#8972ef,#4b9edd); font-weight:700; }
    button.active { border-color:var(--cyan); background:#26365a; }
    input,select { min-width:0; border:1px solid var(--line); border-radius:8px; background:#101422; color:var(--text); padding:8px; }
    .shell { height:100%; min-height:0; display:flex; flex-direction:column; overflow:hidden; }
    .topbar { display:flex; align-items:center; gap:12px; padding:13px 16px; border-bottom:1px solid var(--line); background:rgba(12,14,23,.9); flex:0 0 auto; }
    .brand { display:flex; align-items:center; gap:10px; min-width:245px; }
    .mark { width:36px; height:36px; display:grid; place-items:center; border-radius:11px; background:linear-gradient(145deg,#a887ff,#67dfff); color:#15172a; font-size:20px; font-weight:900; }
    h1 { margin:0; font-size:17px; } .sub { color:var(--muted); font-size:12px; margin-top:2px; }
    .status { flex:1; color:var(--muted); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
    .toolbar { display:flex; gap:8px; align-items:center; flex-wrap:wrap; padding:9px 16px; border-bottom:1px solid var(--line); flex:0 0 auto; }
    .toolbar .readout { color:var(--muted); margin-left:auto; font-size:12px; }
    .body { flex:1; min-height:0; display:grid; grid-template-columns:minmax(0,1fr) minmax(310px,34%); }
    .map-pane { min-width:0; min-height:0; position:relative; background:#0a0d16; }
    #world { width:100%; height:100%; display:block; cursor:grab; }
    #world.dragging { cursor:grabbing; }
    .side { min-width:0; min-height:0; overflow:auto; padding:14px; border-left:1px solid var(--line); background:rgba(21,25,42,.9); }
    .tabs { display:flex; gap:7px; margin-bottom:12px; } .tabs button { flex:1; }
    .card { border:1px solid var(--line); border-radius:13px; background:linear-gradient(160deg,rgba(34,40,70,.9),rgba(20,24,40,.9)); padding:13px; margin-bottom:11px; }
    .card h2,.card h3 { margin:0 0 8px; } .card h2 { font-size:20px; } .card h3 { font-size:14px; }
    .muted { color:var(--muted); } .accent { color:var(--cyan); } .error { color:var(--danger); padding:12px 0; }
    .row { display:flex; justify-content:space-between; gap:10px; align-items:center; margin:7px 0; }
    .row label { color:var(--muted); font-size:12px; } .row input { width:85px; }
    #detail-map { width:100%; display:block; image-rendering:pixelated; border:1px solid var(--line); border-radius:10px; background:#080b12; cursor:crosshair; }
    .map-actions { display:flex; flex-wrap:wrap; gap:7px; } .map-actions button { flex:1; min-width:120px; }
    .interiors { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; }
    .interior { padding:9px; text-align:left; } .interior small { display:block; color:var(--muted); margin-top:3px; }
    .legend { position:absolute; left:12px; bottom:12px; padding:7px 9px; border:1px solid var(--line); border-radius:8px; color:var(--muted); background:rgba(12,14,23,.84); font-size:11px; pointer-events:none; }
    .progress { height:5px; border-radius:9px; background:#242a42; overflow:hidden; margin-top:8px; } .progress i { display:block; height:100%; background:linear-gradient(90deg,var(--accent),var(--cyan)); width:0; transition:width .15s; }
    @media (max-width:800px) { .topbar { flex-wrap:wrap; } .brand { min-width:0; } .status { order:3; flex-basis:100%; } .body { grid-template-columns:1fr; grid-template-rows:minmax(300px,58%) minmax(260px,42%); } .side { border-left:0; border-top:1px solid var(--line); } }
  </style>
</head>
<body>
<main class="shell">
  <header class="topbar"><div class="brand"><div class="mark">✦</div><div><h1>Connected World Map</h1><div class="sub">RPG Maker maps, Essentials connections, and real tiles</div></div></div><div id="status" class="status">Loading world metadata…</div><button id="refresh" class="primary">Refresh</button></header>
  <nav class="toolbar"><button id="zoom-out">−</button><button id="zoom-in">+</button><button id="fit">Fit world</button><span id="zoom" class="readout">1.0×</span><span id="counts" class="readout"></span></nav>
  <section class="body"><div class="map-pane"><canvas id="world"></canvas><div class="legend">Drag to pan · wheel to zoom · click a map or tile</div></div><aside class="side"><div class="tabs"><button id="destination-tab" class="active">Destination</button><button id="indoors-tab">Indoors <span id="indoor-count"></span></button></div><div id="side-content"></div></aside></section>
</main>
<script>
(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const state = { maps:[], outdoor:[], connections:[], transfers:[], previews:new Map(), textures:new Map(), selected:null, x:0, y:0, zoom:1, panX:0, panY:0, layout:null, tab:'destination', error:'', loading:false, queue:[], loaded:0 };
  const world = $('world'), ctx = world.getContext('2d');
  const text = (v, fallback='—') => v == null || v === '' ? fallback : String(v);
  const esc = v => text(v,'').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const api = (target, options) => fetch(target, options).then(async response => { const body = await response.text(); let value={}; try { value = body ? JSON.parse(body) : {}; } catch (_) { throw Error('The game returned invalid JSON.'); } if (!response.ok || value.error) throw Error(value.error || `Request failed (${response.status})`); return value; });
  const mapById = id => state.maps.find(map => Number(map.id) === Number(id));
  const outdoorMaps = () => state.maps.filter(map => map.outdoor === true);

  async function image(category, name) {
    const key = `${category}/${name}`;
    if (state.textures.has(key)) return state.textures.get(key);
    if (!window.FANPLAY || typeof window.FANPLAY.asset !== 'function') return null;
    const src = await window.FANPLAY.asset(category, name);
    if (!src) return null;
    const result = await new Promise(resolve => { const item = new Image(); item.onload = () => resolve(item); item.onerror = () => resolve(null); item.src = src; });
    state.textures.set(key, result); return result;
  }

  async function compose(preview, tileSize) {
    const width = Number(preview.width), height = Number(preview.height);
    if (!width || !height) return null;
    const canvas = document.createElement('canvas'); canvas.width = width * tileSize; canvas.height = height * tileSize;
    const paint = canvas.getContext('2d'); paint.imageSmoothingEnabled = false;
    const tileset = await image('Tilesets', text(preview.tileset, ''));
    const autotiles = await Promise.all((Array.isArray(preview.autotiles) ? preview.autotiles : []).map(name => image('Autotiles', name)));
    const parts = Array.isArray(preview.autotile_parts) ? preview.autotile_parts : [];
    const sourceTile = 32, half = tileSize / 2;
    for (const layer of (Array.isArray(preview.layers) ? preview.layers : [])) {
      for (let y=0; y<height; y++) for (let x=0; x<width; x++) {
        const id = Number(layer?.[y]?.[x] || 0); if (id < 48) continue;
        if (id >= 384) {
          if (!tileset) continue;
          const index = id - 384, sx = (index % 8) * sourceTile, sy = Math.floor(index / 8) * sourceTile;
          paint.drawImage(tileset, sx, sy, sourceTile, sourceTile, x*tileSize, y*tileSize, tileSize, tileSize);
        } else {
          const sheet = autotiles[Math.floor(id / 48) - 1], quadrants = parts[id % 48]; if (!sheet || !Array.isArray(quadrants) || tileSize < 2) continue;
          quadrants.slice(0,4).forEach((part, q) => { const n = Math.max(0, Number(part || 1) - 1), sx = (n % 6) * 16, sy = Math.floor(n / 6) * 16; paint.drawImage(sheet, sx, sy, 16, 16, x*tileSize + (q%2)*half, y*tileSize + Math.floor(q/2)*half, half, half); });
        }
      }
    }
    return canvas;
  }

  async function loadPreview(id, detail=false) {
    if (state.previews.has(Number(id)) && state.previews.get(Number(id)).texture) {
      if (detail && !state.previews.get(Number(id)).detail) state.previews.get(Number(id)).detail = await compose(state.previews.get(Number(id)).data, 32);
      render();
      return;
    }
    try {
      const data = await api(`/v1/game/maps/${encodeURIComponent(id)}/tiles`);
      const entry = { data, texture:null, detail:null }; state.previews.set(Number(id), entry);
      entry.texture = await compose(data, 4);
      if (detail) entry.detail = await compose(data, 32);
      render();
    } catch (error) { state.error = error.message; renderSide(); }
  }

  function buildLayout() {
    const maps = outdoorMaps(), ids = new Set(maps.map(map => Number(map.id))), byId = new Map(maps.map(map => [Number(map.id), map]));
    const edges = [...state.connections, ...state.transfers].map(edge => Array.isArray(edge) ? edge.map(Number) : []).filter(edge => edge.length >= 6 && ids.has(edge[0]) && ids.has(edge[3]));
    const adjacent = new Map(); const add = (a,b,dx,dy) => { if (!adjacent.has(a)) adjacent.set(a,[]); adjacent.get(a).push([b,dx,dy]); };
    edges.forEach(edge => { add(edge[0],edge[3],edge[1]-edge[4],edge[2]-edge[5]); add(edge[3],edge[0],edge[4]-edge[1],edge[5]-edge[2]); });
    const widths = maps.map(map => Math.max(1,Number(map.width)||1)).sort((a,b)=>a-b), grid = Math.max(16,Math.min(64,widths[Math.floor(widths.length/2)]||32));
    const origins = new Map(), visited = new Set();
    for (const root of [...ids].sort((a,b)=>a-b)) {
      if (visited.has(root)) continue; const map = byId.get(root); if (!map || map.world_x == null || map.world_y == null) continue;
      origins.set(root,{x:Number(map.world_x)*grid,y:Number(map.world_y)*grid}); const queue=[root];
      while (queue.length) { const current=queue.shift(); if (visited.has(current)) continue; visited.add(current); const origin=origins.get(current); for (const [next,dx,dy] of (adjacent.get(current)||[])) { if (!origins.has(next)) { origins.set(next,{x:origin.x+dx,y:origin.y+dy}); queue.push(next); } } }
    }
    const placed=[]; for (const id of [...ids].sort((a,b)=>a-b)) { if (!origins.has(id)) continue; const map=byId.get(id), size={w:Math.max(1,Number(map.width)||1),h:Math.max(1,Number(map.height)||1)}; let origin={...origins.get(id)}; const overlaps=point => placed.some(item => point.x < item.x+item.w && point.x+size.w > item.x && point.y < item.y+item.h && point.y+size.h > item.y); if (overlaps(origin)) { let n=1; while (overlaps({x:origin.x+n*2,y:origin.y})) n++; origin={x:origin.x+n*2,y:origin.y}; origins.set(id,origin); } placed.push({x:origin.x,y:origin.y,w:size.w,h:size.h}); }
    return { origins, edges };
  }

  function bounds() { const layout=state.layout; const items=outdoorMaps().filter(map=>layout?.origins.has(Number(map.id))).map(map => { const p=layout.origins.get(Number(map.id)); return {x:p.x,y:p.y,w:Math.max(1,Number(map.width)||1),h:Math.max(1,Number(map.height)||1)}; }); if (!items.length) return {x:0,y:0,w:1,h:1}; const minX=Math.min(...items.map(i=>i.x)), minY=Math.min(...items.map(i=>i.y)), maxX=Math.max(...items.map(i=>i.x+i.w)), maxY=Math.max(...items.map(i=>i.y+i.h)); return {x:minX,y:minY,w:Math.max(1,maxX-minX),h:Math.max(1,maxY-minY)}; }
  function resizeCanvas() { const rect=world.getBoundingClientRect(), dpr=window.devicePixelRatio||1; const w=Math.max(1,Math.floor(rect.width*dpr)), h=Math.max(1,Math.floor(rect.height*dpr)); if (world.width!==w||world.height!==h) { world.width=w; world.height=h; } ctx.setTransform(dpr,0,0,dpr,0,0); return {w:rect.width,h:rect.height}; }
  function screenPoint(point, view, box, scale) { return {x:view.w/2+(point.x-box.x-box.w/2)*scale+state.panX,y:view.h/2+(point.y-box.y-box.h/2)*scale+state.panY}; }
  function renderWorld() {
    const view=resizeCanvas(), box=bounds(); state.fit=Math.min(view.w/Math.max(box.w,1),view.h/Math.max(box.h,1))*.92; const scale=state.fit*state.zoom; ctx.clearRect(0,0,view.w,view.h); ctx.fillStyle='#090c15'; ctx.fillRect(0,0,view.w,view.h); $('zoom').textContent=`${state.zoom.toFixed(1)}×`;
    const point=tile=>screenPoint(tile,view,box,scale), layout=state.layout;
    ctx.lineWidth=2; ctx.strokeStyle='rgba(110,231,255,.65)'; for (const edge of layout?.edges||[]) { const a=layout.origins.get(edge[0]), b=layout.origins.get(edge[3]); if (!a||!b) continue; const p=point({x:a.x+edge[1],y:a.y+edge[2]}), q=point({x:b.x+edge[4],y:b.y+edge[5]}); ctx.beginPath(); ctx.moveTo(p.x,p.y); ctx.lineTo(q.x,q.y); ctx.stroke(); }
    for (const map of outdoorMaps()) { const origin=layout?.origins.get(Number(map.id)); if (!origin) continue; const p=point(origin), w=Math.max(1,Number(map.width)||1)*scale, h=Math.max(1,Number(map.height)||1)*scale, entry=state.previews.get(Number(map.id)); if (entry?.texture) ctx.drawImage(entry.texture,p.x,p.y,w,h); else { ctx.fillStyle=Number(map.id)===Number(state.selected)?'#315d73': '#324a3b'; ctx.fillRect(p.x,p.y,w,h); } ctx.strokeStyle=Number(map.id)===Number(state.selected)?'#ffe66d':'rgba(220,230,255,.55)'; ctx.lineWidth=Number(map.id)===Number(state.selected)?2.5:1; ctx.strokeRect(p.x,p.y,w,h); if (scale>.13) { ctx.fillStyle='white'; ctx.font=`${Math.max(10,Math.min(14,11*scale))}px sans-serif`; ctx.fillText(`${map.id} ${text(map.name,'Map')}`,p.x+4,p.y+15); } if (Number(map.id)===Number(state.selected)) { const cell=point({x:origin.x+state.x+.5,y:origin.y+state.y+.5}); ctx.strokeStyle='#ffe66d'; ctx.lineWidth=2; ctx.strokeRect(cell.x-scale/2,cell.y-scale/2,scale,scale); } }
  }

  function connectedIndoors() { if (state.selected == null) return []; const ids=new Set(); for (const edge of state.transfers) { if (!Array.isArray(edge)||edge.length<6) continue; const other=Number(edge[0])===Number(state.selected)?Number(edge[3]):Number(edge[3])===Number(state.selected)?Number(edge[0]):null; const map=other == null ? null : mapById(other); if (map && map.outdoor !== true) ids.add(other); } return [...ids].sort((a,b)=>a-b).map(mapById); }
  async function drawDetail(preview) {
    const canvas=$('detail-map'); if (!canvas||!preview) return;
    const width=Number(preview.width), height=Number(preview.height), maxW=Math.max(180,canvas.parentElement.clientWidth-4), maxH=380;
    const scale=Math.min(maxW/(width*32),maxH/(height*32),1), texture=state.previews.get(Number(preview.map_id))?.detail || await compose(preview,32);
    if (!texture) return;
    canvas.width=Math.max(1,Math.floor(width*32*scale)); canvas.height=Math.max(1,Math.floor(height*32*scale)); canvas.style.aspectRatio=`${canvas.width}/${canvas.height}`;
    const c=canvas.getContext('2d'); c.imageSmoothingEnabled=false; c.clearRect(0,0,canvas.width,canvas.height); c.drawImage(texture,0,0,canvas.width,canvas.height);
    const x=Math.min(width-1,Math.max(0,state.x)), y=Math.min(height-1,Math.max(0,state.y)); c.strokeStyle='#ffe66d'; c.lineWidth=Math.max(2,2*scale); c.strokeRect(x*32*scale,y*32*scale,32*scale,32*scale);
  }

  function renderSide() {
    const map=mapById(state.selected), indoors=connectedIndoors(), selectedPreview=state.previews.get(Number(state.selected))?.data; $('indoor-count').textContent=indoors.length?`· ${indoors.length}`:'';
    if (state.error) { $('side-content').innerHTML=`<div class="error">${esc(state.error)}</div>`; return; }
    if (!map) { $('side-content').innerHTML='<div class="card muted">Select an outdoor map from the canvas.</div>'; return; }
    if (state.tab==='indoors') { $('side-content').innerHTML=`<div class="card"><h2>Linked interiors</h2><div class="muted">Select an interior to inspect its tiles and teleport destination.</div></div>${indoors.length?`<div class="interiors">${indoors.map(item=>`<button class="interior" data-map="${item.id}">${esc(item.name||'Interior')}<small>Map ${item.id} · ${item.width}×${item.height}</small></button>`).join('')}</div>`:'<div class="card muted">No transfer-linked interiors were found for this map.</div>'}`; document.querySelectorAll('[data-map]').forEach(button=>button.onclick=()=>selectMap(Number(button.dataset.map),0,0)); return; }
    $('side-content').innerHTML=`<div class="card"><h2>${esc(map.name||'Map')}</h2><div class="muted">Map ${map.id} · ${map.width}×${map.height} tiles · tileset ${map.tileset_id}</div><div class="row"><label>Tile X</label><input id="tile-x" type="number" min="0" max="${Math.max(0,Number(map.width)-1)}" value="${state.x}"></div><div class="row"><label>Tile Y</label><input id="tile-y" type="number" min="0" max="${Math.max(0,Number(map.height)-1)}" value="${state.y}"></div><div class="map-actions"><button id="load-tiles">Load actual tiles</button><button id="teleport" class="primary">Teleport to tile</button></div></div>${selectedPreview?'<div class="card"><h3>Selected map</h3><canvas id="detail-map"></canvas><div class="muted" style="margin-top:7px">Click a tile to choose the destination.</div></div>':'<div class="card muted">Loading map tiles…</div>'}${indoors.length?`<div class="card"><h3>Linked interiors · ${indoors.length}</h3><button id="show-indoors">Browse interiors</button></div>`:''}`;
    $('tile-x').onchange=e=>{state.x=Number(e.target.value)||0;render();}; $('tile-y').onchange=e=>{state.y=Number(e.target.value)||0;render();}; $('load-tiles').onclick=()=>loadPreview(state.selected,true); $('teleport').onclick=()=>{ const button=$('teleport'); button.disabled=true; api(`/v1/game/teleport?map_id=${state.selected}&x=${state.x}&y=${state.y}`,{method:'POST'}).then(()=>{ $('status').textContent=`Teleport queued to ${map.name||'map'} (${state.x}, ${state.y}).`; }).catch(error=>{state.error=error.message;renderSide();}).finally(()=>button.disabled=false); }; $('show-indoors')?.addEventListener('click',()=>{state.tab='indoors';renderSide();});
    if (selectedPreview) {
      drawDetail(selectedPreview);
      $('detail-map').onclick = event => {
        const canvas=$('detail-map'), rect=canvas.getBoundingClientRect();
        state.x=Math.max(0,Math.min(Number(map.width)-1,Math.floor((event.clientX-rect.left)/rect.width*Number(map.width))));
        state.y=Math.max(0,Math.min(Number(map.height)-1,Math.floor((event.clientY-rect.top)/rect.height*Number(map.height))));
        renderSide(); renderWorld();
      };
    }
  }

  function selectMap(id,x=0,y=0) { const map=mapById(id); if (!map) return; state.selected=Number(id); state.x=Math.max(0,Math.min(Number(map.width||1)-1,x)); state.y=Math.max(0,Math.min(Number(map.height||1)-1,y)); state.tab='destination'; render(); if (!state.previews.has(Number(id))) loadPreview(Number(id),true); else if (!state.previews.get(Number(id)).detail) loadPreview(Number(id),true); }
  function render() { renderWorld(); renderSide(); }
  async function loadWorld() { state.loading=true; state.error=''; state.previews.clear(); state.textures.clear(); $('status').textContent='Loading world metadata…'; try { const value=await api('/v1/game/world-map'); state.maps=Array.isArray(value.maps)?value.maps:[]; state.connections=Array.isArray(value.connections)?value.connections:[]; state.transfers=Array.isArray(value.transfers)?value.transfers:[]; state.outdoor=outdoorMaps(); state.layout=buildLayout(); const current=state.outdoor.find(map=>Number(map.id)===Number(value.current_map_id))||state.outdoor[0]; if(current) state.selected=Number(current.id); state.queue=state.outdoor.map(map=>Number(map.id)); state.loaded=0; $('counts').textContent=`${state.outdoor.length} overworld maps · ${state.maps.length} total`; render(); while(state.queue.length) { const id=state.queue.shift(); await loadPreview(id,false); state.loaded++; $('status').textContent=`Stitching overworld tiles · ${state.loaded} / ${state.outdoor.length}`; renderWorld(); } $('status').textContent='Click a map or tile to choose a destination.'; if(state.selected!=null) await loadPreview(state.selected,true); render(); } catch(error) { state.error=error.message; $('status').textContent='World map unavailable'; renderSide(); } finally { state.loading=false; } }
  function resetView() { state.zoom=1; state.panX=0; state.panY=0; renderWorld(); }
  function zoomAt(clientX, clientY, factor) {
    const rect=world.getBoundingClientRect(), view={w:rect.width,h:rect.height}, box=bounds(), oldScale=state.fit*state.zoom;
    if (!oldScale) return;
    const localX=clientX-rect.left, localY=clientY-rect.top;
    const worldX=box.x+box.w/2+(localX-view.w/2-state.panX)/oldScale, worldY=box.y+box.h/2+(localY-view.h/2-state.panY)/oldScale;
    state.zoom=Math.max(.01,Math.min(256,state.zoom*factor));
    const newScale=state.fit*state.zoom;
    state.panX=localX-view.w/2-(worldX-box.x-box.w/2)*newScale;
    state.panY=localY-view.h/2-(worldY-box.y-box.h/2)*newScale;
    renderWorld();
  }
  $('refresh').onclick=()=>loadWorld(); $('zoom-in').onclick=()=>zoomAt(world.getBoundingClientRect().left+world.clientWidth/2,world.getBoundingClientRect().top+world.clientHeight/2,1.25); $('zoom-out').onclick=()=>zoomAt(world.getBoundingClientRect().left+world.clientWidth/2,world.getBoundingClientRect().top+world.clientHeight/2,.8); $('fit').onclick=resetView; $('destination-tab').onclick=()=>{state.tab='destination';$('destination-tab').classList.add('active');$('indoors-tab').classList.remove('active');renderSide();}; $('indoors-tab').onclick=()=>{state.tab='indoors';$('indoors-tab').classList.add('active');$('destination-tab').classList.remove('active');renderSide();};
  let dragging=false,moved=false,last={x:0,y:0}; world.addEventListener('pointerdown',event=>{dragging=true;moved=false;last={x:event.clientX,y:event.clientY};world.classList.add('dragging');world.setPointerCapture?.(event.pointerId);}); world.addEventListener('pointermove',event=>{if(!dragging)return;const dx=event.clientX-last.x,dy=event.clientY-last.y;if(Math.abs(dx)+Math.abs(dy)>1)moved=true;state.panX+=dx;state.panY+=dy;last={x:event.clientX,y:event.clientY};renderWorld();}); world.addEventListener('pointerup',()=>{dragging=false;world.classList.remove('dragging');}); world.addEventListener('pointercancel',()=>{dragging=false;world.classList.remove('dragging');}); world.addEventListener('wheel',event=>{event.preventDefault();zoomAt(event.clientX,event.clientY,Math.exp(-event.deltaY*.001));},{passive:false}); world.addEventListener('click',event=>{if(moved)return; const rect=world.getBoundingClientRect(),view={w:rect.width,h:rect.height},box=bounds(),scale=state.fit*state.zoom,worldX=box.x+box.w/2+(event.clientX-rect.left-view.w/2-state.panX)/scale,worldY=box.y+box.h/2+(event.clientY-rect.top-view.h/2-state.panY)/scale; for(const map of outdoorMaps()) { const origin=state.layout?.origins.get(Number(map.id)); if(!origin)continue; const x=Math.floor(worldX-origin.x),y=Math.floor(worldY-origin.y); if(x>=0&&y>=0&&x<Number(map.width)&&y<Number(map.height)){selectMap(Number(map.id),x,y);break;} } }); window.addEventListener('resize',renderWorld); loadWorld();
})();
</script>
</body>
</html>
fanplay:text html-end */
