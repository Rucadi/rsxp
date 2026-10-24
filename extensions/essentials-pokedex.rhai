// rsxp:id = essentials-pokedex
// rsxp:name = Essentials Pokédex
// rsxp:version = 0.1.0
// rsxp:capabilities = ruby.inject,ruby.unload,ui.webview,pokemon_essentials.read,pokemon_essentials.write
// rsxp:operation = pokemon_essentials_pokedex
// rsxp:operation-capabilities = pokemon_essentials_pokedex pokemon_essentials.read
// rsxp:route = GET /v1/pokemon-essentials/pokedex pokemon_essentials_pokedex refresh
// rsxp:operation = pokemon_essentials_species_entry
// rsxp:operation-capabilities = pokemon_essentials_species_entry pokemon_essentials.read
// rsxp:route = GET /v1/pokemon-essentials/species/{species} pokemon_essentials_species_entry species
// rsxp:operation = pokemon_essentials_wild_encounter
// rsxp:operation-capabilities = pokemon_essentials_wild_encounter pokemon_essentials.write
// rsxp:operation-mutates = pokemon_essentials_wild_encounter
// rsxp:route = POST /v1/pokemon-essentials/wild-encounter pokemon_essentials_wild_encounter species,level,shiny,form,gender,nature,ability,item,max_ivs,friendship
// rsxp:panel = pokedex,Essentials Pokédex

// The complete explorer is one portable plugin. Its UI is HTML so the same
// source runs in the native Wry window and in the browser iframe host.
fn pokedex_html() {
    text_block("html")
}

