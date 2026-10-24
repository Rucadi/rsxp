# SPDX-License-Identifier: GPL-2.0-or-later
# Game-specific adapter for the Rust-owned TCP/MCP bridge.

module RSXP
  module GameAPI
    SCANCODES = {
      'up' => 82, 'down' => 81, 'left' => 80, 'right' => 79,
      'action' => 40, 'cancel' => 41
    }.freeze

    @keys = {}
    @key_releases = []

    # The generic core router preserves positional argument slots by sending
    # an empty string when an optional query value is absent. Defaults belong
    # to this extension, so normalize those slots here before validation.
    def self.argument(args, index, default)
      value = args[index]
      value.nil? || value.empty? ? default : value
    end

    # Called from Input.update so all game-state reads and input injection stay
    # on the Ruby/game thread.
    def self.poll
      # Start an encounter scheduled by a previous request first, so its API/MCP
      # reply was already sent and a long battle cannot block the caller.
      start_pending_wild_encounter
      RSXP.call('api_poll').force_encoding(Encoding::UTF_8).each_line do |line|
        id, method, *args = line.chomp.split("\t")
        begin
          result = dispatch(method, args)
          RSXP.call('api_respond', id.to_i, payload: JSON.generate(result))
        rescue StandardError => error
          payload = JSON.generate({ 'error' => "#{error.class}: #{error.message}" })
          RSXP.call('api_respond', id.to_i, payload: payload)
        end
      end
    end

    # Merge MCP key presses into the ordinary RGSS input snapshot.
    def self.inject_input(state)
      @key_releases.each { |scan| state[scan] = state.fetch(scan, 0) | 4 }
      @key_releases.clear
      expired = []
      @keys.each do |scan, entry|
        remaining, first = entry
        flags = state.fetch(scan, 0) | 1
        flags |= 2 if first
        state[scan] = flags
        entry[0] = remaining - 1
        entry[1] = false
        if entry[0] <= 0
          @key_releases << scan
          expired << scan
        end
      end
      expired.each { |scan| @keys.delete(scan) }
    end

    def self.dispatch(method, args)
      case method
      when 'game_status'
        game_status
      when 'get_surroundings'
        radius = Integer(args.fetch(0, '5'))
        raise ArgumentError, 'radius must be 1..15' unless (1..15).cover?(radius)
        surroundings(radius)
      when 'world_map'
        raise RuntimeError, 'the Essentials World Map plugin is not active' unless respond_to?(:world_map)
        world_map
      when 'map_preview'
        raise RuntimeError, 'the Essentials World Map plugin is not active' unless respond_to?(:map_preview)
        map_preview(Integer(args.fetch(0)))
      when 'teleport'
        teleport(Integer(args.fetch(0)), Integer(args.fetch(1)), Integer(args.fetch(2)))
      when 'pokemon_essentials_status'
        pokemon_essentials_status
      when 'pokemon_essentials_party'
        pokemon_party
      when 'pokemon_essentials_info'
        pokemon_info(Integer(args.fetch(0)))
      when 'pokemon_essentials_add_item'
        pokemon_essentials_add_item(args.fetch(0), argument(args, 1, '1'))
      when 'pokemon_essentials_movedex'
        raise RuntimeError, 'the Essentials Movedex plugin is not active' unless respond_to?(:pokemon_essentials_movedex)
        pokemon_essentials_movedex(args[0] == 'refresh')
      when 'pokemon_essentials_pokedex'
        pokemon_essentials_pokedex(args[0] == 'refresh')
      when 'pokemon_essentials_species_entry'
        pokemon_essentials_species_entry(args.fetch(0))
      when 'pokemon_essentials_encounters'
        pokemon_essentials_encounters
      when 'pokemon_essentials_npcs'
        pokemon_essentials_npcs
      when 'pokemon_essentials_battle'
        pokemon_essentials_battle
      when 'pokemon_essentials_wild_encounter'
        pokemon_essentials_wild_encounter(args)
      when 'pokemon_essentials_debug'
        pokemon_essentials_debug(args.fetch(0, ''))
      when 'ruby_inject'
        name = args.fetch(0)
        source = decode_hex(args.fetch(1))
        patch_id = RSXP.call('runtime_patch_register', name, payload: source)
        { 'patch_id' => patch_id, 'registered' => true }
      when 'ruby_unload'
        patch_id = args.fetch(0)
        raise ArgumentError, 'unknown Ruby patch handle' unless RuntimePatches.disable(patch_id)
        RSXP.call('runtime_patch_unregister', patch_id)
        { 'patch_id' => patch_id, 'unloaded' => true }
      when 'start_cable_club'
        start_cable_club
      when 'press_key'
        key = args.fetch(0).to_s.downcase
        scan = SCANCODES[key] || raise(ArgumentError, 'key must be up, down, left, right, action, or cancel')
        frames = Integer(args.fetch(1, '1'))
        raise ArgumentError, 'frames must be 1..60' unless (1..60).cover?(frames)
        @keys[scan] = [frames, !@keys.key?(scan)]
        { 'key' => key, 'frames' => frames, 'accepted' => true }
      when 'release_all_keys'
        @key_releases.concat(@keys.keys)
        @keys.clear
        { 'released' => true }
      else
        # Extensions can install additional game-thread methods on this
        # dispatcher without making the generic Rust bridge enumerate their
        # domain. The method name still comes from a declared plugin
        # operation/route, while the plugin owns validation and semantics.
        if respond_to?(method, true)
          public_send(method, args)
        else
          raise ArgumentError, "unknown game API method: #{method}"
        end
      end
    end

    def self.decode_hex(value)
      raise ArgumentError, 'Ruby source encoding is invalid' unless value.length.even? && value.match?(/\A[0-9a-fA-F]*\z/)
      [value].pack('H*').force_encoding(Encoding::UTF_8)
    end

    # Open the game's own Cable Club screen without requiring its map NPC/event.
    # This runs from the normal Ruby frame poll, so all menus and scene changes
    # still execute on the game thread.
    def self.start_cable_club
      player = defined?($player) ? $player : nil
      raise RuntimeError, 'the game player is not initialized yet' unless player
      raise RuntimeError, 'the Cable Club plugin is not installed' unless Object.new.respond_to?(:pbCableClub, true)

      previous = player.respond_to?(:connecting_online?) ? player.connecting_online? : false
      player.connecting_online = true if player.respond_to?(:connecting_online=)
      begin
        Object.new.send(:pbCableClub)
      ensure
        player.connecting_online = previous if player.respond_to?(:connecting_online=)
      end
      { 'started' => true }
    end

    # Queue a wild encounter with the selected species and options. Validation
    # happens now so the caller gets a real error; the battle itself starts on
    # the next game frame so it never blocks the API/MCP reply. Only queued when
    # the game is on the overworld with no battle, menu or dialogue open.
    def self.pokemon_essentials_wild_encounter(args)
      require_pokemon_essentials!
      species = args.fetch(0).to_s
      raise ArgumentError, 'a species ID is required' if species.empty?
      level = Integer(argument(args, 1, '5'))
      raise ArgumentError, 'level must be between 1 and 100' unless (1..100).cover?(level)
      shiny = parse_optional_bool(args[2], 'shiny')
      form = parse_optional_int(args[3], 'form')
      raise ArgumentError, 'form must be zero or greater' if form && form.negative?
      gender = presence(args[4])
      raise ArgumentError, "gender must be 'male', 'female', or empty" if gender && !%w[male female].include?(gender)
      nature = presence(args[5])
      ability = presence(args[6])
      item = presence(args[7])
      max_ivs = parse_optional_bool(args[8], 'max_ivs')
      friendship = parse_optional_int(args[9], 'friendship')
      raise ArgumentError, 'friendship must be between 0 and 255' if friendship && !(0..255).cover?(friendship)

      species_id = encounter_species_id(species)
      raise ArgumentError, "unknown nature: #{nature}" if nature && !encounter_nature_valid?(nature)
      raise ArgumentError, "unknown ability: #{ability}" if ability && !encounter_ability_valid?(ability)
      raise ArgumentError, "unknown item: #{item}" if item && !encounter_item_valid?(item)
      require_overworld_for_encounter!

      @rsxp_pending_encounter = {
        'species' => species_id, 'level' => level, 'shiny' => shiny, 'form' => form,
        'gender' => gender, 'nature' => nature, 'ability' => ability, 'item' => item,
        'max_ivs' => max_ivs, 'friendship' => friendship
      }
      {
        'scheduled' => true, 'species' => species, 'level' => level, 'shiny' => shiny == true,
        'name' => encounter_species_name(species_id) || species
      }
    end

    # Called at the start of each frame poll. The one-frame delay lets the API
    # response reach the caller before a potentially long battle begins.
    def self.start_pending_wild_encounter
      spec = @rsxp_pending_encounter
      return unless spec
      @rsxp_pending_encounter = nil
      return unless encounter_overworld?
      if legacy_essentials?
        start_legacy_wild_encounter(spec)
      else
        start_game_data_wild_encounter(spec)
      end
    rescue StandardError
      nil
    end

    def self.encounter_overworld?
      require_overworld_for_encounter!
      true
    rescue StandardError
      false
    end

    def self.require_overworld_for_encounter!
      temp = defined?($game_temp) ? $game_temp : nil
      if (temp && read(temp, :in_battle)) || !pokemon_essentials_active_battle.nil?
        raise RuntimeError, 'a battle is already in progress'
      end
      scene = defined?($scene) ? $scene : nil
      overworld = scene && (defined?(Scene_Map) ? scene.is_a?(Scene_Map) : scene.class.name.to_s == 'Scene_Map')
      raise RuntimeError, 'the game is not on the overworld' unless overworld
      raise RuntimeError, 'a menu or dialogue is open' if temp && (read(temp, :message_window_showing) || read(temp, :in_menu))
    end

    def self.encounter_species_id(species)
      value = species.to_s
      if legacy_essentials?
        id = legacy_species_id(value)
        raise ArgumentError, "unknown game species: #{species}" unless id
        id
      else
        raise RuntimeError, 'this game does not expose a GameData::Species registry' unless defined?(GameData::Species)
        begin
          GameData::Species.get(value).id
        rescue StandardError
          raise ArgumentError, "unknown game species: #{species}"
        end
      end
    end

    def self.encounter_species_name(species_id)
      if legacy_essentials?
        legacy_species_name(species_id)
      else
        read((GameData::Species.get(species_id) rescue nil), :name)
      end
    end

    def self.encounter_nature_valid?(nature)
      if legacy_essentials?
        return false unless defined?(PBNatures)
        !(PBNatures.const_get(nature.to_s.upcase, false) rescue nil).nil?
      else
        defined?(GameData::Nature) && GameData::Nature.exists?(nature)
      end
    end

    def self.encounter_ability_valid?(ability)
      if legacy_essentials?
        defined?(PBAbilities) && (1..PBAbilities.maxValue).cover?(ability.to_i)
      else
        defined?(GameData::Ability) && GameData::Ability.exists?(ability)
      end
    end

    def self.encounter_item_valid?(item)
      if legacy_essentials?
        !legacy_item_id(item).nil?
      else
        defined?(GameData::Item) && GameData::Item.exists?(item)
      end
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

    # Older Essentials launches battles from a species/level pair, so options are
    # applied to the Pokémon the game generates for this one battle.
    def self.start_legacy_wild_encounter(spec)
      species = spec['species']
      events = (defined?(Events) && Events.respond_to?(:onWildPokemonCreate)) ? Events.onWildPokemonCreate : nil
      handler = proc do |_sender, params|
        apply_legacy_encounter_options(Array(params).first, spec)
      end
      events + handler if events
      begin
        Object.new.__send__(:pbWildBattle, species, spec['level'])
      ensure
        events - handler if events
      end
    end

    def self.apply_legacy_encounter_options(pokemon, spec)
      return unless pokemon
      (pokemon.shinyflag = spec['shiny']) unless spec['shiny'].nil?
      (pokemon.form = spec['form']) unless spec['form'].nil?
      case spec['gender']
      when 'male' then (pokemon.makeMale rescue nil)
      when 'female' then (pokemon.makeFemale rescue nil)
      end
      (pokemon.setNature(spec['nature']) rescue nil) if spec['nature']
      apply_legacy_encounter_ability(pokemon, spec['ability']) if spec['ability']
      (pokemon.setItem(spec['item']) rescue nil) if spec['item']
      if spec['max_ivs']
        pokemon.iv = [31, 31, 31, 31, 31, 31]
        (pokemon.calcStats rescue nil)
      end
      (pokemon.happiness = spec['friendship']) unless spec['friendship'].nil?
    end

    def self.apply_legacy_encounter_ability(pokemon, ability)
      list = (pokemon.getAbilityList rescue nil)
      index = list.is_a?(Array) ? list.index { |entry| entry[0].to_i == ability.to_i } : nil
      pokemon.setAbility(index || 0)
    end

    # Newer Essentials accepts a fully-built Pokémon object in WildBattle.start,
    # so every option is applied before the battle begins.
    def self.start_game_data_wild_encounter(spec)
      record = GameData::Species.get(spec['species'])
      base = record.species
      pokemon = Pokemon.new(base, spec['level'])
      form = spec['form'].nil? ? record.form : spec['form']
      (pokemon.form = form) unless form.nil?
      (pokemon.shiny = spec['shiny']) unless spec['shiny'].nil?
      case spec['gender']
      when 'male' then (pokemon.makeMale rescue nil)
      when 'female' then (pokemon.makeFemale rescue nil)
      end
      (pokemon.nature = spec['nature'] rescue nil) if spec['nature']
      (pokemon.ability = spec['ability'] rescue nil) if spec['ability']
      (pokemon.item = spec['item'] rescue nil) if spec['item']
      if spec['max_ivs'] && defined?(GameData::Stat)
        GameData::Stat.each_main { |stat| pokemon.iv[stat.id] = 31 }
        (pokemon.calc_stats rescue nil)
      end
      (pokemon.happiness = spec['friendship']) unless spec['friendship'].nil?
      if defined?(WildBattle) && WildBattle.respond_to?(:start)
        WildBattle.start(pokemon)
      else
        Object.new.__send__(:pbWildBattle, pokemon)
      end
    end

    def self.game_status
      player = defined?($game_player) ? $game_player : nil
      map = defined?($game_map) ? $game_map : nil
      {
        'ready' => !player.nil? && !map.nil?,
        'scene' => (defined?($scene) && $scene ? $scene.class.name : nil),
        'frame' => (Graphics.frame_count rescue nil),
        'resolution' => { 'width' => Graphics.width, 'height' => Graphics.height },
        'map' => map ? {
          'id' => read(map, :map_id), 'name' => read(map, :name),
          'width' => read(map, :width),
          'height' => read(map, :height), 'display_x' => read(map, :display_x),
          'display_y' => read(map, :display_y)
        } : nil,
        'player' => player ? player_state(player) : nil
      }
    end

    def self.surroundings(radius)
      map = defined?($game_map) ? $game_map : nil
      player = defined?($game_player) ? $game_player : nil
      raise RuntimeError, 'game map/player is not initialized yet' unless map && player
      x0 = Integer(read(player, :x) || 0)
      y0 = Integer(read(player, :y) || 0)
      data = read(map, :data)
      width = Integer(read(map, :width) || 0)
      height = Integer(read(map, :height) || 0)
      x_range = [[x0 - radius, 0].max, [x0 + radius, width - 1].min]
      y_range = [[y0 - radius, 0].max, [y0 + radius, height - 1].min]
      tiles = []
      if data && x_range[0] <= x_range[1] && y_range[0] <= y_range[1]
        (y_range[0]..y_range[1]).each do |y|
          (x_range[0]..x_range[1]).each do |x|
            layers = (0...3).map { |z| data[x, y, z] rescue nil }
            tile = { 'x' => x, 'y' => y, 'layers' => layers }
            tile['terrain_tag'] = call(map, :terrain_tag, x, y) if map.respond_to?(:terrain_tag)
            tiles << tile
          end
        end
      end
      events = read(map, :events)
      nearby_events = events.respond_to?(:values) ? events.values.filter_map do |event|
        ex = read(event, :x)
        ey = read(event, :y)
        next if ex.nil? || ey.nil? || (Integer(ex) - x0).abs > radius || (Integer(ey) - y0).abs > radius
        {
          'id' => read(event, :id), 'name' => read(event, :name),
          'x' => ex, 'y' => ey, 'direction' => read(event, :direction),
          'moving' => read(event, :moving?)
        }
      end : []
      {
        'map_id' => read(map, :map_id), 'map_name' => read(map, :name),
        'width' => width, 'height' => height,
        'player' => player_state(player), 'radius' => radius,
        'tiles' => tiles, 'events' => nearby_events
      }
    end

    def self.load_game_data(path)
      Object.new.__send__(:load_data, path)
    end

    def self.map_data_path(map_id)
      format('Data/Map%03d.rxdata', map_id)
    end

    def self.teleport(map_id, x, y)
      raise ArgumentError, 'map ID must be positive' unless map_id.positive?
      map = load_game_data(map_data_path(map_id))
      width = Integer(read(map, :width) || 0)
      height = Integer(read(map, :height) || 0)
      raise ArgumentError, 'destination tile is outside the map' unless x >= 0 && x < width && y >= 0 && y < height
      helper = Object.new
      if helper.respond_to?(:pbTransferPlayer, true)
        helper.__send__(:pbTransferPlayer, map_id, x, y)
      elsif defined?($game_player) && $game_player.respond_to?(:reserve_transfer)
        $game_player.reserve_transfer(map_id, x, y, 2)
      elsif defined?($game_temp) && $game_temp
        $game_temp.player_new_map_id = map_id
        $game_temp.player_new_x = x
        $game_temp.player_new_y = y
        $game_temp.player_new_direction = 2 if $game_temp.respond_to?(:player_new_direction=)
        $game_temp.player_transferring = true
      else
        raise RuntimeError, 'the active game does not expose player transfer support'
      end
      { 'accepted' => true, 'map_id' => map_id, 'x' => x, 'y' => y }
    end

    def self.player_state(player)
      {
        'x' => read(player, :x), 'y' => read(player, :y),
        'real_x' => read(player, :real_x), 'real_y' => read(player, :real_y),
        'direction' => read(player, :direction), 'moving' => read(player, :moving?)
      }
    end

    # Essentials changed its player and Pokémon class names across releases.
    # Resolve the active trainer using supported public accessors rather than
    # binding the API to one specific Essentials version.
    def self.trainer
      (defined?($Trainer) && $Trainer) || (defined?($player) && $player)
    end

    # Reads or toggles the game's own debug flag. Essentials uses `$DEBUG` for
    # its debug menus, Ctrl-modifiers and editor shortcuts. An empty/query mode
    # only reports the current value; true/false/toggle set it.
    def self.pokemon_essentials_debug(mode)
      require_pokemon_essentials!
      case mode.to_s
      when '', 'query'
        # report only
      when 'true', 'on', 'enable' then $DEBUG = true
      when 'false', 'off', 'disable' then $DEBUG = false
      when 'toggle' then $DEBUG = !(defined?($DEBUG) && $DEBUG)
      else
        raise ArgumentError, 'debug mode must be query, true, false, or toggle'
      end
      { 'debug' => (defined?($DEBUG) && $DEBUG) ? true : false }
    end

    def self.pokemon_essentials_status
      owner = trainer
      essentials = pokemon_essentials_available?
      {
        'available' => !!essentials,
        'engine' => essentials ? 'Pokémon Essentials' : nil,
        'version' => (defined?(Essentials) ? (Essentials::VERSION rescue Essentials::VERSION_NUMBER rescue nil) : nil),
        'trainer' => essentials ? {
          'name' => read(owner, :name),
          'id' => read(owner, :id),
          'money' => read(owner, :money),
          'badges' => (read(owner, :badges)&.length rescue nil)
        } : nil,
        'map_id' => (read(defined?($game_map) ? $game_map : nil, :map_id) rescue nil),
        'in_battle' => !pokemon_essentials_active_battle.nil?,
        'debug' => (defined?($DEBUG) && $DEBUG) ? true : false
      }
    end

    def self.pokemon_essentials_available?
      !trainer.nil? && !!(defined?(Pokemon) || defined?(PokeBattle_Battle) || defined?(GameData::Species) || defined?(PBSpecies))
    end

    def self.require_pokemon_essentials!
      raise RuntimeError, 'active game does not expose Pokémon Essentials data' unless pokemon_essentials_available?
    end

    def self.pokemon_essentials_encounters
      require_pokemon_essentials!
      return legacy_pokemon_essentials_encounters if legacy_essentials?
      raise RuntimeError, 'Pokémon Essentials encounter data is unavailable' unless defined?(GameData::Encounter)
      map_id = Integer(read(defined?($game_map) ? $game_map : nil, :map_id) || 0)
      raise RuntimeError, 'current map is not initialized' unless map_id.positive?
      encounter_version = (read(defined?($PokemonGlobal) ? $PokemonGlobal : nil, :encounter_version) || 0).to_i
      record = nil
      if GameData::Encounter.respond_to?(:get)
        record = GameData::Encounter.get(map_id, encounter_version) rescue GameData::Encounter.get(map_id)
      elsif GameData::Encounter.respond_to?(:try_get)
        record = GameData::Encounter.try_get(map_id, encounter_version) rescue GameData::Encounter.try_get(map_id)
      end
      if !record && encounter_version.positive?
        record = if GameData::Encounter.respond_to?(:get)
                   GameData::Encounter.get(map_id, 0) rescue GameData::Encounter.get(map_id)
                 elsif GameData::Encounter.respond_to?(:try_get)
                   GameData::Encounter.try_get(map_id, 0) rescue GameData::Encounter.try_get(map_id)
                 end
      end
      return { 'available' => false, 'map_id' => map_id, 'encounter_version' => encounter_version, 'tables' => [] } unless record
      types = read(record, :types) || {}
      chances = read_alias(record, :step_chances, :stepChances) || {}
      tables = types.each_with_object([]) do |(type, entries), result|
        rows = Array(entries)
        next if rows.empty?
        decoded_rows = rows.map do |entry|
          values = Array(entry)
          if values.length >= 4 && values[0].is_a?(Numeric)
            # Essentials PBS encounter rows are [weight, species, min, max].
            { weight: values[0].to_f, species: values[1], min_level: values[2], max_level: values[3] }
          else
            { weight: 1.0, min_level: values[1], max_level: values[2], species: values[0] }
          end
        end
        total_weight = decoded_rows.sum { |entry| entry[:weight] }
        type_id = type.to_s
        type_record = if defined?(GameData::EncounterType) && GameData::EncounterType.respond_to?(:try_get)
                        GameData::EncounterType.try_get(type)
                      end
        table_chance = chances[type] || chances[type_id] || chances[type_id.to_sym]
        result << {
          'type' => type_id,
          'name' => read(type_record, :name) || type_id,
          'step_chance' => table_chance,
          'entries' => decoded_rows.each_with_index.map do |entry, index|
            species_id = entry[:species]
            species = pokemon_essentials_species(species_id)
            {
              'slot' => index,
              'species' => json_value(species_id),
              'dex_number' => read(species, :id_number),
              'name' => read(species, :name) || label(species_id),
              'min_level' => entry[:min_level], 'max_level' => entry[:max_level],
              'weight' => entry[:weight],
              'selection_percent' => total_weight.positive? ? (100.0 * entry[:weight] / total_weight).round(3) : nil,
              'step_percent' => table_chance && total_weight.positive? ? (table_chance.to_f * entry[:weight] / total_weight).round(3) : nil
            }
          end
        }
      end
      {
        'available' => true, 'map_id' => map_id, 'encounter_version' => encounter_version,
        'current_encounter_type' => read(defined?($PokemonEncounters) ? $PokemonEncounters : nil, :encounter_type).to_s,
        'probability_note' => 'selection_percent uses stored slot weights, or equal weights for legacy repeated rows. step_percent scales that share by the encounter type step chance; terrain and game modifiers can change actual encounter odds.',
        'tables' => tables
      }
    rescue StandardError => error
      raise RuntimeError, "cannot read current-map encounters: #{error.class}: #{error.message}"
    end

    def self.pokemon_essentials_npcs
      require_pokemon_essentials!
      map = defined?($game_map) ? $game_map : nil
      raise RuntimeError, 'current map is not initialized' unless map
      events = read(map, :events)
      list = events.respond_to?(:values) ? events.values : []
      {
        'map_id' => read(map, :map_id), 'map_name' => read(map, :name),
        'npcs' => list.filter_map { |event| pokemon_essentials_npc(event) }
      }
    end

    # Essentials encounter tables can contain a numeric Pokédex ID even though
    # GameData::Species.try_get only accepts a symbolic/string species ID.
    # Resolve those through the species records' id_number field instead of
    # passing an Integer into try_get (which raises in some Essentials builds).
    def self.pokemon_essentials_species(species_id)
      return unless defined?(GameData::Species)
      if species_id.is_a?(Numeric)
        return unless GameData::Species.respond_to?(:each)
        GameData::Species.each do |record|
          return record if (read(record, :id_number) || -1).to_i == species_id.to_i
        end
        nil
      elsif GameData::Species.respond_to?(:try_get)
        GameData::Species.try_get(species_id)
      elsif GameData::Species.respond_to?(:get)
        GameData::Species.get(species_id)
      end
    rescue StandardError
      nil
    end

    def self.pokemon_essentials_npc(event)
      data = read_alias(event, :event, :data)
      pages = read(data, :pages)
      page_info = Array(pages).map do |page|
        commands = Array(read(page, :list)).first(500).filter_map do |command|
          code = read(command, :code).to_i
          params = read(command, :parameters)
          next unless [101, 102, 103, 104, 108, 111, 117, 121, 201, 205, 301, 355, 401, 402, 403, 404, 408, 655].include?(code)
          { 'code' => code, 'parameters' => json_value(params) }
        end
        text = commands.filter_map do |command|
          code = command['code']
          params = command['parameters']
          params.is_a?(Array) ? params.filter_map { |part| part if part.is_a?(String) }.join("\n") : nil if [101, 401, 355, 655].include?(code)
        end.join("\n")
        {
          'trigger' => read(page, :trigger),
          'graphic' => {
            'character_name' => read(page, :character_name),
            'character_hue' => read(page, :character_hue),
            'direction' => read(page, :direction), 'pattern' => read(page, :pattern)
          },
          'commands' => commands,
          'text_excerpt' => text.empty? ? nil : text[0, 2000],
          'trainer_battle_heuristic' => text.match?(/trainerbattle|pbtrainerbattle|trainer.*battle/i)
        }
      end
      {
        'id' => read(event, :id), 'name' => read(event, :name) || read(data, :name),
        'x' => read(event, :x), 'y' => read(event, :y),
        'direction' => read(event, :direction), 'moving' => read(event, :moving?),
        'erased' => read(event, :erased?), 'pages' => page_info
      }
    rescue StandardError
      nil
    end

    def self.pokemon_essentials_active_battle
      roots = []
      roots << (defined?($scene) ? $scene : nil)
      roots << (defined?($PokemonTemp) ? $PokemonTemp : nil)
      roots << (defined?($game_temp) ? $game_temp : nil)
      queue = roots.compact.map { |root| [root, 0] }
      seen = {}
      until queue.empty?
        object, depth = queue.shift
        next if seen[object.object_id]
        seen[object.object_id] = true
        return object if object.class.name.to_s.match?(/Battle/) &&
          (read_alias(object, :battlers, :battle).is_a?(Array) || object.instance_variable_defined?(:@battlers))
        next if depth >= 2
        if object.respond_to?(:instance_variables)
          object.instance_variables.first(40).each do |name|
            child = object.instance_variable_get(name) rescue nil
            queue << [child, depth + 1] if child && !child.is_a?(Numeric) && !child.is_a?(String) && !child.is_a?(Symbol)
          end
        end
      end
      nil
    end

    def self.pokemon_essentials_battle
      require_pokemon_essentials!
      battle = pokemon_essentials_active_battle
      return { 'active' => false, 'commands' => [], 'battlers' => [] } unless battle
      battlers = read(battle, :battlers) || (battle.instance_variable_get(:@battlers) rescue [])
      can_run = read_alias(battle, :canRun?, :can_run?)
      active = Array(battlers).each_with_index.filter_map do |battler, index|
        next unless battler
        pokemon = read(battler, :pokemon)
        moves = Array(read(battler, :moves) || read(pokemon, :moves)).map do |move|
          move_details(move).merge('usable_by_pp' => read(move, :pp).to_i.positive?)
        end
        own = read_alias(battler, :pbOwnedByPlayer?, :owned_by_player?)
        own = index.even? if own.nil?
        party_options = party.to_a.each_with_index.filter_map do |member, slot|
          next unless member && read(member, :hp).to_i.positive?
          next if member.equal?(pokemon)
          { 'slot' => slot, 'species' => json_value(read(member, :species)), 'name' => read(member, :name), 'level' => read(member, :level) }
        end
        {
          'index' => index, 'side' => own ? 'player' : 'opponent',
          'name' => read(battler, :name) || read(pokemon, :name),
          'pokemon' => pokemon ? pokemon_details(pokemon, index) : nil,
          'hp' => read(battler, :hp) || read(pokemon, :hp),
          'total_hp' => read_alias(battler, :totalhp, :total_hp) || read_alias(pokemon, :totalhp, :total_hp),
          'status' => json_value(read(battler, :status) || read(pokemon, :status)),
          'status_count' => read_alias(battler, :statusCount, :status_count),
          'stages' => json_value(read_alias(battler, :stages, :stages)),
          'moves' => moves,
          'commands' => {
            'fight' => moves.any? { |move| move['pp'].to_i.positive? },
            'switch' => (own && !party_options.empty?),
            'switch_options' => own ? party_options : [],
            'bag' => own,
            'run' => (own && can_run)
          }
        }
      end
      {
        'active' => true, 'class' => battle.class.name,
        'turn' => read_alias(battle, :turnCount, :turn_count, :turn),
        'decision' => read(battle, :decision),
        'can_run' => can_run,
        'capture_odds' => nil,
        'capture_odds_note' => 'Not reported: Essentials catch probability depends on the selected ball and version-specific modifiers; no ball has been selected by this read-only endpoint.',
        'battlers' => active
      }
    end

    def self.party
      owner = trainer
      return nil unless owner
      list = read(owner, :party) || read(owner, :pokemon_party)
      list = call(owner, :pokemon_party) if list.nil?
      list
    end

    def self.pokemon_party
      require_pokemon_essentials!
      list = party
      raise RuntimeError, 'Pokémon Essentials trainer/party is not initialized' unless list
      pokemon = list.each_with_index.map do |pokemon, index|
        pokemon ? pokemon_summary(pokemon, index) : { 'slot' => index, 'empty' => true }
      end
      { 'count' => pokemon.length, 'pokemon' => pokemon }
    end

    def self.pokemon_info(slot)
      require_pokemon_essentials!
      raise ArgumentError, 'party slot must be 0..5' unless (0..5).cover?(slot)
      pokemon = party&.[](slot)
      raise ArgumentError, "no Pokémon in party slot #{slot}" unless pokemon
      pokemon_details(pokemon, slot)
    end

    def self.pokemon_summary(pokemon, slot)
      {
        'slot' => slot,
        'species' => json_value(read(pokemon, :species)),
        'species_name' => read_alias(pokemon, :speciesName, :species_name),
        'name' => read(pokemon, :name), 'level' => read(pokemon, :level),
        'form' => read(pokemon, :form), 'gender' => pokemon_gender(pokemon),
        'shiny' => read_alias(pokemon, :shiny?, :shiny),
        'egg' => read_alias(pokemon, :egg?, :isEgg?),
        'hp' => read(pokemon, :hp), 'total_hp' => read_alias(pokemon, :totalhp, :total_hp),
        'status' => json_value(read(pokemon, :status)),
        'types' => json_value(call(pokemon, :types))
      }
    end

    def self.pokemon_details(pokemon, slot)
      species_data = call(pokemon, :species_data)
      ability = call(pokemon, :ability)
      nature = call(pokemon, :nature)
      item = call(pokemon, :item)
      moves = read(pokemon, :moves)
      owner = read(pokemon, :owner)
      result = pokemon_summary(pokemon, slot).merge(
        'personal_id' => read_alias(pokemon, :personalID, :personal_id),
        'experience' => read_alias(pokemon, :exp, :experience),
        'growth_rate' => json_value(read_alias(pokemon, :growthrate, :growth_rate)),
        'ability' => { 'id' => json_value(read_alias(pokemon, :ability_id, :abilityID) || read(ability, :id) || ability), 'name' => read(ability, :name) || label(ability) },
        'ability_index' => read_alias(pokemon, :ability_index, :abilityIndex),
        'nature' => { 'id' => json_value(read_alias(pokemon, :nature_id, :natureID) || read(nature, :id) || nature), 'name' => read(nature, :name) || label(nature) },
        'held_item' => { 'id' => json_value(read_alias(pokemon, :item_id, :itemID) || read(item, :id) || item), 'name' => read(item, :name) || label(item) },
        'stats' => stat_values(pokemon),
        'base_stats' => json_value(read_alias(species_data, :base_stats, :baseStats) || read_alias(pokemon, :base_stats, :baseStats)),
        'types' => json_value(read_alias(pokemon, :types) || read_alias(species_data, :types)),
        'ivs' => iv_ev_values(pokemon, :iv, 'HP', 'ATTACK', 'DEFENSE', 'SPECIAL_ATTACK', 'SPECIAL_DEFENSE', 'SPEED'),
        'evs' => iv_ev_values(pokemon, :ev, 'HP', 'ATTACK', 'DEFENSE', 'SPECIAL_ATTACK', 'SPECIAL_DEFENSE', 'SPEED'),
        'moves' => Array(moves).map { |move| move_details(move) },
        'status_count' => read_alias(pokemon, :statusCount, :status_count),
        'happiness' => read_alias(pokemon, :happiness, :happiness),
        'pokerus' => read(pokemon, :pokerus),
        'ribbons' => json_value(read(pokemon, :ribbons)),
        'markings' => json_value(read(pokemon, :markings)),
        'owner' => owner_details(pokemon, owner),
        'obtain' => {
          'method' => read_alias(pokemon, :obtain_method, :obtainMethod),
          'map' => read_alias(pokemon, :obtain_map, :obtainMap),
          'level' => read_alias(pokemon, :obtain_level, :obtainLevel),
          'text' => read_alias(pokemon, :obtain_text, :obtainText),
          'hatched_map' => read_alias(pokemon, :hatched_map, :hatchedMap)
        },
        'species_data' => species_details(species_data),
        'extensions' => extension_data(pokemon)
      )
      result
    end

    def self.pokemon_gender(pokemon)
      gender = call(pokemon, :gender)
      return gender unless gender.nil?
      return 'female' if call(pokemon, :female?)
      return 'male' if call(pokemon, :male?)
      nil
    end

    def self.stat_values(pokemon)
      {
        'hp' => read(pokemon, :hp), 'total_hp' => read_alias(pokemon, :totalhp, :total_hp),
        'attack' => read_alias(pokemon, :attack, :atk), 'defense' => read_alias(pokemon, :defense, :def),
        'special_attack' => read_alias(pokemon, :spatk, :special_attack),
        'special_defense' => read_alias(pokemon, :spdef, :special_defense),
        'speed' => read(pokemon, :speed)
      }
    end

    def self.iv_ev_values(pokemon, accessor, *stats)
      value = read_alias(pokemon, accessor, accessor == :iv ? :ivs : :evs)
      unless value.nil?
        if value.is_a?(Array)
          return stats.each_with_index.each_with_object({}) do |(stat, index), values|
            values[stat.downcase] = json_value(value[index]) if index < value.length
          end
        end
        return json_value(value)
      end
      stats.each_with_object({}) do |stat, values|
        entry = read(pokemon, "#{accessor}#{stat}".to_sym)
        values[stat] = entry unless entry.nil?
      end
    end

    def self.move_details(move)
      data = call(move, :move) || call(move, :data)
      {
        'id' => json_value(read(move, :id)), 'name' => read(move, :name) || read(data, :name),
        'pp' => read(move, :pp), 'total_pp' => read_alias(move, :total_pp, :totalpp),
        'pp_up' => read_alias(move, :ppup, :pp_up),
        'power' => read(data, :power) || read(move, :power),
        'type' => json_value(read(data, :type) || read(move, :type)),
        'category' => json_value(read(data, :category) || read(move, :category)),
        'accuracy' => read(data, :accuracy) || read(move, :accuracy),
        'function_code' => read(data, :function_code) || read(move, :function_code)
      }
    end

    def self.owner_details(pokemon, owner)
      return nil unless owner || read_alias(pokemon, :ot, :owner_name) || read_alias(pokemon, :ot_id, :owner_id)
      {
        'name' => read(owner, :name) || read_alias(pokemon, :ot, :owner_name),
        'id' => read(owner, :id) || read_alias(pokemon, :ot_id, :owner_id),
        'public_id' => read_alias(owner, :public_id, :publicID),
        'gender' => read(owner, :gender), 'language' => read(owner, :language)
      }
    end

    def self.species_details(data)
      return nil unless data
      {
        'id' => json_value(read(data, :id)), 'name' => read(data, :name),
        'category' => read_alias(data, :category, :species),
        'height' => read(data, :height), 'weight' => read(data, :weight),
        'abilities' => json_value(read(data, :abilities)),
        'hidden_abilities' => json_value(read_alias(data, :hidden_abilities, :hiddenAbilities))
      }
    end

    def self.extension_data(pokemon)
      return {} unless pokemon.respond_to?(:instance_variables)
      pokemon.instance_variables.each_with_object({}) do |name, values|
        values[name.to_s.delete_prefix('@')] = json_value(pokemon.instance_variable_get(name))
      end
    end

    def self.json_value(value, depth = 0)
      return '[depth limit]' if depth > 5
      case value
      when NilClass, TrueClass, FalseClass, String, Numeric then value
      when Symbol then value.to_s
      when Array then value.first(200).map { |entry| json_value(entry, depth + 1) }
      when Hash
        value.to_a.first(200).each_with_object({}) { |(key, entry), out| out[key.to_s] = json_value(entry, depth + 1) }
      else
        fields = {}
        if value.respond_to?(:instance_variables)
          value.instance_variables.first(100).each do |name|
            fields[name.to_s.delete_prefix('@')] = json_value(value.instance_variable_get(name), depth + 1)
          end
        end
        { 'class' => value.class.name, 'fields' => fields }
      end
    rescue StandardError
      nil
    end

    def self.read_alias(object, *methods)
      methods.each do |method|
        value = read(object, method)
        return value unless value.nil?
      end
      nil
    end

    def self.label(value)
      value.to_s unless value.nil? || value.is_a?(Numeric)
    end

    def self.read(object, method)
      object.public_send(method) if object.respond_to?(method)
    rescue StandardError
      nil
    end

    def self.call(object, method, *args)
      object.public_send(method, *args) if object.respond_to?(method)
    rescue StandardError
      nil
    end
  end
