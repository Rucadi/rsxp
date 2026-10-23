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
        world_map
      when 'map_preview'
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
        pokemon_essentials_add_item(args.fetch(0), args.fetch(1, '1'))
      when 'pokemon_essentials_movedex'
        pokemon_essentials_movedex(args[0] == 'refresh')
      when 'pokemon_essentials_itemdex'
        pokemon_essentials_itemdex(args[0] == 'refresh')
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
        raise ArgumentError, "unknown game API method: #{method}"
      end
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
      level = Integer(args.fetch(1, '5'))
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

    # Describe every RPG Maker map and the Essentials connection offsets. This
    # is the source for the navigable world canvas, not the town-map/fly points.
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

    # Transfer events are the Essentials-authored links between outdoor maps
    # when the project has no explicit map-connections table. Their source and
    # destination tile coordinates let the world canvas align adjoining maps.
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

      # Read the compiled Essentials metadata at runtime when available. PBS is
      # retained as a fallback for projects that don't expose GameData records.
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

    def self.load_game_data(path)
      Object.new.__send__(:load_data, path)
    end

    def self.map_data_path(map_id)
      format('Data/Map%03d.rxdata', map_id)
    end

    # Return the real RGSS tile layers and their source bitmap names. Rust
    # renders these with the same tileset/autotile cell rules as Tilemap.
    def self.map_preview(map_id)
      raise ArgumentError, 'map ID must be positive' unless map_id.positive?
      map = load_game_data(map_data_path(map_id))
      tilesets = defined?($data_tilesets) && $data_tilesets ? $data_tilesets : load_game_data('Data/Tilesets.rxdata')
      tileset = tilesets[Integer(read(map, :tileset_id) || 0)]
      raise RuntimeError, 'map tileset is unavailable' unless tileset
      width = Integer(read(map, :width) || 0)
      height = Integer(read(map, :height) || 0)
      raise RuntimeError, 'map dimensions are invalid' unless width.positive? && height.positive?
      layers = (0...3).map do |z|
        Array.new(height) do |y|
          Array.new(width) { |x| Integer(map.data[x, y, z] || 0) }
        end
      end
      {
        'map_id' => map_id, 'width' => width, 'height' => height,
        'tileset' => read(tileset, :tileset_name).to_s,
        'autotiles' => Array(read(tileset, :autotile_names)).map(&:to_s),
        'autotile_parts' => (Tilemap::AUTOTILE_PARTS rescue []),
        'layers' => layers
      }
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