/* rsxp:text html-begin
<!doctype html>
<html>
<head>
    <meta charset='utf-8'>
    <meta name='viewport' content='width=device-width,initial-scale=1'>
    <title>Essentials Pokédex</title>
    <style>
        * {
            box-sizing: border-box
        }

        body {
            margin: 0;
            background: #10141d;
            color: #e9edf5;
            font: 14px system-ui, sans-serif;
            height: 100vh;
            overflow: hidden
        }

        button,
        input,
        select {
            font: inherit
        }

        button {
            border: 1px solid #39465d;
            background: #1b2535;
            color: #edf3ff;
            border-radius: 6px;
            padding: 7px 10px;
            cursor: pointer
        }

        button:hover {
            background: #293852
        }

        .toolbar {
            display: flex;
            gap: 8px;
            align-items: center;
            padding: 12px;
            border-bottom: 1px solid #303b4f;
            position: sticky;
            top: 0;
            background: #10141d;
            z-index: 2
        }

        .toolbar input {
            flex: 1;
            min-width: 100px;
            background: #171e2b;
            border: 1px solid #39465d;
            color: #fff;
            border-radius: 6px;
            padding: 8px
        }

        .toolbar label {
            white-space: nowrap;
            color: #aeb9ca
        }

        .status {
            color: #93a4bd;
            font-size: 12px
        }

        .layout {
            display: grid;
            grid-template-columns: minmax(220px, 30%) 1fr;
            height: calc(100vh - 62px);
            min-height: 0
        }

        .list {
            border-right: 1px solid #303b4f;
            min-height: 0;
            overflow: auto
        }

        .row {
            display: flex;
            align-items: center;
            gap: 9px;
            padding: 10px 12px;
            border-bottom: 1px solid #232d3e;
            cursor: pointer
        }

        .row:hover,
        .row.selected {
            background: #1c2c45
        }

        .row strong {
            display: block
        }

        .row-copy {
            min-width: 0;
            flex: 1
        }

        .row small {
            color: #8e9db4
        }

        .thumb {
            flex: 0 0 42px;
            width: 42px;
            height: 42px;
            display: grid;
            place-items: center;
            border-radius: 7px;
            background: #171e2b;
            border: 1px solid #2b374b;
            overflow: hidden
        }

        .thumb.strip {
            place-items: start
        }

        .thumb img {
            width: 42px;
            height: 42px;
            object-fit: contain;
            image-rendering: pixelated;
            opacity: 0
        }

        .thumb img.ready {
            opacity: 1
        }

        .empty {
            padding: 18px;
            color: #98a6ba
        }

        .detail {
            padding: 18px;
            min-width: 0;
            min-height: 0;
            overflow: auto
        }

        .hero {
            display: flex;
            justify-content: space-between;
            gap: 12px;
            border-bottom: 1px solid #303b4f;
            padding-bottom: 14px
        }

        .hero h1 {
            margin: 0 0 4px;
            font-size: 25px
        }

        .art-panel {
            flex: 0 0 192px;
            width: 192px;
            display: grid;
            justify-items: center;
            gap: 4px
        }

        .art-label {
            color: #aeb9ca;
            font-size: 12px
        }

        .art-panel button {
            padding: 4px 7px;
            font-size: 12px
        }

        .art {
            flex: 0 0 192px;
            width: 192px;
            min-width: 192px;
            min-height: 192px;
            max-width: 192px;
            display: grid;
            place-items: center;
            border-radius: 12px;
            background: #171e2b;
            border: 1px solid #2b374b;
            overflow: hidden
        }

        .art img {
            width: 192px;
            height: 192px;
            object-fit: contain;
            image-rendering: pixelated
        }

        .art-fallback {
            padding: 12px;
            color: #91a0b5;
            text-align: center
        }

        .number {
            font-size: 20px;
            color: #7f91ad
        }

        .types {
            display: flex;
            gap: 6px
        }

        .type {
            border-radius: 99px;
            padding: 3px 9px;
            background: #374765;
            color: #fff;
            font-size: 12px
        }

        .tabs {
            display: flex;
            gap: 5px;
            flex-wrap: wrap;
            margin: 14px 0
        }

        .tabs button.active {
            background: #4c6da7;
            border-color: #6c92d4
        }

        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
            gap: 9px
        }

        .card {
            background: #171e2b;
            border: 1px solid #2b374b;
            border-radius: 8px;
            padding: 10px
        }

        .card h3 {
            margin: 0 0 8px;
            font-size: 13px;
            color: #aebdde
        }

        .card p {
            margin: 4px 0
        }

        .muted {
            color: #91a0b5
        }

        .table {
            width: 100%;
            border-collapse: collapse
        }

        .table th,
        .table td {
            text-align: left;
            padding: 7px;
            border-bottom: 1px solid #2b374b
        }

        .table th {
            color: #aebdde
        }

        .form {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 9px
        }

        .field {
            display: flex;
            flex-direction: column;
            gap: 4px;
            color: #aebdde
        }

        .field input,
        .field select {
            background: #111925;
            border: 1px solid #39465d;
            color: #fff;
            border-radius: 5px;
            padding: 7px
        }

        .wide {
            grid-column: 1/-1
        }

        .notice {
            margin-top: 10px;
            padding: 9px;
            border-radius: 6px;
            background: #1b2a3d;
            color: #b9d1f5
        }

        .error {
            color: #ff9d9d
        }

        .pill {
            display: inline-block;
            margin: 2px 4px 2px 0;
            padding: 3px 7px;
            background: #293852;
            border-radius: 5px
        }
    </style>
</head>

<body>
    <div class='toolbar'>
        <input id='search' placeholder='Search species, number or ID'>
        <label>
            <input id='known' type='checkbox'>
            known sources
        </label>
        <button id='refresh'>Refresh</button>
        <span id='status' class='status'>Loading…</span>
    </div>
    <div class='layout'>
        <div id='list' class='list'></div>
        <main id='detail' class='detail'>
            <div class='empty'>Choose a species.</div>
        </main>
    </div>
    <script>
    (function () {
      "use strict";
      const ART_SIZE = 192;
      const state = { rows: [], entry: null, sprite: null, shiny: false, tab: "overview" };
      const $ = (id) => document.getElementById(id);
      const esc = (v) =>
        String(v == null ? "—" : v).replace(
          /[&<>]/g,
          (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c],
        );
      const arr = (v) => (Array.isArray(v) ? v : []);
      const obj = (v) => (v && typeof v === "object" ? v : {});
      function setStatus(text, error) {
        $("status").textContent = text;
        $("status").className = error ? "status error" : "status";
      }
      function request(path, options = {}) {
        const method = String(options.method || "GET").toUpperCase();
        if (window.RSXP && typeof window.RSXP.api === "function" && path.startsWith("/v1/")) {
          return window.RSXP.api(method, path, options.body || "");
        }
        return fetch(path, options);
      }
      function json(path, options) {
        return request(path, options).then((r) =>
          r.text().then((t) => {
            let v;
            try {
              v = t ? JSON.parse(t) : null;
            } catch (e) {
              throw Error("Invalid server response");
            }
            if (!r.ok || (v && v.error))
              throw Error((v && v.error) || "HTTP " + r.status);
            return v;
          }),
        );
      }
      function filtered() {
        const q = $("search").value.trim().toLowerCase();
        const only = $("known").checked;
        return state.rows.filter(
          (r) =>
            (!only || r.source) &&
            (!q ||
              [r.name, r.id, r.number].some((v) =>
                String(v == null ? "" : v)
                  .toLowerCase()
                  .includes(q),
              )),
        );
      }
      function assetSource(descriptor) {
        if (!descriptor || !window.RSXP || typeof window.RSXP.asset !== "function") {
          return Promise.resolve(null);
        }
        const request = window.RSXP.asset(descriptor.category, descriptor.name)
          .catch(() => null);
        // A host that is shutting down or has no asset backend must not leave
        // a plugin promise pending forever (notably the shiny availability
        // probe).
        const timeout = new Promise((resolve) => setTimeout(() => resolve(null), 2500));
        return Promise.race([request, timeout]);
      }
      function loadAssetImage(image, descriptor, size) {
        if (!image || !descriptor) return Promise.resolve(null);
        return assetSource(descriptor).then((src) => {
          if (!src) return null;
          image.src = src;
          image.classList.add("ready");
          image.style.visibility = "visible";
          image.style.width = `${size}px`;
          image.style.height = `${size}px`;
          image.style.objectFit = "contain";
          return src;
        });
      }
      function loadListArtwork() {
        const images = [...document.querySelectorAll(".thumb img[data-asset-name]")];
        const load = (image) => {
          if (image.dataset.loaded) return;
          image.dataset.loaded = "1";
          loadAssetImage(image, {
            category: image.dataset.assetCategory,
            name: image.dataset.assetName,
          }, 42);
        };
        if ("IntersectionObserver" in window) {
          const observer = new IntersectionObserver((entries) => entries.forEach((entry) => {
            if (!entry.isIntersecting) return;
            load(entry.target);
            observer.unobserve(entry.target);
          }), { root: $("list"), rootMargin: "160px" });
          images.forEach((image) => observer.observe(image));
        } else {
          images.forEach(load);
        }
      }
      function renderList() {
        const rows = filtered();
        $("list").innerHTML = rows.length
          ? rows
              .map(
                (r) =>
                  `<div class='row ${state.entry && state.entry.id === r.id ? "selected" : ""}' data-id='${esc(r.id)}'><div class='thumb'><img class='row-sprite' alt=''${r.sprite ? ` data-asset-category='${esc(r.sprite.category)}' data-asset-name='${esc(r.sprite.name)}'` : ""}></div><div class='row-copy'><strong>${esc(r.number ? "#" + r.number + " · " : "")}${esc(r.name)}</strong><small>${esc(r.id)}${r.form_name ? " · " + esc(r.form_name) : ""}${r.source ? " · " + esc(r.source) : ""}</small></div></div>`,
              )
              .join("")
          : `<div class=empty>No species match this search.</div>`;
        loadListArtwork();
        $("status").textContent = `${rows.length} of ${state.rows.length} species`;
      }
      function card(title, body) {
        return `<section class='card'><h3>${esc(title)}</h3>${body}</section>`;
      }
      function list(values) {
        return (
          arr(values)
            .map(
              (v) =>
                `<span class='pill'>${esc(typeof v === "object" ? v.name || v.id : v)}</span>`,
            )
            .join("") || "<span class=muted>None supplied</span>"
        );
      }
      function renderOverview() {
        const e = state.entry;
        const stats = obj(e.stats);
        const facts = obj(e.facts);
        return `<div class='grid'>${card("Pokédex entry", `<p>${esc(e.entry || "No entry text supplied.")}</p>`)}${card("Types", `<div class='types'>${list(e.types)}</div>`)}${card(
          "Stats",
          Object.keys(stats)
            .map(
              (k) =>
                `<p><b>${esc(k)}</b> <span class=muted>${esc(stats[k])}</span></p>`,
            )
            .join("") || "<span class=muted>No stats supplied</span>",
        )}${card("Abilities", `<p>${list(e.abilities)}</p><p><b>Hidden:</b> ${list(e.hidden_abilities)}</p>`)}${card(
          "Facts",
          Object.keys(facts)
            .map(
              (k) =>
                `<p><b>${esc(k.replaceAll("_", " "))}:</b> ${esc(facts[k])}</p>`,
            )
            .join("") || "<span class=muted>No facts supplied</span>",
        )}</div>`;
      }
      function renderLocations() {
        const rows = arr(state.entry.locations);
        return rows.length
          ? `<table class='table'><tr><th>Map</th><th>Encounter</th><th>Levels</th><th>Chance</th></tr>${rows.map((r) => `<tr><td>${esc(r.map_name || r.map_id)}</td><td>${esc(r.type)}${r.version ? " · v" + esc(r.version) : ""}</td><td>${esc(r.min_level)}–${esc(r.max_level)}</td><td>${esc(r.percent == null ? "—" : r.percent + "%")}</td></tr>`).join("")}</table>`
          : "<p class=muted>No indexed locations.</p>";
      }
      function renderMoves() {
        const rows = arr(state.entry.moves);
        return rows.length
          ? `<table class='table'><tr><th>Move</th><th>Method</th><th>Type</th><th>Power</th></tr>${rows.map((r) => `<tr><td>${esc(r.name || r.id)}</td><td>${esc(r.method)}${r.level != null ? " · Lv " + esc(r.level) : ""}</td><td>${esc(r.type)}</td><td>${esc(r.power == null ? "—" : r.power)}</td></tr>`).join("")}</table>`
          : "<p class=muted>No move data.</p>";
      }
      function renderEvolution() {
        const edge = (r, reverse) =>
          `<div class='card'><b>${reverse ? "Previous" : "Evolves into"} ${esc(r.name || r.id)}</b><p class=muted>${esc(r.condition || r.method || "")}</p></div>`;
        return `<div class='grid'>${arr(state.entry.previous)
          .map((r) => edge(r, true))
          .join("")}${
          arr(state.entry.evolutions)
            .map((r) => edge(r, false))
            .join("") || "<p class=muted>No evolution data.</p>"
        }</div>`;
      }
      function renderCheats() {
        const e = state.entry;
        return `<div class='card'><h3>Encounter generator</h3><p class=muted>Queues a wild encounter through the plugin Ruby binding on the game thread.</p><form id='cheat' class='form'><label class='field'>Level<input name='level' type='number' min='1' max='100' value='5'></label><label class='field'>Form<input name='form' type='number' min='0' placeholder='default'></label><label class='field'>Gender<select name='gender'><option value=''>default</option><option>male</option><option>female</option></select></label><label class='field'>Nature<input name='nature' placeholder='default'></label><label class='field'>Ability<input name='ability' placeholder='default'></label><label class='field'>Held item<input name='item' placeholder='default'></label><label class='field'>Friendship<input name='friendship' type='number' min='0' max='255' placeholder='default'></label><label class='field'><span>Options</span><span><input name='shiny' type='checkbox'> shiny</span><span><input name='max_ivs' type='checkbox'> max IVs</span></label><div class='wide'><button type='submit'>Generate ${esc(e.name)}</button></div></form><div id='cheat-result'></div></div>`;
      }
      function renderDetail() {
        const e = state.entry;
        if (!e) {
          $("detail").innerHTML = "<div class=empty>Choose a species.</div>";
          return;
        }
        const tabs = [
          ["overview", "Overview"],
          ["locations", "Locations"],
          ["moves", "Moves"],
          ["evolution", "Evolution"],
          ["cheats", "Cheats"],
        ];
        const body =
          state.tab === "overview"
            ? renderOverview()
            : state.tab === "locations"
              ? renderLocations()
              : state.tab === "moves"
                ? renderMoves()
                : state.tab === "evolution"
                  ? renderEvolution()
                  : renderCheats();
        const artworkData = state.shiny ? e.shiny_sprite : e.sprite;
        const artworkLabel = state.shiny ? "Shiny" : "Normal";
        const shinyKnown = e.shiny_available !== undefined;
        const shinyAvailable = e.shiny_available === true;
        const artworkAttrs = artworkData
          ? ` data-asset-category='${esc(artworkData.category)}' data-asset-name='${esc(artworkData.name)}'`
          : "";
        $("detail").innerHTML =
          `<div class='hero'><div><h1>${esc(e.name)} <span class='number'>#${esc(e.number || "—")}</span></h1><div class=muted>${esc(e.id)}${e.form_name ? " · " + esc(e.form_name) : ""}</div></div><div class='art-panel'><div class='art-label'>${artworkLabel}</div><div class='art'><img id='sprite' alt='${esc(e.name)} ${artworkLabel.toLowerCase()} artwork' style='visibility:hidden'${artworkAttrs}><span id='sprite-fallback' class='art-fallback'>No artwork found</span></div><button id='shiny-toggle' type='button'${shinyKnown && shinyAvailable ? "" : " disabled"}>${!shinyKnown ? "Checking shiny…" : shinyAvailable ? (state.shiny ? "Show normal" : "Show shiny") : "Shiny unavailable"}</button></div><div class='types'>${list(e.types)}</div></div><div class='tabs'>${tabs.map((t) => `<button class='${state.tab === t[0] ? "active" : ""}' data-tab='${t[0]}'>${t[1]}</button>`).join("")}</div>${body}`;
        const artwork = $("sprite");
        if (artwork && artworkData) {
          loadAssetImage(artwork, artworkData, ART_SIZE).then((src) => {
            if (src) {
              $("sprite-fallback").hidden = true;
            }
          });
        }
        if (!shinyKnown) {
          const shinyProbe = e.shiny_sprite
            ? assetSource(e.shiny_sprite)
            : Promise.resolve(null);
          shinyProbe.then((src) => {
            if (state.entry !== e) return;
            e.shiny_available = !!src;
            renderDetail();
          });
        }
        document.querySelectorAll("[data-tab]").forEach(
          (b) =>
            (b.onclick = () => {
              state.tab = b.dataset.tab;
              renderDetail();
            }),
        );
        const shinyToggle = $("shiny-toggle");
        if (shinyToggle && e.shiny_sprite) {
          shinyToggle.onclick = () => {
            state.shiny = !state.shiny;
            renderDetail();
          };
        }
        const form = $("cheat");
        if (form) form.onsubmit = sendCheat;
      }
      function selectSpecies(id) {
        setStatus("Loading details…");
        json("/v1/pokemon-essentials/species/" + encodeURIComponent(id))
          .then((e) => {
            state.entry = e;
            state.sprite = e.sprite || null;
            state.shiny = false;
            state.tab = "overview";
            renderList();
            renderDetail();
            setStatus(`${state.rows.length} species`);
          })
          .catch((e) => setStatus(e.message, true));
      }
      function loadDex(refresh) {
        setStatus("Loading Pokédex…");
        json("/v1/pokemon-essentials/pokedex" + (refresh ? "?refresh=refresh" : ""))
          .then((v) => {
            state.rows = arr(v && v.species);
            renderList();
            if (state.rows.length && !state.entry) selectSpecies(state.rows[0].id);
            else setStatus(`${state.rows.length} species`);
          })
          .catch((e) => setStatus(e.message, true));
      }
      function sendCheat(ev) {
        ev.preventDefault();
        const f = new FormData(ev.target);
        const params = new URLSearchParams({
          species: state.entry.id,
          level: f.get("level") || "5",
          shiny: f.get("shiny") ? "true" : "false",
          form: f.get("form") || "",
          gender: f.get("gender") || "",
          nature: f.get("nature") || "",
          ability: f.get("ability") || "",
          item: f.get("item") || "",
          max_ivs: f.get("max_ivs") ? "true" : "false",
          friendship: f.get("friendship") || "",
        });
        $("cheat-result").innerHTML =
          `<div class='notice'>Sending encounter request…</div>`;
        json("/v1/pokemon-essentials/wild-encounter?" + params, { method: "POST" })
          .then((v) => {
            $("cheat-result").innerHTML =
              `<div class='notice'>${esc(v.name || "Encounter scheduled")} · level ${esc(v.level)}</div>`;
          })
          .catch((e) => {
            $("cheat-result").innerHTML =
              `<div class='notice error'>${esc(e.message)}</div>`;
          });
      }
      $("search").oninput = renderList;
      $("known").onchange = renderList;
      $("refresh").onclick = () => loadDex(true);
      $("list").onclick = (e) => {
        const row = e.target.closest("[data-id]");
        if (row) selectSpecies(row.dataset.id);
      };
      loadDex(false);
    })();
    </script>
</body>
</html>

rsxp:text html-end */

