// fanplay:id = essentials-itemdex
// fanplay:name = Essentials Itemdex
// fanplay:cores = rgss
// fanplay:version = 0.1.0
// fanplay:capabilities = ruby.inject,ruby.unload,ui.webview,pokemon_essentials.read,pokemon_essentials.write
// fanplay:operation = pokemon_essentials_itemdex
// fanplay:operation-capabilities = pokemon_essentials_itemdex pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/itemdex pokemon_essentials_itemdex refresh
// fanplay:panel = itemdex,Essentials Itemdex

// Item data, filtering, presentation, and the catalog adapter belong to this
// plugin. The Rust host only supplies the generic webview, route, and Ruby
// injection primitives.
fn itemdex_html() {
    text_block("html")
}

fn on_activate() {
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + "; FANPLAY::ItemdexPlugin.install }, revert: proc { FANPLAY::ItemdexPlugin.uninstall if defined?(FANPLAY::ItemdexPlugin) } }";
    let pending = ruby_inject("essentials-itemdex", apply);
    state_set("pending", pending);
    if pending < 0 {
        state_set("error", last_error());
    }
}

fn build_ui() {
    let error = state_get("error");
    if error != () && error != "" {
        return [ui_label("itemdex.error", error)];
    }
    if state_get("patch_handle") == () {
        return [ui_loading("itemdex.installing", "Installing the Itemdex plugin…")];
    }
    [
        ui_button("itemdex.open", "Open Itemdex (HTML)"),
        ui_label("itemdex.note", "Search the active game's items, inspect usage and prices, and add items to the bag.")
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "itemdex.open" {
        webview_open("essentials-itemdex", "Essentials Itemdex", itemdex_html());
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
  <title>Essentials Itemdex</title>
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
    .topbar { display: flex; align-items: center; gap: 14px; padding: 18px 22px 14px; border-bottom: 1px solid var(--line); background: rgba(12,13,21,.88); flex: 0 0 auto; }
    .brand { display: flex; align-items: center; gap: 11px; min-width: 220px; }
    .mark { width: 38px; height: 38px; border-radius: 12px; display: grid; place-items: center; background: linear-gradient(145deg, var(--accent), var(--accent-2)); color: #121324; font-size: 21px; font-weight: 900; box-shadow: 0 0 30px rgba(184,121,255,.3); }
    .brand h1 { margin: 0; font-size: 17px; letter-spacing: .02em; }
    .brand p { margin: 2px 0 0; color: var(--muted); font-size: 12px; }
    .search { flex: 1; display: flex; align-items: center; gap: 8px; max-width: 720px; }
    input, select { min-width: 0; border: 1px solid var(--line); border-radius: 9px; background: #101220; color: var(--text); padding: 9px 11px; outline: none; }
    input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(184,121,255,.12); }
    #search { flex: 1; }
    .count { white-space: nowrap; color: var(--muted); font-size: 12px; }
    .toolbar { display: flex; gap: 9px; align-items: center; padding: 12px 22px; border-bottom: 1px solid rgba(44,48,72,.75); flex-wrap: wrap; flex: 0 0 auto; }
    .toolbar label { color: var(--muted); font-size: 12px; display: flex; align-items: center; gap: 6px; }
    .toolbar input[type=number] { width: 88px; padding: 7px 8px; }
    .toolbar select { padding: 7px 9px; }
    .check { user-select: none; }
    .check input { accent-color: var(--accent); }
    .layout { display: grid; grid-template-columns: minmax(285px, 34%) 1fr; flex: 1; min-height: 0; }
    .list-pane { min-height: 0; overflow: auto; border-right: 1px solid var(--line); padding: 13px; }
    .list-head { color: var(--muted); font-size: 12px; padding: 2px 4px 10px; }
    .row { display: grid; grid-template-columns: 46px 1fr auto; align-items: center; gap: 10px; padding: 9px; margin-bottom: 7px; border: 1px solid transparent; border-radius: 12px; cursor: pointer; background: rgba(27,30,49,.55); }
    .row:hover { border-color: #4f456b; background: #20233b; }
    .row.selected { border-color: var(--accent); background: linear-gradient(100deg, rgba(184,121,255,.18), rgba(113,216,255,.06)); }
    .orb { width: 42px; height: 42px; display: grid; place-items: center; border-radius: 13px; background: #282946; color: var(--accent-2); font-weight: 800; font-size: 10px; text-align: center; overflow: hidden; }
    .orb img { width: 38px; height: 38px; object-fit: contain; image-rendering: pixelated; }
    .row strong { display: block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .row small { display: block; color: var(--muted); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .row .mini { color: var(--muted); font-size: 11px; text-align: right; }
    .tag { display: inline-flex; align-items: center; min-height: 23px; padding: 3px 9px; margin: 2px 4px 2px 0; border-radius: 99px; background: rgba(184,121,255,.14); color: #d9c4ff; border: 1px solid rgba(184,121,255,.32); font-size: 11px; font-weight: 700; }
    .detail-pane { min-width: 0; min-height: 0; overflow: hidden; padding: 24px clamp(18px, 4vw, 54px); }
    .detail-scroll { height: 100%; overflow: auto; padding-right: 4px; }
    .empty { display: grid; place-items: center; min-height: 55vh; color: var(--muted); text-align: center; }
    .hero { display: flex; justify-content: space-between; gap: 20px; align-items: flex-start; padding-bottom: 22px; border-bottom: 1px solid var(--line); }
    .eyebrow { color: var(--accent-2); font-size: 11px; text-transform: uppercase; letter-spacing: .14em; font-weight: 800; }
    h2 { margin: 5px 0 2px; font-size: clamp(24px, 4vw, 38px); line-height: 1.05; }
    h3 { margin: 0 0 11px; font-size: 14px; }
    .id, .muted { color: var(--muted); }
    .hero-icon { width: 92px; height: 92px; display: grid; place-items: center; border-radius: 22px; background: linear-gradient(145deg, #282946, #171a2c); border: 1px solid var(--line); }
    .hero-icon img { width: 78px; height: 78px; object-fit: contain; image-rendering: pixelated; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 9px; margin: 18px 0; }
    .stat { padding: 11px 12px; border: 1px solid var(--line); border-radius: 11px; background: rgba(20,22,36,.75); }
    .stat label { display: block; color: var(--muted); font-size: 11px; margin-bottom: 3px; }
    .stat b { font-size: 14px; }
    .section { padding: 17px 0; border-top: 1px solid var(--line); }
    .description { color: #d9dcef; white-space: pre-wrap; }
    .prices { display: flex; flex-wrap: wrap; gap: 8px; }
    .price { padding: 9px 12px; border-radius: 10px; background: #1b1e31; border: 1px solid var(--line); }
    .price b { color: var(--good); }
    .action { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .action input { width: 82px; }
    .move { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px 12px; background: #1b1e31; border: 1px solid var(--line); border-radius: 10px; }
    .move code { color: var(--accent-2); }
    .notice { margin-top: 10px; color: var(--good); }
    .error { color: var(--danger); }
    pre { margin: 0; max-height: 220px; overflow: auto; padding: 12px; border-radius: 10px; background: #0e101b; color: #c7cce2; font: 12px/1.5 ui-monospace, SFMono-Regular, monospace; white-space: pre-wrap; }
    @media (max-width: 760px) { .topbar { flex-wrap: wrap; } .brand { min-width: 0; } .search { order: 3; flex-basis: 100%; } .layout { grid-template-columns: minmax(220px, 42%) 1fr; } .detail-pane { padding: 18px; } }
  </style>
</head>
<body>
  <main class="shell">
    <header class="topbar">
      <div class="brand"><div class="mark">✦</div><div><h1>Essentials Itemdex</h1><p>Live item registry from the running game</p></div></div>
      <div class="search"><input id="search" placeholder="Search by name, ID, number, pocket, tag…" autocomplete="off"><span id="count" class="count">Loading…</span><button id="refresh" class="primary">Refresh</button></div>
    </header>
    <section class="toolbar" aria-label="Item filters">
      <select id="pocket"><option value="">All pockets</option></select>
      <select id="tag-filter"><option value="">All item kinds</option></select>
      <select id="usage"><option value="">Any usage</option><option value="field">Field usable</option><option value="battle">Battle usable</option><option value="none">Not usable</option></select>
      <select id="sort"><option value="relevance">Best match</option><option value="name">Name A–Z</option><option value="price">Highest price</option><option value="number">National number</option></select>
      <label class="check"><input id="consumable" type="checkbox"> Consumable</label>
      <label>Price <input id="price-min" type="number" min="0" placeholder="min">–<input id="price-max" type="number" min="0" placeholder="max"></label>
      <button id="clear" class="ghost">Clear filters</button>
    </section>
    <section class="layout">
      <aside class="list-pane"><div id="list-head" class="list-head">Catalog</div><div id="list"></div></aside>
      <article id="detail" class="detail-pane"><div class="empty">Choose an item to inspect its game data.</div></article>
    </section>
  </main>
  <script>
  (() => {
    'use strict';
    const $ = id => document.getElementById(id);
    const state = { rows: [], selected: null, error: '' };
    const arr = value => Array.isArray(value) ? value : [];
    const text = (value, fallback = '—') => value == null || value === '' ? fallback : String(value);
    const esc = value => text(value, '').replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
    const number = value => value == null || value === '' ? null : Number(value);
    const normalized = value => text(value, '').toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '').replace(/[^a-z0-9]/g, '');
    const tags = row => Object.keys(row.tags || {});
    function api(target, options) { return fetch(target, options).then(async response => { const body = await response.text(); let value; try { value = body ? JSON.parse(body) : {}; } catch (_) { throw Error('The game returned invalid JSON.'); } if (!response.ok || value.error) throw Error(value.error || `Request failed (${response.status})`); return value; }); }
    function score(row) { const query = normalized($('search').value); if (!query) return 0; const fields = [row.name, row.id, row.number, row.pocket_name, row.description, ...tags(row), ...arr(row.flags)].map(normalized); let best = Infinity; fields.forEach((field, index) => { if (!field) return; const at = field.indexOf(query); if (at < 0) return; best = Math.min(best, at === 0 ? index * 4 : 30 + at); }); return best === Infinity ? null : best; }
    function filtered() {
      const pocket = $('pocket').value, tag = $('tag-filter').value, usage = $('usage').value, min = number($('price-min').value), max = number($('price-max').value), rows = [];
      for (const row of state.rows) { const value = number(row.price); const field = number(row.field_use), battle = number(row.battle_use); if (score(row) == null || pocket && row.pocket_name !== pocket || tag && !tags(row).includes(tag) || usage === 'field' && (field == null || field === 0) || usage === 'battle' && (battle == null || battle === 0) || usage === 'none' && (field !== 0 && battle !== 0) || $('consumable').checked && row.consumable !== true || min != null && (value == null || value < min) || max != null && (value == null || value > max)) continue; rows.push({ row, score: score(row) }); }
      const sort = $('sort').value; rows.sort((a, b) => sort === 'name' ? text(a.row.name).localeCompare(text(b.row.name)) : sort === 'price' ? (number(b.row.price) || -1) - (number(a.row.price) || -1) : sort === 'number' ? (number(a.row.number) || 999999) - (number(b.row.number) || 999999) : a.score - b.score || text(a.row.name).localeCompare(text(b.row.name))); return rows.map(value => value.row);
    }
    function assetSource(descriptor) { if (!descriptor || !window.FANPLAY || typeof window.FANPLAY.asset !== 'function') return Promise.resolve(null); const request = window.FANPLAY.asset(descriptor.category, descriptor.name).catch(() => null); return Promise.race([request, new Promise(resolve => setTimeout(() => resolve(null), 2500))]); }
    function icon(image, descriptor, size) { if (!image || !descriptor) return; assetSource(descriptor).then(src => { if (src) { image.src = src; image.hidden = false; image.style.width = `${size}px`; image.style.height = `${size}px`; } }); }
    function renderFilters() { const pockets = [...new Set(state.rows.map(row => row.pocket_name).filter(Boolean))].sort(); const kinds = [...new Set(state.rows.flatMap(tags))].sort(); $('pocket').innerHTML = '<option value="">All pockets</option>' + pockets.map(value => `<option>${esc(value)}</option>`).join(''); $('tag-filter').innerHTML = '<option value="">All item kinds</option>' + kinds.map(value => `<option>${esc(value)}</option>`).join(''); }
    function iconMarkup(row, cls = '') { return row.icon ? `<span class="orb ${cls}"><img hidden data-icon='${esc(JSON.stringify(row.icon))}' alt=""></span>` : `<span class="orb ${cls}">${esc(text(row.pocket_name, 'ITEM').slice(0, 5).toUpperCase())}</span>`; }
    function hydrateIcons(root = document) { root.querySelectorAll('img[data-icon]').forEach(image => { try { icon(image, JSON.parse(image.dataset.icon), image.closest('.hero-icon') ? 78 : 38); } catch (_) {} }); }
    function renderList(rows) { $('count').textContent = `${rows.length} / ${state.rows.length} items`; $('list-head').textContent = rows.length ? `${rows.length} matching items` : 'No matching items'; $('list').innerHTML = rows.length ? rows.map(row => `<div class="row ${state.selected === row.id ? 'selected' : ''}" data-id="${esc(row.id)}">${iconMarkup(row)}<div><strong>${esc(row.name || row.id)}</strong><small>${esc(row.id)} · ${esc(row.pocket_name || 'Unknown pocket')}</small></div><div class="mini">${row.price == null ? '—' : esc(row.price)}<br><span>${row.consumable ? 'consumable' : 'held'}</span></div></div>`).join('') : '<div class="empty" style="min-height:180px">Try clearing a filter or searching for another item.</div>'; hydrateIcons($('list')); }
    function chip(value) { return value ? `<span class="tag">${esc(value)}</span>` : ''; }
    function stat(name, value) { return `<div class="stat"><label>${esc(name)}</label><b>${esc(value)}</b></div>`; }
    function renderDetail(row) { if (!row) { $('detail').innerHTML = state.error ? `<div class="empty error">${esc(state.error)}</div>` : '<div class="empty">Choose an item to inspect its game data.</div>'; return; } const move = row.move; $('detail').innerHTML = `<div class="detail-scroll"><div class="hero"><div><div class="eyebrow">Item #${esc(row.number || '—')}</div><h2>${esc(row.name || row.id)}</h2><div class="id">${esc(row.id)} · ${esc(row.pocket_name || 'Unknown pocket')}</div></div>${row.icon ? `<div class="hero-icon"><img hidden data-icon='${esc(JSON.stringify(row.icon))}' alt="${esc(row.name)}"></div>` : ''}</div><div class="cards">${stat('Pocket', text(row.pocket_name))}${stat('Field use', text(row.field_use_label))}${stat('Battle use', text(row.battle_use_label))}${stat('Consumable', row.consumable == null ? 'Unknown' : row.consumable ? 'Yes' : 'No')}${stat('Buy price', row.price == null ? '—' : row.price)}${stat('Sell price', row.sell_price == null ? '—' : row.sell_price)}</div><section class="section"><h3>Item kinds</h3><div>${tags(row).map(chip).join('') || '<span class="muted">No game predicates reported.</span>'}</div></section><section class="section"><h3>Description</h3><div class="description">${esc(row.description || 'This item has no description in the active game.')}</div></section><section class="section"><h3>Inventory</h3><div class="action"><input id="quantity" type="number" min="1" max="9999" value="1"><button id="add" class="primary">Add to bag</button></div><div id="notice"></div></section>${move ? `<section class="section"><h3>Teaches this move</h3><div class="move"><div><code>${esc(move.id)}</code><br><strong>${esc(move.name || move.id)}</strong></div><span class="muted">${esc(move.method || 'Item')}</span></div></section>` : ''}<section class="section"><div class="action"><button id="copy">Copy item JSON</button><button id="raw-toggle" class="ghost">Show raw game fields</button></div><pre id="raw-data" hidden>${esc(JSON.stringify(row.raw || {}, null, 2))}</pre></section></div>`; hydrateIcons($('detail')); $('copy').onclick = () => navigator.clipboard?.writeText(JSON.stringify(row, null, 2)).then(() => notice('Item data copied.')); $('raw-toggle').onclick = event => { const raw = $('raw-data'); raw.hidden = !raw.hidden; event.target.textContent = raw.hidden ? 'Show raw game fields' : 'Hide raw game fields'; }; $('add').onclick = () => addItem(row.id); }
    function notice(message, error = false) { const node = $('notice'); if (node) node.innerHTML = `<div class="notice ${error ? 'error' : ''}">${esc(message)}</div>`; }
    function addItem(item) { const quantity = Math.max(1, Math.min(9999, Number($('quantity').value) || 1)); notice('Adding item to the bag…'); api('/v1/pokemon-essentials/inventory/add?' + new URLSearchParams({ item, quantity: String(quantity) }), { method:'POST' }).then(value => notice(`${text(value.name, item)} added · ${text(value.total, 'bag total unavailable')} total.`)).catch(error => notice(error.message, true)); }
    function render() { const rows = filtered(); if (!rows.some(row => row.id === state.selected)) state.selected = rows[0]?.id || null; renderList(rows); renderDetail(rows.find(row => row.id === state.selected)); }
    function load(refresh = false) { state.error = ''; $('count').textContent = 'Loading…'; api('/v1/pokemon-essentials/itemdex' + (refresh ? '?refresh=refresh' : '')).then(value => { state.rows = arr(value.entries); renderFilters(); render(); }).catch(error => { state.error = error.message; state.rows = []; render(); $('count').textContent = 'Unavailable'; }); }
    ['search','pocket','tag-filter','usage','sort','consumable','price-min','price-max'].forEach(id => $(id).addEventListener(id === 'search' ? 'input' : 'change', render));
    $('clear').onclick = () => { $('search').value = ''; $('pocket').value = ''; $('tag-filter').value = ''; $('usage').value = ''; $('sort').value = 'relevance'; $('consumable').checked = false; $('price-min').value = ''; $('price-max').value = ''; render(); };
    $('refresh').onclick = () => load(true);
    $('list').onclick = event => { const row = event.target.closest('[data-id]'); if (row) { state.selected = row.dataset.id; render(); } };
    load();
  })();
  </script>
</body>
</html>
fanplay:text html-end */

/* fanplay:text ruby-begin
# The catalog adapter is installed by this plugin. Shared reflective helpers
# remain in the generic Essentials bridge; the item registry and legacy shape
# conversion do not belong to the Rust host or its route table.
module FANPLAY
  module ItemdexPlugin
    def self.install
      @api = FANPLAY::GameAPI
      @original = @api.method(:pokemon_essentials_itemdex) if @api.respond_to?(:pokemon_essentials_itemdex)
      @api.define_singleton_method(:pokemon_essentials_itemdex) do |refresh = false|
        FANPLAY::ItemdexPlugin.pokemon_essentials_itemdex(refresh)
      end
    end

    def self.uninstall
      return unless @api
      if @original
        original = @original
        @api.define_singleton_method(:pokemon_essentials_itemdex) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :pokemon_essentials_itemdex) rescue nil
      end
      @api = nil
      @original = nil
      @catalog = nil
      @legacy_catalog = nil
    end

    def self.pokemon_essentials_itemdex(refresh = false)
      # The generic extension dispatcher passes route arguments as one array;
      # direct Ruby callers may still pass the old boolean/string form.
      refresh = refresh.is_a?(Array) ? refresh[0].to_s == 'refresh' : (refresh == true || refresh.to_s == 'refresh')
      return legacy_catalog(refresh) if @api.legacy_essentials?
      @catalog = nil if refresh
      return @catalog if @catalog
      @catalog = { 'entries' => @api.dex_registry(:Item).map do |record|
        id = @api.dex_id(record)
        pocket = @api.read(record, :pocket)
        pocket_names = defined?(PokemonBag) ? @api.call(PokemonBag, :pocket_names) : nil
        pocket_name = nil
        if pocket_names.is_a?(Array) && pocket.is_a?(Integer) && pocket.positive?
          offset = pocket_names.first.nil? || pocket_names.first.to_s.empty? ? 0 : 1
          pocket_name = pocket_names[pocket - offset]
        end
        tags = { 'Medicine' => :is_healing_item?, 'Berry' => :is_berry?, 'Poké Ball' => :is_poke_ball?,
                 'Key item' => :is_key_item?, 'TM / HM / TR' => :is_machine?, 'Evolution item' => :is_evolution_stone?,
                 'Mega Stone' => :is_mega_stone?, 'Can be held' => :can_hold?, 'Mail' => :is_mail?,
                 'Vitamin' => :is_vitamin?, 'Fossil' => :is_fossil? }.filter_map { |name, predicate| name if @api.call(record, predicate) }
        move = @api.read(record, :move)
        { 'id' => id, 'number' => @api.read(record, :id_number),
          'name' => @api.read_alias(record, :display_name, :name) || id,
          'description' => @api.read(record, :description), 'pocket' => pocket,
          'pocket_name' => pocket_name || (pocket.nil? ? 'Not supplied' : "Pocket #{pocket}"),
          'price' => @api.read(record, :price), 'sell_price' => @api.read(record, :sell_price), 'bp_price' => @api.read(record, :bp_price),
          'consumable' => @api.read_alias(record, :consumed_after_use?, :consumable),
          'field_use' => @api.read(record, :field_use), 'field_use_label' => item_usage(record, :field_use),
          'battle_use' => @api.read(record, :battle_use), 'battle_use_label' => item_usage(record, :battle_use),
          'tags' => tags, 'flags' => Array(@api.read(record, :flags)).map(&:to_s),
          'move' => move ? @api.dex_move(move, 'Taught by item') : nil,
          'icon' => asset_descriptor(@api.call(GameData::Item, :icon_filename, id)), 'raw' => @api.extension_data(record) }
      end }
    end

    def self.item_usage(record, key)
      value = @api.read(record, key)
      schema_key = key == :field_use ? 'FieldUse' : 'BattleUse'
      schema = GameData::Item.respond_to?(:schema) ? GameData::Item.schema : nil
      choices = schema && schema[schema_key] && schema[schema_key][2]
      name = choices.key(value) if choices.is_a?(Hash)
      return name.to_s.gsub(/([a-z])([A-Z])/) { "#{$1} #{$2}" } if name
      value.nil? ? 'Not supplied' : (value == 0 ? 'Not usable' : "Mode #{value}")
    rescue StandardError
      value.to_s
    end

    def self.legacy_catalog(refresh = false)
      @legacy_catalog = nil if refresh
      return @legacy_catalog if @legacy_catalog
      rows = @api.legacy_item_records.map do |id, record|
        machine = record[9].to_i
        { 'id' => @api.legacy_item_key(id), 'number' => id,
          'name' => @api.legacy_item_name(id) || @api.legacy_item_key(id),
          'description' => (@api.legacy_message(:ItemDescriptions, id) || record[5]),
          'pocket' => record[3], 'pocket_name' => @api.legacy_pocket_name(record[3]),
          'price' => record[4], 'sell_price' => nil, 'bp_price' => nil, 'consumable' => nil,
          'field_use' => record[6], 'field_use_label' => @api.legacy_item_use_label(record[6]),
          'battle_use' => record[7], 'battle_use_label' => @api.legacy_item_battle_use_label(record[7]),
          'tags' => @api.legacy_item_tags(record, id), 'flags' => [],
          'move' => machine.positive? ? @api.legacy_dex_move(machine, 'Taught by item') : nil,
          'icon' => asset_descriptor(@api.legacy_item_icon(id)), 'raw' => @api.legacy_item_raw(record, id) }
      end
      @legacy_catalog = { 'entries' => rows }
    end

    def self.asset_descriptor(path)
      normalized = path.to_s.tr('\\', '/')
      marker = normalized.index('Graphics/')
      return nil unless marker
      relative = normalized[(marker + 'Graphics/'.length)..]
      relative = relative.sub(/\.(?:png|bmp|gif|jpg|jpeg)\z/i, '')
      parts = relative.split('/').reject(&:empty?)
      name = parts.pop
      return nil if parts.empty? || name.nil? || name.empty?
      { 'category' => parts.join('/'), 'name' => name }
    end
  end
end
fanplay:text ruby-end */