end
# The explorer reads the loaded registries, preserving game modifications and
# translations. Its session cache avoids walking every encounter for each click.
module RSXP::GameAPI
  def self.dex_records
    raise RuntimeError, 'This game does not expose a GameData::Species registry' unless defined?(GameData::Species) && GameData::Species.respond_to?(:each)
    rows = []
    GameData::Species.each { |record| rows << record }
    rows
  end

  def self.dex_id(record)
    (read(record, :id) || read(record, :species) || read(record, :name).to_s.upcase).to_s
  end

  def self.dex_lookup(kind, id)
    return nil unless GameData.const_defined?(kind, false)
    klass = GameData.const_get(kind, false)
    return klass.try_get(id) if klass.respond_to?(:try_get)
    klass.get(id) if klass.respond_to?(:get)
  rescue StandardError
    nil
  end

  def self.dex_named(kind, id)
    data = dex_lookup(kind, id)
    { 'id' => id.to_s, 'name' => read(data, :name) || id.to_s, 'description' => read(data, :description) }
  end

  def self.dex_evolutions(record)
    # The fourth field marks the reverse edge in Essentials' compiled records.
    Array(read(record, :evolutions)).filter_map do |entry|
      next if entry[3] || entry[1].to_s == 'None'
      target = pokemon_essentials_species(entry[0])
      { 'id' => target ? dex_id(target) : entry[0].to_s,
        'name' => read(target, :name) || entry[0].to_s,
        'method' => entry[1].to_s, 'parameter' => json_value(entry[2]),
        'condition' => dex_evolution_condition(entry[1], entry[2]) }
    end
  end

  def self.dex_evolution_condition(method, parameter)
    token = method.to_s
    item = token.include?('Item') ? dex_named(:Item, parameter)['name'] : parameter
    case token
    when 'Level' then "Reach level #{parameter}"
    when 'Item' then "Use #{item}"
    when 'Trade' then 'Trade this Pokémon'
    when 'Happiness' then 'Level up with high friendship'
    when 'HappinessDay' then 'Level up with high friendship during the day'
    when 'HappinessNight' then 'Level up with high friendship at night'
    else
      words = token.gsub(/([a-z])([A-Z])/) { "#{$1} #{$2}" }
      "#{words} · #{item}"
    end
  end

  def self.pokemon_essentials_pokedex(refresh = false)
    return legacy_pokemon_essentials_pokedex(refresh) if legacy_essentials?
    @rsxp_dex = nil if refresh
    return @rsxp_dex['index'] if @rsxp_dex
    records = dex_records
    by_id = records.to_h { |record| [dex_id(record), record] }
    numbers = {}
    next_number = 0
    records.each do |record|
      next if (read(record, :form) || 0).to_i != 0
      next_number += 1
      numbers[(read(record, :species) || dex_id(record)).to_s] = read(record, :id_number) || next_number
    end
    locations = Hash.new { |hash, key| hash[key] = [] }
    infos = load_game_data('Data/MapInfos.rxdata') rescue {}
    if defined?(GameData::Encounter) && GameData::Encounter.respond_to?(:each)
      GameData::Encounter.each do |encounter|
        map_id = read_alias(encounter, :map, :map_id)
        version = read(encounter, :version) || 0
        (read(encounter, :types) || {}).each do |type, entries|
          rows = Array(entries).map do |entry|
            weighted = entry.length >= 4 && entry[0].is_a?(Numeric)
            [weighted ? entry[1] : entry[0], weighted ? entry[2] : entry[1], weighted ? entry[3] : entry[2], weighted ? entry[0].to_f : 1.0]
          end
          total = rows.sum { |row| row[3] }
          rows.group_by { |row| row[0] }.each do |species, slots|
            data = pokemon_essentials_species(species)
            next unless data
            locations[dex_id(data)] << {
              'map_id' => map_id, 'map_name' => read(infos[map_id], :name) || "Map #{map_id}",
              'type' => read(dex_lookup(:EncounterType, type), :name) || type.to_s,
              'version' => version, 'min_level' => slots.map { |row| row[1].to_i }.min,
              'max_level' => slots.map { |row| row[2].to_i }.max,
              'percent' => total.positive? ? (slots.sum { |row| row[3] } * 100.0 / total).round(2) : nil
            }
          end
        end
      end
    end
    obtainable = locations.keys.to_h { |id| [id, 'Wild encounters'] }
    Array(read(trainer, :party)).each do |pokemon|
      record = pokemon_essentials_species(read(pokemon, :species))
      obtainable[dex_id(record)] ||= 'In your party' if record
    end
    # Reachability includes evolutions and breeding relatives. It is evidence of
    # a source, not a claim that the player currently meets its conditions.
    loop do
      previous_count = obtainable.length
      records.each do |record|
        id = dex_id(record)
        if obtainable[id]
          dex_evolutions(record).each { |edge| obtainable[edge['id']] ||= 'Evolution' if by_id[edge['id']] }
          baby = call(record, :get_baby_species)
          baby_record = baby && pokemon_essentials_species(baby)
          obtainable[dex_id(baby_record)] ||= 'Breeding' if baby_record && Array(read(record, :egg_groups)).none? { |group| %w[Undiscovered None].include?(group.to_s) }
        end
      end
      break if obtainable.length == previous_count
    end
    index = records.map do |record|
      id = dex_id(record)
      { 'id' => id, 'name' => read(record, :name) || id,
        'number' => read(record, :id_number) || numbers[(read(record, :species) || id).to_s],
        'form' => read(record, :form) || 0, 'form_name' => read(record, :form_name),
        'types' => Array(read(record, :types) || [read(record, :type1), read(record, :type2)]).compact.uniq.map(&:to_s),
        'source' => obtainable[id], 'map_count' => locations[id].map { |row| row['map_id'] }.uniq.length }
    end
    @rsxp_dex = { 'records' => by_id, 'locations' => locations,
      'index' => { 'species' => index, 'note' => 'Known sources include wild encounters, party, evolution and breeding. Scripted gifts, trades, quests and conditional forms may not be indexed. No known source does not mean unobtainable.' } }
    @rsxp_dex['index']
  end

  def self.dex_move(id, method, level = nil)
    data = dex_lookup(:Move, id)
    { 'id' => id.to_s, 'name' => read(data, :name) || id.to_s, 'method' => method, 'level' => level,
      'type' => read(data, :type).to_s, 'category' => read(data, :category),
      'power' => read_alias(data, :power, :base_damage), 'accuracy' => read(data, :accuracy),
      'pp' => read_alias(data, :total_pp, :pp), 'priority' => read(data, :priority),
      'description' => read(data, :description), 'function_code' => read(data, :function_code) }
  end

  def self.pokemon_essentials_species_entry(id)
    return legacy_species_entry(id) if legacy_essentials?
    pokemon_essentials_pokedex
    record = @rsxp_dex['records'][id.to_s]
    raise ArgumentError, "Unknown game species: #{id}" unless record
    summary = @rsxp_dex['index']['species'].find { |row| row['id'] == id.to_s }
    evolutions = dex_evolutions(record)
    previous = @rsxp_dex['records'].values.flat_map do |other|
      dex_evolutions(other).filter_map do |edge|
        next unless edge['id'] == id.to_s
        edge.merge('id' => dex_id(other), 'name' => read(other, :name) || dex_id(other))
      end
    end
    moves = Array(read(record, :moves)).map { |level, move| dex_move(move, 'Level-up', level) }
    moves.concat(Array(read(record, :tutor_moves)).map { |move| dex_move(move, 'TM / tutor') })
    moves.concat(Array(read(record, :egg_moves)).map { |move| dex_move(move, 'Egg') })
    stats = read(record, :base_stats)
    if stats.is_a?(Array)
      stats = %w[HP ATTACK DEFENSE SPEED SPECIAL_ATTACK SPECIAL_DEFENSE].zip(stats).to_h
    end
    facts = {}
    %i[category height weight growth_rate gender_ratio catch_rate happiness base_exp egg_groups hatch_steps habitat color shape generation evs incense offspring flags mega_stone mega_move].each do |key|
      value = read(record, key)
      facts[key.to_s] = json_value(value) unless value.nil?
    end
    facts['height_m'] = facts.delete('height').to_f / 10.0 if facts['height'].is_a?(Numeric)
    facts['weight_kg'] = facts.delete('weight').to_f / 10.0 if facts['weight'].is_a?(Numeric)
    summary.merge('entry' => read_alias(record, :pokedex_entry, :real_pokedex_entry),
      'stats' => json_value(stats || {}), 'facts' => facts,
      'abilities' => Array(read(record, :abilities)).map { |ability| dex_named(:Ability, ability) },
      'hidden_abilities' => Array(read_alias(record, :hidden_abilities, :hiddenAbilities)).map { |ability| dex_named(:Ability, ability) },
      'locations' => @rsxp_dex['locations'][id.to_s], 'moves' => moves,
      'evolutions' => evolutions, 'previous' => previous,
      'raw' => extension_data(record))
  end
