// fanplay:id = essentials-movedex
// fanplay:name = Essentials Movedex
// fanplay:version = 0.1.0
// fanplay:capabilities = ruby.inject,ruby.unload,ui.webview,pokemon_essentials.read,pokemon_essentials.write
// fanplay:operation = pokemon_essentials_movedex
// fanplay:operation-capabilities = pokemon_essentials_movedex pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/movedex pokemon_essentials_movedex refresh
// fanplay:panel = movedex,Essentials Movedex

// The host only supplies a webview and a generic game-thread bridge. Move
// catalog shape, filtering, presentation, and the Ruby adapter live here.
fn movedex_html() {
    text_block("html")
}

fn on_activate() {
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + "; FANPLAY::MovedexPlugin.install }, revert: proc { FANPLAY::MovedexPlugin.uninstall if defined?(FANPLAY::MovedexPlugin) } }";
    let pending = ruby_inject("essentials-movedex", apply);
    state_set("pending", pending);
    if pending < 0 {
        state_set("error", last_error());
    }
}

fn build_ui() {
    let error = state_get("error");
    if error != () && error != "" {
        return [ui_label("movedex.error", error)];
    }
    if state_get("patch_handle") == () {
        return [ui_loading("movedex.installing", "Installing the Movedex plugin…")];
    }
    [
        ui_button("movedex.open", "Open Movedex (HTML)"),
        ui_label("movedex.note", "Search, filters, move details, and TM actions run in the plugin webview.")
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "movedex.open" {
        webview_open("essentials-movedex", "Essentials Movedex", movedex_html());
    }
}

fn on_operation_result(request_id, result) {
    if request_id == state_get("pending") {
        state_set("pending", ());
        state_set("patch_handle", result["patch_id"]);
    }
}

fn on_shutdown() {
    let handle = state_get("patch_handle");
    if handle != () {
        ruby_unload(handle);
    }
}

/* fanplay:text html-begin
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Essentials Movedex</title>
  <style>
    :root {
      color-scheme: dark;
      --bg: #0c0d15;
      --panel: #141624;
      --panel-2: #1b1e31;
      --line: #2c3048;
      --muted: #9ca4c2;
      --text: #f3f4ff;
      --accent: #b879ff;
      --accent-2: #71d8ff;
      --good: #70e0b0;
      --danger: #ff8f9f;
    }
    * { box-sizing: border-box; }
    html, body { width: 100%; height: 100%; overflow: hidden; }
    body { margin: 0; background: radial-gradient(circle at 85% 0%, #252047 0, transparent 35%), var(--bg); color: var(--text); font: 14px/1.45 Inter, ui-sans-serif, system-ui, sans-serif; }
    button, input, select { font: inherit; }
    button { border: 1px solid var(--line); border-radius: 9px; background: #20243a; color: var(--text); padding: 8px 12px; cursor: pointer; transition: .15s ease; }
    button:hover { border-color: var(--accent); background: #2b2d4c; transform: translateY(-1px); }
    button.primary { border-color: transparent; background: linear-gradient(135deg, #9d63ef, #528edb); font-weight: 700; }
    button.ghost { background: transparent; }
    .shell { height: 100%; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
    .topbar { display: flex; align-items: center; gap: 14px; padding: 18px 22px 14px; border-bottom: 1px solid var(--line); backdrop-filter: blur(14px); background: rgba(12,13,21,.82); position: sticky; top: 0; z-index: 3; }
    .brand { display: flex; align-items: center; gap: 11px; min-width: 220px; }
    .mark { width: 38px; height: 38px; border-radius: 12px; display: grid; place-items: center; background: linear-gradient(145deg, var(--accent), var(--accent-2)); color: #121324; font-size: 21px; font-weight: 900; box-shadow: 0 0 30px rgba(184,121,255,.3); }
    .brand h1 { margin: 0; font-size: 17px; letter-spacing: .02em; }
    .brand p { margin: 2px 0 0; color: var(--muted); font-size: 12px; }
    .search { flex: 1; display: flex; align-items: center; gap: 8px; max-width: 680px; }
    input, select { min-width: 0; border: 1px solid var(--line); border-radius: 9px; background: #101220; color: var(--text); padding: 9px 11px; outline: none; }
    input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(184,121,255,.12); }
    #search { flex: 1; }
    .count { white-space: nowrap; color: var(--muted); font-size: 12px; }
    .toolbar { display: flex; gap: 9px; align-items: center; padding: 12px 22px; border-bottom: 1px solid rgba(44,48,72,.75); flex-wrap: wrap; }
    .toolbar label { color: var(--muted); font-size: 12px; display: flex; align-items: center; gap: 6px; }
    .toolbar input[type=number] { width: 76px; padding: 7px 8px; }
    .toolbar select { padding: 7px 9px; }
    .check { user-select: none; }
    .check input { accent-color: var(--accent); }
    .layout { display: grid; grid-template-columns: minmax(270px, 34%) 1fr; flex: 1; min-height: 0; }
    .list-pane { min-height: 0; overflow: auto; border-right: 1px solid var(--line); padding: 13px; }
    .list-head { color: var(--muted); font-size: 12px; padding: 2px 4px 10px; }
    .row { display: grid; grid-template-columns: 44px 1fr auto; align-items: center; gap: 10px; padding: 10px; margin-bottom: 7px; border: 1px solid transparent; border-radius: 12px; cursor: pointer; background: rgba(27,30,49,.55); }
    .row:hover { border-color: #4f456b; background: #20233b; }
    .row.selected { border-color: var(--accent); background: linear-gradient(100deg, rgba(184,121,255,.18), rgba(113,216,255,.06)); }
    .orb { width: 40px; height: 40px; display: grid; place-items: center; border-radius: 13px; background: #282946; color: var(--accent-2); font-weight: 800; font-size: 11px; }
    .row strong { display: block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .row small { display: block; color: var(--muted); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .row .mini { color: var(--muted); font-size: 11px; text-align: right; }
    .tag { display: inline-flex; align-items: center; min-height: 23px; padding: 3px 9px; border-radius: 99px; background: color-mix(in srgb, var(--tag, #858aa8) 20%, transparent); color: color-mix(in srgb, var(--tag, #b8bedb) 75%, white); border: 1px solid color-mix(in srgb, var(--tag, #858aa8) 45%, transparent); font-size: 11px; font-weight: 700; letter-spacing: .04em; }
    .detail-pane { min-width: 0; min-height: 0; overflow: hidden; padding: 24px clamp(18px, 4vw, 54px); }
    .empty { display: grid; place-items: center; min-height: 55vh; color: var(--muted); text-align: center; }
    .hero { display: flex; justify-content: space-between; gap: 20px; align-items: flex-start; padding-bottom: 22px; border-bottom: 1px solid var(--line); }
    .eyebrow { color: var(--accent-2); font-size: 11px; text-transform: uppercase; letter-spacing: .14em; font-weight: 800; }
    .hero h2 { margin: 5px 0 4px; font-size: clamp(27px, 4vw, 42px); line-height: 1.05; letter-spacing: -.03em; }
    .hero .id { color: var(--muted); font-family: ui-monospace, SFMono-Regular, monospace; }
    .hero-tags { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 7px; padding-top: 8px; }
    .cards { display: grid; grid-template-columns: repeat(6, minmax(80px, 1fr)); gap: 9px; margin: 20px 0; }
    .stat { padding: 12px; border: 1px solid var(--line); border-radius: 12px; background: linear-gradient(160deg, rgba(30,34,56,.9), rgba(20,22,36,.9)); }
    .stat label { display: block; color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
    .stat b { display: block; margin-top: 3px; font-size: 18px; }
    .section { margin-top: 18px; padding: 17px 18px; border: 1px solid var(--line); border-radius: 15px; background: rgba(20,22,36,.82); }
    .section h3 { margin: 0 0 10px; font-size: 14px; }
    .description { color: #d4d8eb; white-space: pre-wrap; }
    .machine-list { display: flex; flex-wrap: wrap; gap: 8px; }
    .machine { display: flex; align-items: center; gap: 8px; padding: 7px 9px; border: 1px solid var(--line); border-radius: 9px; background: #171a2b; }
    .machine code { color: var(--accent-2); }
    .machine button { padding: 4px 7px; font-size: 11px; }
    .actions { display: flex; flex-wrap: wrap; gap: 8px; }
    .notice { margin-top: 10px; padding: 9px 11px; border-radius: 9px; background: rgba(112,224,176,.1); color: var(--good); }
    .notice.error, .error { color: var(--danger); }
    .error { padding: 22px; text-align: center; }
    .muted { color: var(--muted); }
    @media (max-width: 820px) {
      .topbar { flex-wrap: wrap; padding: 14px; }
      .brand { min-width: 0; }
      .search { order: 3; flex-basis: 100%; max-width: none; }
      .toolbar { padding: 10px 14px; }
      .layout { grid-template-columns: 1fr; }
      .list-pane { max-height: 38vh; border-right: 0; border-bottom: 1px solid var(--line); }
      .detail-pane { padding: 20px 14px; }
      .cards { grid-template-columns: repeat(3, 1fr); }
    }
    @media (max-width: 440px) { .hero { display: block; } .hero-tags { justify-content: flex-start; } .cards { grid-template-columns: repeat(2, 1fr); } .topbar button { padding: 7px 9px; } }
  </style>
</head>
<body>
  <main class="shell">
    <header class="topbar">
      <div class="brand"><div class="mark">✦</div><div><h1>Essentials Movedex</h1><p>Live move registry from the running game</p></div></div>
      <div class="search"><input id="search" type="search" placeholder="Search by name, ID, number, type…" autocomplete="off"><span id="count" class="count">Loading…</span></div>
      <button id="refresh" class="primary">Refresh</button>
    </header>
    <section class="toolbar" aria-label="Move filters">
      <select id="type"><option value="">All types</option></select>
      <select id="category"><option value="">All categories</option></select>
      <select id="sort"><option value="relevance">Best match</option><option value="name">Name A–Z</option><option value="power">Highest power</option><option value="accuracy">Highest accuracy</option><option value="pp">Highest PP</option></select>
      <label class="check"><input id="tm" type="checkbox"> Has TM</label>
      <label>Power <input id="power-min" type="number" min="0" placeholder="min">–<input id="power-max" type="number" min="0" placeholder="max"></label>
      <button id="clear" class="ghost">Clear filters</button>
    </section>
    <section class="layout">
      <aside class="list-pane"><div id="list-head" class="list-head">Catalog</div><div id="list"></div></aside>
      <article id="detail" class="detail-pane"><div class="empty">Choose a move to inspect its game data.</div></article>
    </section>
  </main>
  <script>
  (() => {
    'use strict';
    const $ = id => document.getElementById(id);
    const state = { rows: [], selected: null, loading: false, error: '', notice: '' };
    const arr = value => Array.isArray(value) ? value : [];
    const text = (value, fallback = '—') => value == null || value === '' ? fallback : String(value);
    const esc = value => text(value, '').replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
    const number = value => value == null || value === '' ? null : Number(value);
    const label = value => text(value).replace(/[_-]+/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
    const normalized = value => text(value, '').toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '').replace(/[^a-z0-9]/g, '');
    function api(target, options) { return fetch(target, options).then(async response => { const body = await response.text(); let value; try { value = body ? JSON.parse(body) : {}; } catch (_) { throw Error('The game returned invalid JSON.'); } if (!response.ok || value.error) throw Error(value.error || `Request failed (${response.status})`); return value; }); }
    function moveScore(move) { const query = normalized($('search').value); if (!query) return 0; const candidates = [move.name, move.id, move.number, move.type_name, move.category_name].map(normalized); let best = Infinity; for (const candidate of candidates) { if (!candidate) continue; if (candidate === query) best = Math.min(best, 0); else if (candidate.startsWith(query)) best = Math.min(best, 10 + candidate.length - query.length); else { const index = candidate.indexOf(query); if (index >= 0) best = Math.min(best, 30 + index); } } return best === Infinity ? null : best; }
    function filtered() {
      const type = $('type').value, category = $('category').value, tm = $('tm').checked;
      const min = number($('power-min').value), max = number($('power-max').value), rows = [];
      for (const move of state.rows) { const score = moveScore(move); if (score == null || type && move.type !== type || category && move.category_name !== category || tm && !arr(move.machines).length || min != null && (number(move.power) == null || number(move.power) < min) || max != null && (number(move.power) == null || number(move.power) > max)) continue; rows.push({ move, score }); }
      const sort = $('sort').value;
      rows.sort((a, b) => sort === 'name' ? text(a.move.name).localeCompare(text(b.move.name)) : sort === 'power' ? (number(b.move.power) || -1) - (number(a.move.power) || -1) : sort === 'accuracy' ? (number(b.move.accuracy) || -1) - (number(a.move.accuracy) || -1) : sort === 'pp' ? (number(b.move.pp) || -1) - (number(a.move.pp) || -1) : a.score - b.score || text(a.move.name).localeCompare(text(b.move.name))); return rows.map(row => row.move);
    }
    function typeColor(type) { return ({ FIRE:'#ff886d', WATER:'#70b8ff', GRASS:'#72d591', ELECTRIC:'#f4d267', ICE:'#8ee7ef', PSYCHIC:'#e89ad5', POISON:'#c68ae2', GHOST:'#a794d8', DRAGON:'#9ea8ff', DARK:'#9b8d9e', STEEL:'#b8c5d6', GROUND:'#d0ae7b', ROCK:'#d4a477', FIGHTING:'#e78e89', BUG:'#b2ce70', FAIRY:'#f8a8ca' })[String(type || '').toUpperCase()] || '#9ea8bb'; }
    function tag(value, color) { return value ? `<span class="tag" style="--tag:${color || typeColor(value)}">${esc(value)}</span>` : ''; }
    function renderFilters() { const types = [...new Set(state.rows.map(row => row.type).filter(Boolean))].sort(); const categories = [...new Set(state.rows.map(row => row.category_name).filter(Boolean))].sort(); $('type').innerHTML = '<option value="">All types</option>' + types.map(value => `<option>${esc(value)}</option>`).join(''); $('category').innerHTML = '<option value="">All categories</option>' + categories.map(value => `<option>${esc(value)}</option>`).join(''); }
    function renderList(rows) { $('count').textContent = `${rows.length} / ${state.rows.length} moves`; $('list-head').textContent = rows.length ? `${rows.length} matching moves` : 'No matching moves'; $('list').innerHTML = rows.length ? rows.map(move => `<div class="row ${state.selected === move.id ? 'selected' : ''}" data-id="${esc(move.id)}"><div class="orb">${esc(move.type || 'MOVE').slice(0, 4)}</div><div><strong>${esc(move.name || move.id)}</strong><small>${esc(move.id)} · ${esc(move.category_name || 'Unknown')}</small></div><div class="mini">${move.power == null ? '—' : esc(move.power)}<br><span>${move.pp == null ? '—' : esc(move.pp)} PP</span></div></div>`).join('') : '<div class="empty" style="min-height:180px">Try clearing a filter or searching for a different move.</div>'; }
    function stat(name, value) { return `<div class="stat"><label>${esc(name)}</label><b>${esc(value)}</b></div>`; }
    function renderDetail(move) { if (!move) { $('detail').innerHTML = state.error ? `<div class="error">${esc(state.error)}</div>` : '<div class="empty">Choose a move to inspect its game data.</div>'; return; } const machines = arr(move.machines); $('detail').innerHTML = `<div class="hero"><div><div class="eyebrow">Move #${esc(move.number || '—')}</div><h2>${esc(move.name || move.id)}</h2><div class="id">${esc(move.id)}</div></div><div class="hero-tags">${tag(move.type_name || move.type)}${tag(move.category_name, '#b879ff')}</div></div><div class="cards">${stat('Power', text(move.power))}${stat('Accuracy', move.accuracy === 0 ? 'Never misses' : move.accuracy == null ? '—' : `${move.accuracy}%`)}${stat('Base PP', text(move.pp))}${stat('Priority', text(move.priority, '0'))}${stat('Effect', move.effect_chance == null ? '—' : `${move.effect_chance}%`)}${stat('Target', text(move.target))}</div><section class="section"><h3>Description</h3><div class="description">${esc(move.description || 'This move has no translated description in the active game.')}</div></section><section class="section"><h3>Machines <span class="muted">· ${machines.length} available</span></h3>${machines.length ? `<div class="machine-list">${machines.map(machine => `<div class="machine"><code>${esc(machine.id)}</code><span>${esc(machine.name || machine.id)}</span><button data-add="${esc(machine.id)}">Add ×1</button></div>`).join('')}</div>` : '<div class="muted">No machine item teaches this move in the active game.</div>'}<div id="notice"></div></section><section class="section"><div class="actions"><button id="copy">Copy move JSON</button><button id="raw">Show raw game fields</button></div><pre id="raw-data" class="muted" hidden style="white-space:pre-wrap;overflow:auto">${esc(JSON.stringify(move.raw || {}, null, 2))}</pre></section>`; $('copy').onclick = () => navigator.clipboard?.writeText(JSON.stringify(move, null, 2)).then(() => notice('Move data copied.')); $('raw').onclick = event => { const raw = $('raw-data'); raw.hidden = !raw.hidden; event.target.textContent = raw.hidden ? 'Show raw game fields' : 'Hide raw game fields'; }; document.querySelectorAll('[data-add]').forEach(button => button.onclick = () => addMachine(button.dataset.add)); }
    function notice(message, error = false) { const node = $('notice'); if (node) node.innerHTML = `<div class="notice ${error ? 'error' : ''}">${esc(message)}</div>`; }
    function addMachine(item) { notice('Adding machine to the bag…'); api('/v1/pokemon-essentials/inventory/add?' + new URLSearchParams({ item, quantity:'1' }), { method:'POST' }).then(value => notice(`${text(value.name, item)} added · ${text(value.total, 'bag total unavailable')} total.`)).catch(error => notice(error.message, true)); }
    function render() { const rows = filtered(); if (!rows.some(move => move.id === state.selected)) state.selected = rows[0]?.id || null; renderList(rows); renderDetail(rows.find(move => move.id === state.selected)); }
    function load(refresh = false) { state.error = ''; state.loading = true; $('count').textContent = 'Loading…'; api('/v1/pokemon-essentials/movedex' + (refresh ? '?refresh=refresh' : '')).then(value => { state.rows = arr(value.entries); renderFilters(); render(); }).catch(error => { state.error = error.message; state.rows = []; render(); $('count').textContent = 'Unavailable'; }).finally(() => { state.loading = false; }); }
    ['search','type','category','sort','tm','power-min','power-max'].forEach(id => $(id).addEventListener(id === 'search' ? 'input' : 'change', render));
    $('clear').onclick = () => { $('search').value = ''; $('type').value = ''; $('category').value = ''; $('sort').value = 'relevance'; $('tm').checked = false; $('power-min').value = ''; $('power-max').value = ''; render(); };
    $('refresh').onclick = () => load(true);
    $('list').onclick = event => { const row = event.target.closest('[data-id]'); if (row) { state.selected = row.dataset.id; render(); } };
    load();
  })();
  </script>
</body>
</html>
fanplay:text html-end */

/* fanplay:text ruby-begin
# Move data is installed by this plugin instead of being compiled into the
# Rust host. The shared Essentials adapter still provides safe reflective
# helpers and legacy-game decoding, but this module owns the catalog builder.
module FANPLAY
  module MovedexPlugin
    def self.install
      @api = FANPLAY::GameAPI
      @original = @api.method(:pokemon_essentials_movedex) if @api.respond_to?(:pokemon_essentials_movedex)
      @api.define_singleton_method(:pokemon_essentials_movedex) do |refresh = false|
        FANPLAY::MovedexPlugin.pokemon_essentials_movedex(refresh)
      end
    end

    def self.uninstall
      return unless @api
      if @original
        original = @original
        @api.define_singleton_method(:pokemon_essentials_movedex) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :pokemon_essentials_movedex) rescue nil
      end
      @api = nil
      @original = nil
      @catalog = nil
    end

    def self.pokemon_essentials_movedex(refresh = false)
      return legacy_catalog(refresh) if @api.legacy_essentials?
      @catalog = nil if refresh
      return @catalog if @catalog
      records = @api.dex_registry(:Move)
      machines = Hash.new { |hash, key| hash[key] = [] }
      if GameData.const_defined?(:Item, false) && GameData::Item.respond_to?(:each)
        @api.dex_registry(:Item).each do |item|
          is_machine = item.respond_to?(:is_TM?) ? item.is_TM? : @api.read(item, :field_use) == 3
          move = @api.read(item, :move)
          next unless is_machine && move
          id = @api.dex_id(item)
          machines[move.to_s] << { 'id' => id, 'name' => @api.read_alias(item, :display_name, :name) || id }
        end
      end
      @catalog = { 'entries' => records.map do |record|
        id = @api.dex_id(record)
        row = @api.dex_move(id, 'Game move')
        category = @api.read(record, :category)
        row.merge('number' => @api.read(record, :id_number),
          'type_name' => @api.dex_named(:Type, @api.read(record, :type))['name'],
          'category_name' => { 0 => 'Physical', 1 => 'Special', 2 => 'Status' }[category] || category.to_s,
          'target' => @api.dex_named(:Target, @api.read(record, :target))['name'],
          'effect_chance' => @api.read(record, :effect_chance), 'machines' => machines[id],
          'flags' => Array(@api.read(record, :flags)).map(&:to_s), 'raw' => @api.extension_data(record))
      end }
    end

    def self.legacy_catalog(refresh = false)
      @legacy_catalog = nil if refresh
      return @legacy_catalog if @legacy_catalog
      rows = []
      machines = @api.legacy_move_machines
      (1..(defined?(PBMoves) ? PBMoves.maxValue : 0)).each do |id|
        details = @api.legacy_move_details(id)
        next unless details && details['name'] && !details['name'].to_s.strip.empty?
        rows << details.merge('number' => nil,
          'category_name' => { 0 => 'Physical', 1 => 'Special', 2 => 'Status' }[details['category'].to_i] || details['category'].to_s,
          'machines' => machines[id] || [], 'raw' => @api.legacy_move_raw(id, details))
      end
      @legacy_catalog = { 'entries' => rows }
    end
  end
end
fanplay:text ruby-end */