fn on_activate() {
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + " }, revert: proc { RSXP::PokedexPlugin.uninstall if defined?(RSXP::PokedexPlugin) } }";
    state_set("pending", ruby_inject("essentials-pokedex", apply));
}

fn build_ui() {
    if state_get("patch_handle") == () {
        return [ui_loading("pokedex.installing", "Installing Essentials Pokédex plugin…")];
    }
    [ui_button("pokedex.open", "Open Pokédex (HTML)"), ui_label("pokedex.note", "The explorer and encounter generator run in the plugin webview.")]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "pokedex.open" {
        webview_open("essentials-pokedex", "Essentials Pokédex", pokedex_html());
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
    if handle != () { ruby_unload(handle); }
}

/* rsxp:text ruby-begin
# The Ruby in this block is deliberately plugin-owned. It supplies the
# Pokedex index/detail builders and installs only the generic operation names
# that the core dispatcher already forwards to the active game API module.
module RSXP
  module PokedexPlugin
    def self.install
      @api = RSXP::GameAPI
      @originals ||= {}
      %i[pokemon_essentials_pokedex pokemon_essentials_species_entry pokemon_essentials_wild_encounter start_pending_wild_encounter].each do |name|
        @originals[name] = @api.method(name) if @api.respond_to?(name) && !@originals.key?(name)
        @api.define_singleton_method(name) { |*args| RSXP::PokedexPlugin.public_send(name, *args) }
      end
    end

    def self.uninstall
      return unless @api
      @originals.each do |name, method|
        @api.define_singleton_method(name) { |*args| method.call(*args) }
      end
      @api = nil
      @originals = nil
    end

    def self.pokemon_essentials_pokedex(refresh = false)
      api = @api
      return legacy_pokemon_essentials_pokedex(refresh) if api.legacy_essentials?
      @dex = nil if refresh
      return @dex['index'] if @dex
      records = api.dex_records
      by_id = records.to_h { |record| [api.dex_id(record), record] }
      numbers = {}
      next_number = 0
      records.each do |record|
        next if (api.read(record, :form) || 0).to_i != 0
        next_number += 1
        numbers[(api.read(record, :species) || api.dex_id(record)).to_s] = api.read(record, :id_number) || next_number
      end
      locations = Hash.new { |hash, key| hash[key] = [] }
      infos = api.load_game_data('Data/MapInfos.rxdata') rescue {}
      if defined?(GameData::Encounter) && GameData::Encounter.respond_to?(:each)
        GameData::Encounter.each do |encounter|
          map_id = api.read_alias(encounter, :map, :map_id)
          version = api.read(encounter, :version) || 0
          (api.read(encounter, :types) || {}).each do |type, entries|
            rows = Array(entries).map do |entry|
              weighted = entry.length >= 4 && entry[0].is_a?(Numeric)
              [weighted ? entry[1] : entry[0], weighted ? entry[2] : entry[1], weighted ? entry[3] : entry[2], weighted ? entry[0].to_f : 1.0]
            end
            total = rows.sum { |row| row[3] }
            rows.group_by { |row| row[0] }.each do |species, slots|
              data = api.pokemon_essentials_species(species)
              next unless data
              locations[api.dex_id(data)] << {
                'map_id' => map_id, 'map_name' => api.read(infos[map_id], :name) || "Map #{map_id}",
                'type' => api.read(api.dex_lookup(:EncounterType, type), :name) || type.to_s,
                'version' => version, 'min_level' => slots.map { |row| row[1].to_i }.min,
                'max_level' => slots.map { |row| row[2].to_i }.max,
                'percent' => total.positive? ? (slots.sum { |row| row[3] } * 100.0 / total).round(2) : nil
              }
            end
          end
        end
      end
      obtainable = locations.keys.to_h { |id| [id, 'Wild encounters'] }
      Array(api.read(api.trainer, :party)).each do |pokemon|
        record = api.pokemon_essentials_species(api.read(pokemon, :species))
        obtainable[api.dex_id(record)] ||= 'In your party' if record
      end
      loop do
        previous_count = obtainable.length
        records.each do |record|
          id = api.dex_id(record)
          if obtainable[id]
            api.dex_evolutions(record).each { |edge| obtainable[edge['id']] ||= 'Evolution' if by_id[edge['id']] }
            baby = api.call(record, :get_baby_species)
            baby_record = baby && api.pokemon_essentials_species(baby)
            obtainable[api.dex_id(baby_record)] ||= 'Breeding' if baby_record && Array(api.read(record, :egg_groups)).none? { |group| %w[Undiscovered None].include?(group.to_s) }
          end
        end
        break if obtainable.length == previous_count
      end
      index = records.map do |record|
        id = api.dex_id(record)
        row = { 'id' => id, 'name' => api.read(record, :name) || id,
          'number' => api.read(record, :id_number) || numbers[(api.read(record, :species) || id).to_s],
          'form' => api.read(record, :form) || 0, 'form_name' => api.read(record, :form_name),
          'types' => Array(api.read(record, :types) || [api.read(record, :type1), api.read(record, :type2)]).compact.uniq.map(&:to_s),
          'source' => obtainable[id], 'map_count' => locations[id].map { |row| row['map_id'] }.uniq.length }
        # List rows only need a compact icon. Full battler sheets stay on the
        # detail request so the index response cannot balloon with every
        # species' animation frames.
        row['sprite'] = sprite_data(id, row, :icon)
        row
      end
      @dex = { 'records' => by_id, 'locations' => locations,
        'index' => { 'species' => index, 'note' => 'Known sources include wild encounters, party, evolution and breeding. Scripted gifts, trades, quests and conditional forms may not be indexed. No known source does not mean unobtainable.' } }
      @dex['index']
    end

    def self.pokemon_essentials_species_entry(id)
      api = @api
      return legacy_species_entry(id) if api.legacy_essentials?
      pokemon_essentials_pokedex
      record = @dex['records'][id.to_s]
      raise ArgumentError, "Unknown game species: #{id}" unless record
      summary = @dex['index']['species'].find { |row| row['id'] == id.to_s }
      evolutions = api.dex_evolutions(record)
      previous = @dex['records'].values.flat_map do |other|
        api.dex_evolutions(other).filter_map do |edge|
          next unless edge['id'] == id.to_s
          edge.merge('id' => api.dex_id(other), 'name' => api.read(other, :name) || api.dex_id(other))
        end
      end
      moves = Array(api.read(record, :moves)).map { |level, move| api.dex_move(move, 'Level-up', level) }
      moves.concat(Array(api.read(record, :tutor_moves)).map { |move| api.dex_move(move, 'TM / tutor') })
      moves.concat(Array(api.read(record, :egg_moves)).map { |move| api.dex_move(move, 'Egg') })
      stats = api.read(record, :base_stats)
      stats = %w[HP ATTACK DEFENSE SPEED SPECIAL_ATTACK SPECIAL_DEFENSE].zip(stats).to_h if stats.is_a?(Array)
      facts = {}
      %i[category height weight growth_rate gender_ratio catch_rate happiness base_exp egg_groups hatch_steps habitat color shape generation evs incense offspring flags mega_stone mega_move].each do |key|
        value = api.read(record, key)
        facts[key.to_s] = api.json_value(value) unless value.nil?
      end
      facts['height_m'] = facts.delete('height').to_f / 10.0 if facts['height'].is_a?(Numeric)
      facts['weight_kg'] = facts.delete('weight').to_f / 10.0 if facts['weight'].is_a?(Numeric)
      summary.merge('entry' => api.read_alias(record, :pokedex_entry, :real_pokedex_entry),
        'stats' => api.json_value(stats || {}), 'facts' => facts,
        'abilities' => Array(api.read(record, :abilities)).map { |ability| api.dex_named(:Ability, ability) },
        'hidden_abilities' => Array(api.read_alias(record, :hidden_abilities, :hiddenAbilities)).map { |ability| api.dex_named(:Ability, ability) },
        'locations' => @dex['locations'][id.to_s], 'moves' => moves,
        'evolutions' => evolutions, 'previous' => previous,
        'raw' => api.extension_data(record), 'sprite' => sprite_data(id, summary, :battler),
        'shiny_sprite' => sprite_data(id, summary, :battler, true))
    end

    def self.pokemon_essentials_wild_encounter(args)
      api = @api
      api.require_pokemon_essentials!
      species = args.fetch(0).to_s
      raise ArgumentError, 'a species ID is required' if species.empty?
      level = Integer(api.argument(args, 1, '5'))
      raise ArgumentError, 'level must be between 1 and 100' unless (1..100).cover?(level)
      shiny = parse_optional_bool(args[2], 'shiny')
      form = parse_optional_int(args[3], 'form')
      raise ArgumentError, 'form must be zero or greater' if form && form.negative?
      gender = presence(args[4])
      raise ArgumentError, "gender must be 'male', 'female', or empty" if gender && !%w[male female].include?(gender)
      nature = presence(args[5]); ability = presence(args[6]); item = presence(args[7])
      max_ivs = parse_optional_bool(args[8], 'max_ivs')
      friendship = parse_optional_int(args[9], 'friendship')
      raise ArgumentError, 'friendship must be between 0 and 255' if friendship && !(0..255).cover?(friendship)
      species_id = encounter_species_id(species)
      raise ArgumentError, "unknown nature: #{nature}" if nature && !encounter_nature_valid?(nature)
      raise ArgumentError, "unknown ability: #{ability}" if ability && !encounter_ability_valid?(ability)
      raise ArgumentError, "unknown item: #{item}" if item && !encounter_item_valid?(item)
      require_overworld_for_encounter!
      @pending_encounter = { 'species' => species_id, 'level' => level, 'shiny' => shiny, 'form' => form,
        'gender' => gender, 'nature' => nature, 'ability' => ability, 'item' => item,
        'max_ivs' => max_ivs, 'friendship' => friendship }
      { 'scheduled' => true, 'species' => species, 'level' => level, 'shiny' => shiny == true,
        'name' => encounter_species_name(species_id) || species }
    end

    def self.start_pending_wild_encounter
      spec = @pending_encounter
      return unless spec
      @pending_encounter = nil
      return unless encounter_overworld?
      api.legacy_essentials? ? start_legacy_wild_encounter(spec) : start_game_data_wild_encounter(spec)
    rescue StandardError
      nil
    end

    def self.api
      @api
    end

    def self.encounter_overworld?
      require_overworld_for_encounter!
      true
    rescue StandardError
      false
    end

    def self.require_overworld_for_encounter!
      api = @api
      temp = defined?($game_temp) ? $game_temp : nil
      raise RuntimeError, 'a battle is already in progress' if (temp && api.read(temp, :in_battle)) || !api.pokemon_essentials_active_battle.nil?
      scene = defined?($scene) ? $scene : nil
      overworld = scene && (defined?(Scene_Map) ? scene.is_a?(Scene_Map) : scene.class.name.to_s == 'Scene_Map')
      raise RuntimeError, 'the game is not on the overworld' unless overworld
      raise RuntimeError, 'a menu or dialogue is open' if temp && (api.read(temp, :message_window_showing) || api.read(temp, :in_menu))
    end

    def self.encounter_species_id(species)
      api = @api
      if api.legacy_essentials?
        id = api.legacy_species_id(species.to_s)
        raise ArgumentError, "unknown game species: #{species}" unless id
        id
      else
        raise RuntimeError, 'this game does not expose a GameData::Species registry' unless defined?(GameData::Species)
        begin
          GameData::Species.get(species.to_s).id
        rescue StandardError
          raise ArgumentError, "unknown game species: #{species}"
        end
      end
    end

    def self.encounter_species_name(species_id)
      api.legacy_essentials? ? api.legacy_species_name(species_id) : api.read((GameData::Species.get(species_id) rescue nil), :name)
    end

    def self.encounter_nature_valid?(nature)
      api.legacy_essentials? ? (defined?(PBNatures) && !(PBNatures.const_get(nature.to_s.upcase, false) rescue nil).nil?) : (defined?(GameData::Nature) && GameData::Nature.exists?(nature))
    end

    def self.encounter_ability_valid?(ability)
      api.legacy_essentials? ? (defined?(PBAbilities) && (1..PBAbilities.maxValue).cover?(ability.to_i)) : (defined?(GameData::Ability) && GameData::Ability.exists?(ability))
    end

    def self.encounter_item_valid?(item)
      api.legacy_essentials? ? !api.legacy_item_id(item).nil? : (defined?(GameData::Item) && GameData::Item.exists?(item))
    end

    def self.parse_optional_bool(value, label)
      return nil if value.nil? || value.to_s.empty?
      return true if value.to_s == 'true'
      return false if value.to_s == 'false'
      raise ArgumentError, "#{label} must be true or false"
    end

    def self.parse_optional_int(value, label)
      return nil if value.nil? || value.to_s.empty?
      Integer(value)
    rescue ArgumentError
      raise ArgumentError, "#{label} must be an integer"
    end

    def self.presence(value)
      text = value.to_s
      text.empty? ? nil : text
    end

    def self.start_legacy_wild_encounter(spec)
      events = (defined?(Events) && Events.respond_to?(:onWildPokemonCreate)) ? Events.onWildPokemonCreate : nil
      handler = proc { |_sender, params| apply_legacy_encounter_options(Array(params).first, spec) }
      events + handler if events
      begin
        Object.new.__send__(:pbWildBattle, spec['species'], spec['level'])
      ensure
        events - handler if events
      end
    end

    def self.apply_legacy_encounter_options(pokemon, spec)
      return unless pokemon
      pokemon.shinyflag = spec['shiny'] unless spec['shiny'].nil?
      pokemon.form = spec['form'] unless spec['form'].nil?
      spec['gender'] == 'male' ? (pokemon.makeMale rescue nil) : (pokemon.makeFemale rescue nil) if %w[male female].include?(spec['gender'])
      (pokemon.setNature(spec['nature']) rescue nil) if spec['nature']
      if spec['ability']
        list = (pokemon.getAbilityList rescue nil)
        index = list.is_a?(Array) ? list.index { |entry| entry[0].to_i == spec['ability'].to_i } : nil
        pokemon.setAbility(index || 0)
      end
      (pokemon.setItem(spec['item']) rescue nil) if spec['item']
      if spec['max_ivs']
        pokemon.iv = [31, 31, 31, 31, 31, 31]
        (pokemon.calcStats rescue nil)
      end
      pokemon.happiness = spec['friendship'] unless spec['friendship'].nil?
    end

    def self.start_game_data_wild_encounter(spec)
      record = GameData::Species.get(spec['species'])
      pokemon = Pokemon.new(record.species, spec['level'])
      pokemon.form = spec['form'].nil? ? record.form : spec['form']
      pokemon.shiny = spec['shiny'] unless spec['shiny'].nil?
      spec['gender'] == 'male' ? (pokemon.makeMale rescue nil) : (pokemon.makeFemale rescue nil) if %w[male female].include?(spec['gender'])
      (pokemon.nature = spec['nature'] rescue nil) if spec['nature']
      (pokemon.ability = spec['ability'] rescue nil) if spec['ability']
      (pokemon.item = spec['item'] rescue nil) if spec['item']
      if spec['max_ivs'] && defined?(GameData::Stat)
        GameData::Stat.each_main { |stat| pokemon.iv[stat.id] = 31 }
        (pokemon.calc_stats rescue nil)
      end
      pokemon.happiness = spec['friendship'] unless spec['friendship'].nil?
      defined?(WildBattle) && WildBattle.respond_to?(:start) ? WildBattle.start(pokemon) : Object.new.__send__(:pbWildBattle, pokemon)
    end

    def self.legacy_pokemon_essentials_pokedex(refresh = false)
      api = @api
      @legacy_dex = nil if refresh
      return @legacy_dex['index'] if @legacy_dex
      names = api.legacy_species_names
      locations = api.legacy_encounter_locations
      records = {}
      (1..(defined?(PBSpecies) ? PBSpecies.maxValue : 0)).each do |species|
        name = api.legacy_species_name(species)
        next if name.nil? || name.to_s.strip.empty?
        key = names[species] || "SPECIES_#{species}"
        bytes = api.legacy_dex_bytes(species)
        records[species] = { 'id' => species, 'key' => key, 'name' => name, 'types' => api.legacy_species_types(bytes), 'bytes' => bytes }
      end
      obtainable = {}
      locations.each_key { |species| obtainable[species] = 'Wild encounters' if records[species] }
      api.legacy_party_species_ids.each { |species| obtainable[species] ||= 'In your party' if records[species] }
      @legacy_dex = { 'records' => records.each_with_object({}) { |(species, record), out| out[record['key']] = record }, 'locations' => locations,
        'index' => { 'species' => records.values.map { |record| species = record['id']; row = { 'id' => record['key'], 'name' => record['name'], 'number' => species, 'form' => 0, 'form_name' => nil, 'types' => record['types'], 'source' => obtainable[species], 'map_count' => locations[species].map { |row| row['map_id'] }.uniq.length }; row['sprite'] = sprite_data(record['key'], row, :icon); row }, 'note' => 'Known sources include wild encounters and your party.' } }
      @legacy_dex['index']
    end

    def self.legacy_species_entry(id)
      api = @api
      legacy_pokemon_essentials_pokedex
      record = @legacy_dex['records'][id.to_s] || @legacy_dex['records'].values.find { |row| row['id'].to_s == id.to_s }
      raise ArgumentError, "Unknown game species: #{id}" unless record
      species = record['id']
      bytes = record['bytes'] || api.legacy_dex_bytes(species)
      summary = @legacy_dex['index']['species'].find { |row| row['id'] == record['key'] } || {}
      moves = api.legacy_level_moves(species).filter_map { |level, move| api.legacy_dex_move(move, 'Level-up', level) if move.to_i.positive? }
      moves.concat(api.legacy_machine_moves(species).filter_map { |move| api.legacy_dex_move(move, 'TM / tutor') if move.to_i.positive? })
      moves.concat(api.legacy_egg_moves(species).filter_map { |move| api.legacy_dex_move(move, 'Egg') if move.to_i.positive? })
      summary.merge('entry' => api.legacy_message(:Entries, species), 'stats' => api.legacy_base_stats(bytes), 'facts' => api.legacy_species_facts(species, bytes),
        'abilities' => api.legacy_species_abilities(bytes), 'hidden_abilities' => api.legacy_species_hidden_abilities(bytes), 'locations' => @legacy_dex['locations'][species],
        'moves' => moves, 'evolutions' => api.legacy_evolutions(species), 'previous' => api.legacy_previous_evolutions(species), 'raw' => api.legacy_species_raw(species, bytes),
        'sprite' => sprite_data(id, summary, :battler), 'shiny_sprite' => sprite_data(id, summary, :battler, true))
    end

    # Return a lightweight asset reference. The webview asks the Rust host for
    # the actual image only when a row or detail panel needs it. The important
    # part is that the running game resolves the asset first: different
    # Essentials generations and projects use different directories and names
    # (for example Pokémon Z uses Battlers/001 and Icons/icon001).
    def self.sprite_data(id, summary, kind = :battler, shiny = false)
      api = @api
      number = summary['number'].to_i
      form = summary['form'].to_i
      species_number_or_id = number.positive? ? number : id

      # Modern Essentials owns the complete path convention, including its
      # separate "Front shiny" / "Icons shiny" directories. Use those methods
      # directly when available instead of reconstructing their filenames.
      game_record = api.pokemon_essentials_species(id)
      game_species = api.read(game_record, :species) || id
      resolved = if defined?(GameData::Species) && GameData::Species.respond_to?(:front_sprite_filename)
                   if kind.to_sym == :icon && GameData::Species.respond_to?(:icon_filename)
                     api.call(GameData::Species, :icon_filename, game_species, form, 0, shiny, false, false)
                   else
                     api.call(GameData::Species, :front_sprite_filename, game_species, form, 0, shiny, false)
                   end
                 end
      descriptor = asset_descriptor(resolved)
      return descriptor if descriptor

      # Older Essentials projects often expose the exact helpers used by the
      # game renderer. Prefer them so shiny/form ordering and project-specific
      # fallbacks are identical to the game's own Pokédex and battle scenes.
      resolved = if kind.to_sym == :icon
                   api.legacy_kernel(:pbCheckPokemonIconFiles, [species_number_or_id, false, shiny, form, false], false)
                 else
                   api.legacy_kernel(:pbCheckPokemonBitmapFiles, [species_number_or_id, false, false, shiny, form, false])
                 end
      descriptor = asset_descriptor(resolved)
      return descriptor if descriptor

      raw_values = [id, summary['id'], summary['name'], summary['number']].filter_map do |value|
        value = value.to_s
        next if value.empty? || value.include?('/') || value.include?('\\')
        value
      end
      numeric_values = number.positive? ? [format('%03d', number), format('%04d', number), number.to_s] : []
      form_values = if form.positive?
                      (numeric_values + raw_values).map { |value| "#{value}_#{form}" }
                    else
                      []
                    end
      values = (form_values + numeric_values + raw_values).flat_map do |value|
        [value, value.upcase, value.downcase]
      end.uniq

      # Match the old native Pokédex's search order, but ask pbResolveBitmap
      # which candidate actually exists in this game. This is metadata only;
      # image bytes stay in the host backend and are never read by Ruby.
      roots = kind.to_sym == :icon ? %w[Graphics/Icons Graphics/Pokemon/Icons Graphics/Pokemon] : %w[Graphics/Battlers Graphics/Pokemon/Front Graphics/Pokemon]
      roots += roots.map { |root| "#{root} shiny" } if shiny
      prefixes = kind.to_sym == :icon ? ['', 'icon'] : ['']
      # Some projects use a shiny directory (with no filename suffix), while
      # others use a suffix. Try both conventions when the game has no helper.
      suffixes = shiny ? ['s', '_s', '_shiny', '-shiny', ''] : ['']
      roots.each do |root|
        values.each do |value|
          prefixes.each do |prefix|
            suffixes.each do |suffix|
              requested = "#{root}/#{prefix}#{value}#{suffix}"
              resolved = api.legacy_kernel(:pbResolveBitmap, requested)
              next unless resolved.is_a?(String) && !resolved.empty?
              descriptor = asset_descriptor(resolved)
              return descriptor if descriptor
            end
          end
        end
      end
      nil
    end

    # Convert the game resolver's path back to the safe category/name pair
    # accepted by the local Rust asset bridge. Absolute paths are fine here;
    # only the Graphics-relative portion is exposed to the host.
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
RSXP::PokedexPlugin.install
rsxp:text ruby-end */