end

module RSXP::GameAPI
  def self.dex_registry(kind)
    unless defined?(GameData) && GameData.const_defined?(kind, false) && GameData.const_get(kind, false).respond_to?(:each)
      raise RuntimeError, "This game does not expose a GameData::#{kind} registry"
    end
    records = []
    GameData.const_get(kind, false).each { |record| records << record }
    records.uniq { |record| dex_id(record) }
  end

end

module RSXP::GameAPI
  # Executed on the game thread. Preflight capacity so standard bag methods that
  # can otherwise add a partial stack do not silently mutate a full inventory.
  def self.pokemon_essentials_add_item(item, requested_quantity)
    quantity = Integer(requested_quantity)
    raise ArgumentError, 'La cantidad debe estar entre 1 y 9999.' unless (1..9999).cover?(quantity)
    if legacy_essentials?
      id = legacy_item_id(item)
      raise ArgumentError, "Objeto desconocido: #{item}" unless id
      item_name = legacy_item_name(id) || legacy_item_key(id)
    else
      record = dex_lookup(:Item, item.to_s)
      raise ArgumentError, "Objeto desconocido: #{item}" unless record
      id = read(record, :id) || item.to_s.to_sym
      item_name = read_alias(record, :display_name, :name) || id.to_s
    end
    bag = (defined?($bag) && $bag) || (defined?($PokemonBag) && $PokemonBag)
    raise RuntimeError, 'La bolsa de la partida todavía no está disponible.' unless bag
    capacity = [:can_add?, :pbCanStore?].find { |method| bag.respond_to?(method) }
    storage = [:add_all, :add, :pbStoreItem].find { |method| bag.respond_to?(method) }
    raise RuntimeError, 'Esta versión de la bolsa no permite añadir objetos.' unless storage
    if capacity
      raise RuntimeError, 'No hay espacio suficiente en la bolsa para esa cantidad.' unless bag.public_send(capacity, id, quantity)
    elsif storage != :add_all
      raise RuntimeError, 'Esta versión de la bolsa no permite comprobar su capacidad.'
    end
    counter = [:quantity, :pbQuantity].find { |method| bag.respond_to?(method) }
    before = counter && bag.public_send(counter, id)
    accepted = bag.public_send(storage, id, quantity)
    after = counter && bag.public_send(counter, id)
    added = before.is_a?(Numeric) && after.is_a?(Numeric) ? after - before : (accepted ? quantity : 0)
    raise RuntimeError, 'La bolsa no ha aceptado el objeto; no se ha añadido ninguna unidad.' unless added.positive?
    { 'item' => id.to_s, 'name' => item_name,
      'requested' => quantity, 'added' => added, 'total' => after,
      'complete' => added == quantity }
  end
