// fanplay:id = essentials-encounters
// fanplay:name = Essentials Map Encounters
// fanplay:version = 0.1.0
// fanplay:cores = rgss
// fanplay:capabilities = ruby.inject,ruby.unload,ui.webview,pokemon_essentials.read
// fanplay:panel = encounters,Current Map Encounters

// The encounter endpoint is supplied by the generic Essentials game API. This
// plugin owns the HTML UI and watches the status endpoint for map changes.
fn on_activate() {
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + "; FANPLAY::EncountersPlugin.install }, revert: proc { FANPLAY::EncountersPlugin.uninstall if defined?(FANPLAY::EncountersPlugin) } }";
    let pending = ruby_inject("essentials-encounters", apply);
    state_set("pending", pending);
    if pending < 0 {
        state_set("error", last_error());
    }
}

fn build_ui() {
    let error = state_get("error");
    if error != () && error != "" {
        return [ui_label("encounters.error", error)];
    }
    if state_get("patch_handle") == () {
        return [ui_loading("encounters.installing", "Installing the encounter viewer…")];
    }
    [
        ui_button("encounters.open", "Open Current Map Encounters (HTML)"),
        ui_label("encounters.note", "The view checks the active map and refreshes encounter tables when the map or encounter version changes.")
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "encounters.open" {
        webview_open("essentials-encounters", "Current Map Encounters", text_block("html"));
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
  <title>Current Map Encounters</title>
  <style>
    :root { color-scheme: dark; --bg:#0b0d15; --panel:#151829; --panel-2:#1d2137; --line:#303650; --text:#f4f5ff; --muted:#9da6c4; --accent:#ae7bff; --blue:#6fd8ff; --good:#72dfb2; --danger:#ff8f9f; }
    * { box-sizing: border-box; }
    html, body { width:100%; height:100%; overflow:hidden; }
    body { margin:0; background:radial-gradient(circle at 80% 0,#28224d 0,transparent 38%),var(--bg); color:var(--text); font:14px/1.45 Inter,ui-sans-serif,system-ui,sans-serif; }
    button { border:1px solid var(--line); border-radius:9px; background:#222740; color:var(--text); padding:8px 13px; font:inherit; cursor:pointer; }
    button:hover { border-color:var(--accent); background:#2c3151; }
    button.primary { border-color:transparent; background:linear-gradient(135deg,#9a65ed,#4d9bda); font-weight:700; }
    .shell { height:100%; min-height:0; display:flex; flex-direction:column; overflow:hidden; }
    header { display:flex; align-items:center; gap:15px; padding:18px 22px 15px; border-bottom:1px solid var(--line); flex:0 0 auto; background:rgba(11,13,21,.88); }
    .mark { width:39px; height:39px; display:grid; place-items:center; border-radius:13px; background:linear-gradient(145deg,var(--accent),var(--blue)); color:#131425; font-size:22px; font-weight:900; }
    h1 { margin:0; font-size:18px; } header p { margin:2px 0 0; color:var(--muted); font-size:12px; }
    .head-copy { flex:1; min-width:0; } .head-actions { display:flex; align-items:center; gap:10px; }
    #map-label { color:var(--muted); font-size:12px; white-space:nowrap; }
    .content { flex:1; min-height:0; overflow:auto; padding:20px clamp(16px,4vw,52px) 36px; }
    .hero { display:flex; align-items:flex-start; justify-content:space-between; gap:18px; margin-bottom:18px; }
    .eyebrow { color:var(--blue); text-transform:uppercase; letter-spacing:.14em; font-size:11px; font-weight:800; }
    h2 { margin:5px 0 3px; font-size:clamp(23px,4vw,34px); line-height:1.08; } .muted { color:var(--muted); }
    .notice { padding:13px 15px; border:1px solid var(--line); border-radius:12px; background:rgba(21,24,41,.82); color:var(--muted); }
    .notice.error { color:var(--danger); border-color:rgba(255,143,159,.4); }
    .tables { display:grid; grid-template-columns:repeat(auto-fit,minmax(320px,1fr)); gap:14px; }
    .table { border:1px solid var(--line); border-radius:14px; overflow:hidden; background:rgba(21,24,41,.82); }
    .table-head { padding:13px 15px; background:linear-gradient(100deg,rgba(174,123,255,.16),rgba(111,216,255,.05)); border-bottom:1px solid var(--line); }
    .table-head h3 { margin:0; font-size:15px; } .table-head span { display:block; color:var(--muted); font-size:12px; margin-top:2px; }
    .entry { display:grid; grid-template-columns:45px 1fr auto; align-items:center; gap:11px; padding:11px 14px; border-bottom:1px solid rgba(48,54,80,.62); }
    .entry:last-child { border-bottom:0; }
    .orb { width:40px; height:40px; display:grid; place-items:center; border-radius:13px; background:#282d4b; color:var(--blue); font-weight:800; font-size:11px; overflow:hidden; }
    .orb img { width:40px; height:40px; object-fit:contain; image-rendering:pixelated; }
    .entry strong { display:block; } .entry small { display:block; color:var(--muted); margin-top:2px; }
    .odds { text-align:right; color:#d6baff; font-size:12px; white-space:nowrap; } .odds small { color:var(--muted); }
    .empty { min-height:240px; display:grid; place-items:center; text-align:center; color:var(--muted); padding:20px; }
    .spinner { display:inline-block; width:14px; height:14px; margin-right:7px; vertical-align:-2px; border:2px solid #4e5678; border-top-color:var(--blue); border-radius:50%; animation:spin .8s linear infinite; }
    @keyframes spin { to { transform:rotate(360deg); } }
    @media (max-width:640px) { header { flex-wrap:wrap; } .head-actions { width:100%; justify-content:space-between; } .tables { grid-template-columns:1fr; } }
  </style>
</head>
<body>
  <main class="shell">
    <header>
      <div class="mark">✦</div>
      <div class="head-copy"><h1>Current Map Encounters</h1><p>Live encounter tables from the running game</p></div>
      <div class="head-actions"><span id="map-label">Checking map…</span><button id="refresh" class="primary">Refresh</button></div>
    </header>
    <section class="content"><div id="hero" class="hero"><div><div class="eyebrow">Encounter browser</div><h2 id="title">Waiting for the game</h2><div id="subtitle" class="muted">Checking the active map…</div></div></div><div id="notice" class="notice"><span class="spinner"></span>Checking the active map…</div><div id="tables" class="tables" hidden></div></section>
  </main>
  <script>
  (() => {
    'use strict';
    const $ = id => document.getElementById(id);
    const state = { mapKey:null, mapId:null, loading:false, polling:false, timer:null, iconIndex:null, iconIndexLoading:false };
    const arr = value => Array.isArray(value) ? value : [];
    const text = (value, fallback = '—') => value == null || value === '' ? fallback : String(value);
    const esc = value => text(value, '').replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
    const number = value => value == null || value === '' ? null : Number(value);
    function api(path, options) { return fetch(path, options).then(async response => { const body = await response.text(); let value = {}; try { value = body ? JSON.parse(body) : {}; } catch (_) { throw Error('The game returned invalid JSON.'); } if (!response.ok || value.error) throw Error(value.error || `Request failed (${response.status})`); return value; }); }
    function assetSource(descriptor) { if (!descriptor || !window.FANPLAY || typeof window.FANPLAY.asset !== 'function') return Promise.resolve(null); const request = window.FANPLAY.asset(descriptor.category, descriptor.name).catch(() => null); return Promise.race([request, new Promise(resolve => setTimeout(() => resolve(null), 2500))]); }
    function hydrateIcons(root = document) { root.querySelectorAll('img[data-icon]').forEach(image => { try { const descriptor = JSON.parse(image.dataset.icon); assetSource(descriptor).then(src => { if (src) { image.src = src; image.hidden = false; } }); } catch (_) {} }); }
    function lookupIcon(entry) { if (!state.iconIndex) return entry.icon || null; return entry.icon || state.iconIndex.byId[String(entry.species)] || state.iconIndex.byName[ String(entry.name || '').toLowerCase() ] || state.iconIndex.byNumber[String(entry.dex_number)] || null; }
    function iconMarkup(entry) { const descriptor = lookupIcon(entry); return descriptor ? `<div class="orb"><img hidden data-icon='${esc(JSON.stringify(descriptor))}' alt="${esc(entry.name || entry.species || 'Pokémon')}"></div>` : `<div class="orb">${esc(entry.dex_number == null ? '★' : `#${entry.dex_number}`)}</div>`; }
    function hydrateFallbackIcons() { document.querySelectorAll('.entry[data-species]').forEach(node => { if (node.querySelector('img[data-icon]')) return; const descriptor = lookupIcon({ species:node.dataset.species, name:node.dataset.name, dex_number:node.dataset.number }); if (!descriptor) return; const orb = node.querySelector('.orb'); if (orb) { orb.innerHTML = `<img hidden data-icon='${esc(JSON.stringify(descriptor))}' alt="">`; } }); hydrateIcons($('tables')); }
    async function ensureIconIndex() { if (state.iconIndex || state.iconIndexLoading) return; state.iconIndexLoading = true; try { const value = await api('/v1/pokemon-essentials/pokedex'); const index = { byId:{}, byName:{}, byNumber:{} }; arr(value.species).forEach(row => { if (!row.sprite) return; if (row.id != null) index.byId[String(row.id)] = row.sprite; if (row.name) index.byName[String(row.name).toLowerCase()] = row.sprite; if (row.number != null) index.byNumber[String(row.number)] = row.sprite; }); state.iconIndex = index; hydrateFallbackIcons(); } catch (_) { state.iconIndex = { byId:{}, byName:{}, byNumber:{} }; } finally { state.iconIndexLoading = false; } }
    // The status endpoint intentionally exposes only the current map ID. Do
    // not include encounter_version here: it is present in the table reply
    // but not in status, and doing so would reload every polling interval.
    function mapKey(value) { return value && value.map_id != null ? String(value.map_id) : null; }
    function setNotice(message, error = false, loading = false) { $('notice').className = `notice${error ? ' error' : ''}`; $('notice').innerHTML = loading ? `<span class="spinner"></span>${esc(message)}` : esc(message); }
    function render(data) {
      const tables = arr(data.tables); $('title').textContent = data.map_id == null ? 'No active map' : `Map ${data.map_id}`; $('subtitle').textContent = data.current_encounter_type ? `Current encounter type: ${data.current_encounter_type}` : 'Encounter tables from the active map'; $('map-label').textContent = data.map_id == null ? 'No map' : `Map ${data.map_id}`;
      if (data.available === false || !tables.length) { $('tables').hidden = true; if (data.available === false) setNotice('No encounter data is available for this map.'); else setNotice('No encounter tables were reported for this map.'); return; }
      $('tables').hidden = false; $('tables').innerHTML = tables.map(table => { const chance = number(table.step_chance); return `<article class="table"><div class="table-head"><h3>${esc(table.name || table.type || 'Encounter type')}</h3><span>${chance == null ? 'Step chance unavailable' : `Step chance ${chance.toFixed(3)}%`}</span></div>${arr(table.entries).map(entry => { const min = number(entry.min_level), max = number(entry.max_level); const levels = min == null ? 'Level unknown' : max == null || min === max ? `Level ${min}` : `Levels ${min}–${max}`; const selection = number(entry.selection_percent), step = number(entry.step_percent); return `<div class="entry" data-species="${esc(entry.species)}" data-name="${esc(entry.name)}" data-number="${esc(entry.dex_number)}">${iconMarkup(entry)}<div><strong>${esc(entry.name || entry.species || 'Unknown species')}</strong><small>${esc(levels)} · slot ${esc((number(entry.slot) || 0) + 1)}</small></div><div class="odds">${selection == null ? '—' : `${selection.toFixed(3)}%`}<small>${step == null ? 'per slot' : `${step.toFixed(3)}% / step`}</small></div></div>`; }).join('') || '<div class="empty">No entries in this table.</div>'}</article>`; }).join(''); hydrateIcons($('tables')); ensureIconIndex(); setNotice('Live · encounter tables update when the map changes.'); }
    async function loadEncounters(key, force = false) { if (state.loading && !force) return; state.loading = true; setNotice('Loading encounter tables…', false, true); try { const data = await api('/v1/pokemon-essentials/encounters/current-map'); const nextKey = mapKey(data); state.mapKey = nextKey; state.mapId = data.map_id; render(data); } catch (error) { setNotice(error.message, true); } finally { state.loading = false; } }
    async function checkMap(force = false) { if (state.polling) return; state.polling = true; try { const status = await api('/v1/pokemon-essentials/status'); const nextKey = mapKey(status); if (status.available === true && (force || nextKey !== state.mapKey)) { await loadEncounters(nextKey, force); } else if (status.available !== true && state.mapKey == null) { $('map-label').textContent = 'Waiting for Essentials…'; setNotice('Waiting for Essentials to finish starting…', false, true); } } catch (error) { if (!state.mapKey) setNotice(error.message, true); } finally { state.polling = false; } }
    $('refresh').onclick = () => { state.mapKey = null; checkMap(true); };
    checkMap(true); state.timer = setInterval(() => checkMap(false), 750);
  })();
  </script>
</body>
</html>
fanplay:text html-end */

/* fanplay:text ruby-begin
# Add safe, lazy icon descriptors to encounter rows. The webview requests the
# actual image through the generic asset bridge only when it renders a row.
module FANPLAY
  module EncountersPlugin
    def self.install
      @api = FANPLAY::GameAPI
      @original = @api.method(:pokemon_essentials_encounters) if @api.respond_to?(:pokemon_essentials_encounters)
      return unless @original
      original = @original
      @api.define_singleton_method(:pokemon_essentials_encounters) do |*args|
        FANPLAY::EncountersPlugin.add_icons(original.call(*args))
      end
    end

    def self.uninstall
      return unless @api
      if @original
        original = @original
        @api.define_singleton_method(:pokemon_essentials_encounters) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :pokemon_essentials_encounters) rescue nil
      end
      @api = nil
      @original = nil
    end

    def self.add_icons(data)
      return data unless data.is_a?(Hash)
      tables = Array(data['tables']).map do |table|
        next table unless table.is_a?(Hash)
        table.merge('entries' => Array(table['entries']).map do |entry|
          next entry unless entry.is_a?(Hash)
          entry.merge('icon' => icon_descriptor(entry))
        end)
      end
      data.merge('tables' => tables)
    end

    def self.icon_descriptor(entry)
      species_id = entry['species']
      if defined?(FANPLAY::PokedexPlugin) && FANPLAY::PokedexPlugin.respond_to?(:sprite_data)
        summary = { 'id' => species_id, 'name' => entry['name'], 'number' => entry['dex_number'], 'form' => 0 }
        descriptor = FANPLAY::PokedexPlugin.sprite_data(species_id, summary, :icon)
        return descriptor if descriptor
      end
      record = @api.pokemon_essentials_species(species_id)
      species = @api.read(record, :species) || species_id
      form = (@api.read(record, :form) || 0).to_i
      if defined?(GameData::Species) && GameData::Species.respond_to?(:icon_filename)
        descriptor = asset_descriptor(@api.call(GameData::Species, :icon_filename, species, form, 0, false, false, false))
        return descriptor if descriptor
      end

      number = entry['dex_number'].to_i
      if @api.legacy_essentials?
        resolved = @api.legacy_kernel(:pbCheckPokemonIconFiles, [number.positive? ? number : species_id, false, false, form, false], false)
        descriptor = asset_descriptor(resolved)
        return descriptor if descriptor
      end

      values = [species_id, entry['name'], number.positive? ? format('%03d', number) : nil].compact.map(&:to_s).reject(&:empty?).uniq
      values += values.map(&:upcase)
      values.uniq.each do |value|
        %W[Graphics/Pokemon/Icons Graphics/Icons Graphics/Pokemon].each do |root|
          ['', 'icon'].each do |prefix|
            resolved = @api.legacy_kernel(:pbResolveBitmap, "#{root}/#{prefix}#{value}")
            descriptor = asset_descriptor(resolved)
            return descriptor if descriptor
          end
        end
      end
      nil
    rescue StandardError
      nil
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