end

# Pre-`GameData` Essentials releases (for example Pokémon Z) keep compiled data
# in the generated `PBSpecies`/`PBMoves`/`PBItems` modules and flat binary files
# (`dexdata.dat`, `moves.dat`, `attacksRS.dat`, `evolutions.dat`, `tm.dat`)
# instead of the newer `GameData::*` registries. These helpers read that layout
# so the Pokédex, Movedex and current-map encounters work for both data
# models. Everything runs on the game thread and only reads existing game state.
module RSXP::GameAPI
  LEGACY_MOVE_FLAGS = {
    0x0001 => 'Contact', 0x0002 => 'Protect', 0x0004 => 'Magic Coat',
    0x0008 => 'Snatch', 0x0010 => 'Mirror Move', 0x0020 => "King's Rock",
    0x0040 => 'Thaw User', 0x0080 => 'High Critical', 0x0100 => 'Biting',
    0x0200 => 'Punching', 0x0400 => 'Sound', 0x0800 => 'Powder',
    0x1000 => 'Pulse', 0x2000 => 'Bomb'
  }.freeze
  LEGACY_STAT_NAMES = %w[HP ATTACK DEFENSE SPEED SPECIAL_ATTACK SPECIAL_DEFENSE].freeze
  LEGACY_GROWTH_RATES = %w[Medium Erratic Fluctuating Parabolic Fast Slow].freeze
  LEGACY_HABITATS = %w[Grassland Forest WatersEdge Sea Cave Mountain RoughTerrain Urban Rare].freeze
  LEGACY_POCKET_FALLBACK = { 3 => 'Poké Balls', 4 => 'MTs' }.freeze
  LEGACY_ITEM_TYPES = {
    1 => 'Mail', 2 => 'Mail', 3 => 'Poké Ball', 4 => 'Poké Ball',
    5 => 'Berry', 6 => 'Key item'
  }.freeze

  # Pokémon Z exposes the generated PBS modules but not the v19+ registries.
  def self.legacy_essentials?
    !defined?(GameData::Species) && defined?(PBSpecies) && defined?(PBMoves) && defined?(PBItems)
  end

  # Call a game-defined top-level helper without leaking failures into the API.
  def self.legacy_kernel(method, *args)
    Object.new.__send__(method, *args)
  rescue StandardError
    nil
  end

  # id => constant name for a compiled PBS module such as PBSpecies/PBMoves.
  def self.legacy_constant_names(mod)
    return {} unless mod
    names = {}
    ((mod.constants(false) rescue mod.constants)).each do |name|
      value = (mod.const_get(name, false) rescue nil)
      next unless value.is_a?(Integer)
      names[value] ||= name.to_s
    end
    names
  end

  def self.legacy_species_names
    @rsxp_legacy_species_names ||= legacy_constant_names(defined?(PBSpecies) ? PBSpecies : nil)
  end

  def self.legacy_move_names
    @rsxp_legacy_move_names ||= legacy_constant_names(defined?(PBMoves) ? PBMoves : nil)
  end

  def self.legacy_item_names
    @rsxp_legacy_item_names ||= legacy_constant_names(defined?(PBItems) ? PBItems : nil)
  end

  def self.legacy_type_keys
    @rsxp_legacy_type_keys ||= legacy_constant_names(defined?(PBTypes) ? PBTypes : nil)
  end

  def self.legacy_item_ids
    @rsxp_legacy_item_ids ||= legacy_item_names.each_with_object({}) { |(id, name), out| out[name] = id }
  end

  def self.legacy_species_ids
    @rsxp_legacy_species_ids ||= legacy_species_names.each_with_object({}) { |(id, name), out| out[name] = id }
  end

  def self.legacy_species_id(value)
    text = value.to_s
    return text.to_i if text =~ /\A\d+\z/
    legacy_species_ids[text] || legacy_species_ids[text.upcase]
  end

  def self.legacy_message(kind, id)
    type = defined?(MessageTypes) && MessageTypes.const_defined?(kind, false) ? MessageTypes.const_get(kind, false) : nil
    return nil unless type
    legacy_kernel(:pbGetMessage, type, id)
  end

  def self.legacy_species_key(id)
    legacy_species_names[id.to_i] || "SPECIES_#{id.to_i}"
  end

  def self.legacy_move_key(id)
    legacy_move_names[id.to_i] || "MOVE_#{id.to_i}"
  end

  def self.legacy_item_key(id)
    legacy_item_names[id.to_i] || "ITEM_#{id.to_i}"
  end

  def self.legacy_species_name(id)
    legacy_message(:Species, id.to_i)
  end

  def self.legacy_move_name(id)
    legacy_message(:Moves, id.to_i)
  end

  def self.legacy_item_name(id)
    legacy_message(:Items, id.to_i)
  end

  def self.legacy_type_name(id)
    legacy_message(:Types, id.to_i)
  end

  # Encoded type ID => this game's internal constant name (NORMAL, FIRE, ...).
  def self.legacy_type_key(id)
    return nil if id.nil?
    legacy_type_keys[id.to_i] || id.to_s
  end

  # The 76-byte species record starts at 76*(species-1) in dexdata.dat.
  def self.legacy_dex_bytes(species)
    return nil unless defined?(PBSpecies)
    species = species.to_i
    return nil unless species.between?(1, PBSpecies.maxValue)
    dexdata = legacy_kernel(:pbOpenDexData)
    return nil unless dexdata
    begin
      legacy_kernel(:pbDexDataOffset, dexdata, species, 0)
      Array.new(76) { dexdata.fgetb }
    ensure
      (dexdata.close rescue nil)
    end
  end

  def self.legacy_word(bytes, offset)
    return 0 unless bytes
    (bytes[offset] || 0) | ((bytes[offset + 1] || 0) << 8)
  end

  def self.legacy_base_stats(bytes)
    LEGACY_STAT_NAMES.each_with_index.to_h { |name, index| [name, bytes ? (bytes[10 + index] || 0) : 0] }
  end

  def self.legacy_ev_yield(bytes)
    LEGACY_STAT_NAMES.each_with_index.to_h { |name, index| [name, bytes ? (bytes[23 + index] || 0) : 0] }
  end

  def self.legacy_species_types(bytes)
    return [] unless bytes
    [bytes[8], bytes[9]].compact.map { |type| legacy_type_key(type) }.compact.uniq
  end

  def self.legacy_egg_groups(bytes)
    return [] unless bytes
    [bytes[31], bytes[32]].compact.map { |group| (PBEggGroups.getName(group) rescue nil) }.compact.reject { |name| name.to_s.empty? }.uniq
  end

  def self.legacy_wild_items(bytes)
    return [] unless bytes
    [legacy_word(bytes, 48), legacy_word(bytes, 50), legacy_word(bytes, 52)].filter_map do |item|
      next if item.to_i <= 0
      legacy_item_name(item) || legacy_item_key(item)
    end
  end

  def self.legacy_species_facts(species, bytes)
    return {} unless bytes
    height = legacy_word(bytes, 33)
    weight = legacy_word(bytes, 35)
    facts = {
      'category' => legacy_message(:Kinds, species),
      'height_m' => (height / 10.0).round(2),
      'weight_kg' => (weight / 10.0).round(2),
      'catch_rate' => bytes[16],
      'gender_ratio' => bytes[18],
      'happiness' => bytes[19],
      'growth_rate' => LEGACY_GROWTH_RATES[bytes[20].to_i] || bytes[20],
      'base_exp' => legacy_word(bytes, 38),
      'hatch_steps' => legacy_word(bytes, 21),
      'color' => (PBColors.getName(bytes[6]) rescue nil),
      'habitat' => (bytes[7].to_i.positive? ? LEGACY_HABITATS[bytes[7].to_i - 1] : nil),
      'egg_groups' => legacy_egg_groups(bytes),
      'evs' => legacy_ev_yield(bytes),
      'wild_held_items' => legacy_wild_items(bytes),
      'incense' => (legacy_item_name(legacy_word(bytes, 54)) if legacy_word(bytes, 54).to_i.positive?)
    }
    facts.delete_if { |_key, value| value.nil? || (value.respond_to?(:empty?) && value.empty?) }
    facts
  end

  def self.legacy_species_raw(species, bytes)
    { 'internal_name' => legacy_species_key(species), 'number' => species.to_i, 'dex_bytes' => bytes }
  end

  # Ability pairs live at bytes 2/4; the four hidden slots at 40..47.
  def self.legacy_ability(id)
    return nil if id.nil? || id.to_i <= 0
    { 'id' => json_value(id), 'name' => legacy_message(:Abilities, id.to_i) || label(id),
      'description' => legacy_message(:AbilityDescs, id.to_i) }
  end

  def self.legacy_species_abilities(bytes)
    return [] unless bytes
    [legacy_word(bytes, 2), legacy_word(bytes, 4)].filter_map { |ability| legacy_ability(ability) }
  end

  def self.legacy_species_hidden_abilities(bytes)
    return [] unless bytes
    [legacy_word(bytes, 40), legacy_word(bytes, 42), legacy_word(bytes, 44), legacy_word(bytes, 46)].filter_map { |ability| legacy_ability(ability) }
  end

  # evolutions.dat stores five bytes per edge: a type/data byte, a parameter word
  # and the destination species word. Only forward edges are exposed here.
  def self.legacy_evolutions_by_species
    return @rsxp_legacy_evolutions if @rsxp_legacy_evolutions
    result = {}
    return @rsxp_legacy_evolutions = result unless defined?(PBSpecies)
    file = legacy_kernel(:pbRgssOpen, 'Data/evolutions.dat', 'rb')
    return @rsxp_legacy_evolutions = result unless file
    begin
      (1..PBSpecies.maxValue).each do |species|
        file.pos = (species - 1) * 8
        offset = file.fgetdw
        length = file.fgetdw
        next if length.to_i <= 0
        file.pos = offset
        rows = []
        index = 0
        while index < length
          evo = file.fgetb
          method = evo & 0x3F
          parameter = file.fgetw
          target = file.fgetw
          index += 5
          next unless (evo & 0xC0) == 0x00
          rows << [method, parameter, target]
        end
        result[species] = rows unless rows.empty?
      end
    ensure
      (file.close rescue nil)
    end
    @rsxp_legacy_evolutions = result
  end

  def self.legacy_evolution_param_type(method)
    return 0 unless defined?(PBEvolution) && PBEvolution.const_defined?(:EVOPARAM, false)
    PBEvolution::EVOPARAM[method].to_i
  end

  def self.legacy_evolution_method(method)
    return 'Unknown' unless defined?(PBEvolution) && PBEvolution.const_defined?(:EVONAMES, false)
    PBEvolution::EVONAMES[method] || 'Unknown'
  end

  def self.legacy_evolution_parameter(method, parameter)
    case legacy_evolution_param_type(method)
    when 2 then legacy_item_name(parameter) || parameter
    when 3 then legacy_move_name(parameter) || parameter
    when 4 then legacy_species_name(parameter) || parameter
    when 5 then legacy_type_name(parameter) || parameter
    else parameter
    end
  end

  def self.legacy_evolution_condition(method, parameter)
    token = legacy_evolution_method(method)
    value = legacy_evolution_parameter(method, parameter)
    case token
    when 'Level' then "Reach level #{value}"
    when 'Happiness' then 'Level up with high friendship'
    when 'HappinessDay' then 'Level up with high friendship during the day'
    when 'HappinessNight' then 'Level up with high friendship at night'
    when 'Trade' then 'Trade this Pokémon'
    when 'TradeItem' then "Trade holding #{value}"
    when 'Item' then "Use #{value}"
    else
      words = token.gsub(/([a-z])([A-Z])/) { "#{$1} #{$2}" }
      legacy_evolution_param_type(method).zero? ? words : "#{words} · #{value}"
    end
  end

  def self.legacy_evolutions(species)
    rows = legacy_evolutions_by_species[species.to_i] || []
    rows.filter_map do |method, parameter, target|
      target = target.to_i
      next if target <= 0
      { 'id' => legacy_species_key(target), 'name' => legacy_species_name(target) || legacy_species_key(target),
        'method' => legacy_evolution_method(method), 'parameter' => legacy_evolution_parameter(method, parameter),
        'condition' => legacy_evolution_condition(method, parameter) }
    end
  end

  def self.legacy_previous_evolutions(species)
    species = species.to_i
    legacy_evolutions_by_species.each_with_object([]) do |(source, _rows), out|
      next if source == species
      legacy_evolutions(source).each do |edge|
        next unless edge['id'] == legacy_species_key(species)
        out << edge.merge('id' => legacy_species_key(source), 'name' => legacy_species_name(source) || legacy_species_key(source))
      end
    end
  end

  # Level-up moves are pairs of words in attacksRS.dat (with an offset table).
  def self.legacy_level_moves(species)
    file = legacy_kernel(:pbRgssOpen, 'Data/attacksRS.dat', 'rb')
    return [] unless file
    begin
      index = species.to_i - 1
      offset = file.getOffset(index)
      length = file.getLength(index) >> 1
      file.pos = offset
      Array.new(length) { [file.fgetw, file.fgetw] }
    ensure
      (file.close rescue nil)
    end
  end

  def self.legacy_egg_moves(species)
    file = legacy_kernel(:pbRgssOpen, 'Data/eggEmerald.dat', 'rb')
    return [] unless file
    begin
      file.pos = (species.to_i - 1) * 8
      offset = file.fgetdw
      length = file.fgetdw
      return [] if length.to_i <= 0
      file.pos = offset
      Array.new(length / 2) { file.fgetw }
    ensure
      (file.close rescue nil)
    end
  end

  # tm.dat is a move-indexed list of compatible species. It may be an Array
  # (older layout) or a Hash (projects that rewrote the compatibility table).
  def self.legacy_tm_data
    @rsxp_legacy_tm ||= (load_game_data('Data/tm.dat') rescue nil)
  end

  def self.legacy_machine_moves(species)
    species = species.to_i
    data = legacy_tm_data
    moves = []
    if data.is_a?(Array)
      (1...data.length).each { |move| moves << move if Array(data[move]).include?(species) }
    elsif data.is_a?(Hash)
      data.each { |move, list| moves << move.to_i if Array(list).include?(species) }
    end
    moves
  end

  def self.legacy_target_name(target)
    if defined?(PBTargets)
      name = (PBTargets.constants(false) rescue PBTargets.constants).find do |candidate|
        (PBTargets.const_get(candidate, false) rescue nil) == target
      end
      return name.to_s.gsub(/([a-z])([A-Z])/) { "#{$1} #{$2}" } if name
    end
    target.to_s
  end

  def self.legacy_move_flags(bits)
    bits = bits.to_i
    LEGACY_MOVE_FLAGS.select { |mask, _name| (bits & mask) != 0 }.values
  end

  def self.legacy_move_details(id)
    return nil unless defined?(PBMoveData)
    data = (PBMoveData.new(id.to_i) rescue nil)
    return nil unless data
    {
      'id' => legacy_move_key(id), 'name' => legacy_move_name(id) || legacy_move_key(id),
      'type' => legacy_type_key(data.type), 'type_name' => legacy_type_name(data.type),
      'category' => data.category, 'power' => data.basedamage, 'accuracy' => data.accuracy,
      'pp' => data.totalpp, 'priority' => data.priority,
      'description' => legacy_message(:MoveDescriptions, id.to_i),
      'function_code' => data.function, 'effect_chance' => data.addlEffect,
      'target' => legacy_target_name(data.target), 'flags' => legacy_move_flags(data.flags)
    }
  end

  def self.legacy_dex_move(id, method, level = nil)
    if (details = legacy_move_details(id))
      details.merge('method' => method, 'level' => level)
    else
      { 'id' => legacy_move_key(id), 'name' => legacy_move_name(id) || legacy_move_key(id), 'method' => method, 'level' => level }
    end
  end

  def self.legacy_move_raw(id, details)
    { 'internal_name' => legacy_move_key(id), 'function_code' => details['function_code'],
      'effect_chance' => details['effect_chance'], 'target' => details['target'] }
  end

  # Item records come from items.dat (or the live bag cache). Field layout:
  # id, name, plural, pocket, price, description, field use, battle use, type, machine.
  def self.legacy_item_records
    return @rsxp_legacy_items if @rsxp_legacy_items
    list = (defined?($ItemData) && $ItemData) || legacy_kernel(:readItemList, 'Data/items.dat')
    records = {}
    return @rsxp_legacy_items = records unless list
    max = defined?(PBItems) ? PBItems.maxValue : 0
    (1..max).each do |id|
      record = (list[id] rescue nil)
      next unless record
      next unless (record[0] || 0).to_i == id
      records[id] = record
    end
    @rsxp_legacy_items = records
  end

  def self.legacy_item_id(item)
    value = item.to_s
    return value.to_i if value =~ /\A\d+\z/
    legacy_item_ids[value] || legacy_item_ids[value.upcase]
  end

  def self.legacy_pocket_name(pocket)
    return 'Not supplied' if pocket.nil?
    name = (legacy_kernel(:pbPocketNames) || [])[pocket.to_i]
    return name if name && !name.to_s.empty?
    LEGACY_POCKET_FALLBACK[pocket.to_i] || "Pocket #{pocket}"
  end

  def self.legacy_item_use_label(use)
    case use.to_i
    when 0 then 'Not usable'
    when 1 then 'Used on a Pokémon (consumed)'
    when 2 then 'Used from the bag'
    when 3 then 'TM'
    when 4 then 'HM'
    when 5 then 'Used on a Pokémon'
    else "Mode #{use}"
    end
  end

  def self.legacy_item_battle_use_label(use)
    case use.to_i
    when 0 then 'Not usable'
    when 1 then 'Used on a Pokémon'
    when 2 then 'Used in battle'
    when 3 then 'Used on a Pokémon (ball)'
    when 4 then 'Used in battle (ball)'
    else "Mode #{use}"
    end
  end

  def self.legacy_predicate(method, id)
    legacy_kernel(method, id) ? true : false
  end

  def self.legacy_item_tags(record, id)
    use = record[6].to_i
    type = record[8].to_i
    tags = []
    tags << LEGACY_ITEM_TYPES[type] if LEGACY_ITEM_TYPES[type]
    tags << 'TM / HM / TR' if use == 3 || use == 4
    tags << 'Medicine' if type != 5 && type != 3 && type != 4 && use == 1
    tags << 'Evolution item' if legacy_predicate(:pbIsEvolutionStone?, id)
    tags << 'Mega Stone' if legacy_predicate(:pbIsMegaStone?, id)
    tags << 'Can be held' if type != 6 && use != 3 && use != 4
    tags.uniq
  end

  def self.legacy_item_icon(id)
    named = "Graphics/Icons/item#{legacy_item_key(id)}"
    return named if legacy_kernel(:pbResolveBitmap, named)
    format('Graphics/Icons/item%03d', id.to_i)
  end

  def self.legacy_item_raw(record, id)
    { 'internal_name' => legacy_item_key(id), 'name' => legacy_item_name(id), 'plural' => record[2],
      'pocket' => record[3], 'price' => record[4], 'description' => record[5],
      'field_use' => record[6], 'battle_use' => record[7], 'type' => record[8], 'machine_move' => record[9] }
  end

  def self.legacy_move_machines
    machines = Hash.new { |hash, key| hash[key] = [] }
    legacy_item_records.each do |id, record|
      next unless record[6].to_i == 3 # only real TMs teach moves in the add-TM action
      move = record[9].to_i
      next if move <= 0
      machines[move] << { 'id' => legacy_item_key(id), 'name' => legacy_item_name(id) || legacy_item_key(id) }
    end
    machines
  end

  def self.legacy_encounter_type_name(type)
    return nil unless defined?(EncounterTypes) && EncounterTypes.const_defined?(:Names, false)
    EncounterTypes::Names[type]
  end

  def self.legacy_encounter_chances(type)
    return [] unless defined?(EncounterTypes) && EncounterTypes.const_defined?(:EnctypeChances, false)
    EncounterTypes::EnctypeChances[type] || []
  end

  # Data/encounters.dat maps a map id to [density per type, slot rows per type].
  # Each slot row is [species, min level, max level]; the slot weight comes from
  # the matching EncounterTypes::EnctypeChances table.
  def self.legacy_encounter_locations
    return @rsxp_legacy_locations if @rsxp_legacy_locations
    locations = Hash.new { |hash, key| hash[key] = [] }
    data = (load_game_data('Data/encounters.dat') rescue nil)
    infos = (load_game_data('Data/MapInfos.rxdata') rescue nil)
    infos = {} unless infos.is_a?(Hash)
    return @rsxp_legacy_locations = locations unless data.is_a?(Hash)
    data.each do |map_id, entry|
      next unless entry.is_a?(Array) && entry[1].is_a?(Array)
      entry[1].each_with_index do |rows, type|
        next unless rows.is_a?(Array) && !rows.empty?
        chances = legacy_encounter_chances(type)
        decoded = rows.each_with_index.map do |row, index|
          [row[0].to_i, row[1], row[2], (chances[index] || 1)]
        end
        total = decoded.sum { |row| row[3] }
        decoded.group_by { |row| row[0] }.each do |species, slots|
          next if species <= 0
          locations[species] << {
            'map_id' => map_id, 'map_name' => read(infos[map_id], :name) || "Map #{map_id}",
            'type' => legacy_encounter_type_name(type) || "Type #{type}", 'version' => 0,
            'min_level' => slots.map { |row| row[1].to_i }.min,
            'max_level' => slots.map { |row| row[2].to_i }.max,
            'percent' => total.positive? ? (slots.sum { |row| row[3] } * 100.0 / total).round(2) : nil
          }
        end
      end
    end
    @rsxp_legacy_locations = locations
  end

  def self.legacy_current_encounter_type
    encounters = defined?($PokemonEncounters) ? $PokemonEncounters : nil
    return nil unless encounters && encounters.respond_to?(:pbEncounterType)
    type = encounters.pbEncounterType
    type && type.to_i >= 0 ? legacy_encounter_type_name(type) : nil
  rescue StandardError
    nil
  end

  def self.legacy_pokemon_essentials_encounters
    map = defined?($game_map) ? $game_map : nil
    map_id = Integer(read(map, :map_id) || 0)
    raise RuntimeError, 'current map is not initialized' unless map_id.positive?
    data = (load_game_data('Data/encounters.dat') rescue nil)
    entry = data.is_a?(Hash) ? data[map_id] : nil
    unless entry.is_a?(Array) && entry[1].is_a?(Array)
      return { 'available' => false, 'map_id' => map_id, 'encounter_version' => 0, 'tables' => [] }
    end
    density = entry[0]
    tables = []
    entry[1].each_with_index do |rows, type|
      next unless rows.is_a?(Array) && !rows.empty?
      chances = legacy_encounter_chances(type)
      decoded = rows.each_with_index.map do |row, index|
        { species: row[0].to_i, min_level: row[1], max_level: row[2], weight: (chances[index] || 1) }
      end
      total = decoded.sum { |row| row[:weight] }
      step_chance = density.is_a?(Array) && density[type] ? (density[type].to_f * 100.0 / 180.0).round(3) : nil
      name = legacy_encounter_type_name(type) || "Type #{type}"
      tables << {
        'type' => name, 'name' => name, 'step_chance' => step_chance,
        'entries' => decoded.each_with_index.map do |row, index|
          species = row[:species]
          { 'slot' => index, 'species' => legacy_species_key(species), 'dex_number' => species,
            'name' => legacy_species_name(species) || legacy_species_key(species),
            'min_level' => row[:min_level], 'max_level' => row[:max_level], 'weight' => row[:weight],
            'selection_percent' => total.positive? ? (row[:weight] * 100.0 / total).round(3) : nil,
            'step_percent' => step_chance && total.positive? ? (step_chance * row[:weight] / total).round(3) : nil }
        end
      }
    end
    {
      'available' => true, 'map_id' => map_id, 'encounter_version' => 0,
      'current_encounter_type' => legacy_current_encounter_type.to_s,
      'probability_note' => 'Slot shares use the compiled slot weights. Per-step values scale by this map’s encounter density; terrain, abilities and game modifiers change the actual odds.',
      'tables' => tables
    }
  end

  def self.legacy_party_species_ids
    owner = trainer
    list = owner && (read(owner, :party) || read(owner, :pokemon_party))
    Array(list).filter_map { |pokemon| read(pokemon, :species).to_i if pokemon }
  rescue StandardError
    []
  end

  def self.legacy_pokemon_essentials_pokedex(refresh = false)
    @rsxp_legacy_dex = nil if refresh
    return @rsxp_legacy_dex['index'] if @rsxp_legacy_dex
    names = legacy_species_names
    locations = legacy_encounter_locations
    records = {}
    (1..(defined?(PBSpecies) ? PBSpecies.maxValue : 0)).each do |species|
      name = legacy_species_name(species)
      next if name.nil? || name.to_s.strip.empty?
      key = names[species] || "SPECIES_#{species}"
      bytes = legacy_dex_bytes(species)
      records[species] = { 'id' => species, 'key' => key, 'name' => name,
                           'types' => legacy_species_types(bytes), 'bytes' => bytes }
    end
    obtainable = {}
    locations.each_key { |species| obtainable[species] = 'Wild encounters' if records[species] }
    legacy_party_species_ids.each { |species| obtainable[species] ||= 'In your party' if records[species] }
    # Reachability includes evolutions and breeding relatives, exactly like the
    # GameData path: evidence of a source, not a claim the player can reach it now.
    loop do
      previous_count = obtainable.length
      legacy_evolutions_by_species.each do |species, rows|
        next unless obtainable[species]
        rows.each do |_method, _parameter, target|
          obtainable[target.to_i] ||= 'Evolution' if records[target.to_i]
        end
      end
      obtainable.keys.each do |species|
        bytes = records[species] && records[species]['bytes']
        next unless bytes
        next if [bytes[31].to_i, bytes[32].to_i].include?(15) || (bytes[31].to_i + bytes[32].to_i).zero?
        baby = legacy_kernel(:pbGetBabySpecies, species).to_i
        obtainable[baby] ||= 'Breeding' if baby.positive? && records[baby]
      end
      break if obtainable.length == previous_count
    end
    index = records.values.map do |record|
      species = record['id']
      { 'id' => record['key'], 'name' => record['name'], 'number' => species, 'form' => 0,
        'form_name' => nil, 'types' => record['types'],
        'source' => obtainable[species], 'map_count' => locations[species].map { |row| row['map_id'] }.uniq.length }
    end
    @rsxp_legacy_dex = {
      'records' => records.each_with_object({}) { |(species, record), out| out[record['key']] = record },
      'locations' => locations,
      'index' => { 'species' => index, 'note' => 'Known sources include wild encounters, party, evolution and breeding. Scripted gifts, trades, quests and conditional forms may not be indexed. No known source does not mean unobtainable.' }
    }
    @rsxp_legacy_dex['index']
  end

  def self.legacy_species_entry(id)
    legacy_pokemon_essentials_pokedex
    record = @rsxp_legacy_dex['records'][id.to_s] ||
             @rsxp_legacy_dex['records'].values.find { |row| row['id'].to_s == id.to_s }
    raise ArgumentError, "Unknown game species: #{id}" unless record
    species = record['id']
    bytes = record['bytes'] || legacy_dex_bytes(species)
    summary = @rsxp_legacy_dex['index']['species'].find { |row| row['id'] == record['key'] } || {}
    moves = legacy_level_moves(species).filter_map do |level, move|
      legacy_dex_move(move, 'Level-up', level) if move.to_i.positive?
    end
    moves.concat(legacy_machine_moves(species).filter_map { |move| legacy_dex_move(move, 'TM / tutor') if move.to_i.positive? })
    moves.concat(legacy_egg_moves(species).filter_map { |move| legacy_dex_move(move, 'Egg') if move.to_i.positive? })
    summary.merge(
      'entry' => legacy_message(:Entries, species),
      'stats' => legacy_base_stats(bytes),
      'facts' => legacy_species_facts(species, bytes),
      'abilities' => legacy_species_abilities(bytes),
      'hidden_abilities' => legacy_species_hidden_abilities(bytes),
      'locations' => @rsxp_legacy_dex['locations'][species],
      'moves' => moves,
      'evolutions' => legacy_evolutions(species),
      'previous' => legacy_previous_evolutions(species),
      'raw' => legacy_species_raw(species, bytes)
    )
  end

end
