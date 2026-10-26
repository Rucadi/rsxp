module FANPLAY::GameAPI
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
    @fanplay_legacy_species_names ||= legacy_constant_names(defined?(PBSpecies) ? PBSpecies : nil)
  end

  def self.legacy_move_names
    @fanplay_legacy_move_names ||= legacy_constant_names(defined?(PBMoves) ? PBMoves : nil)
  end

  def self.legacy_item_names
    @fanplay_legacy_item_names ||= legacy_constant_names(defined?(PBItems) ? PBItems : nil)
  end

  def self.legacy_type_keys
    @fanplay_legacy_type_keys ||= legacy_constant_names(defined?(PBTypes) ? PBTypes : nil)
  end

  def self.legacy_item_ids
    @fanplay_legacy_item_ids ||= legacy_item_names.each_with_object({}) { |(id, name), out| out[name] = id }
  end

  def self.legacy_species_ids
    @fanplay_legacy_species_ids ||= legacy_species_names.each_with_object({}) { |(id, name), out| out[name] = id }
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
    return @fanplay_legacy_evolutions if @fanplay_legacy_evolutions
    result = {}
    return @fanplay_legacy_evolutions = result unless defined?(PBSpecies)
    file = legacy_kernel(:pbRgssOpen, 'Data/evolutions.dat', 'rb')
    return @fanplay_legacy_evolutions = result unless file
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
    @fanplay_legacy_evolutions = result
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
    @fanplay_legacy_tm ||= (load_game_data('Data/tm.dat') rescue nil)
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
    return @fanplay_legacy_items if @fanplay_legacy_items
    list = (defined?($ItemData) && $ItemData) || legacy_kernel(:readItemList, 'Data/items.dat')
    records = {}
    return @fanplay_legacy_items = records unless list
    max = defined?(PBItems) ? PBItems.maxValue : 0
    (1..max).each do |id|
      record = (list[id] rescue nil)
      next unless record
      next unless (record[0] || 0).to_i == id
      records[id] = record
    end
    @fanplay_legacy_items = records
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
    return @fanplay_legacy_locations if @fanplay_legacy_locations
    locations = Hash.new { |hash, key| hash[key] = [] }
    data = (load_game_data('Data/encounters.dat') rescue nil)
    infos = (load_game_data('Data/MapInfos.rxdata') rescue nil)
    infos = {} unless infos.is_a?(Hash)
    return @fanplay_legacy_locations = locations unless data.is_a?(Hash)
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
    @fanplay_legacy_locations = locations
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
    @fanplay_legacy_dex = nil if refresh
    return @fanplay_legacy_dex['index'] if @fanplay_legacy_dex
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
    @fanplay_legacy_dex = {
      'records' => records.each_with_object({}) { |(species, record), out| out[record['key']] = record },
      'locations' => locations,
      'index' => { 'species' => index, 'note' => 'Known sources include wild encounters, party, evolution and breeding. Scripted gifts, trades, quests and conditional forms may not be indexed. No known source does not mean unobtainable.' }
    }
    @fanplay_legacy_dex['index']
  end

  def self.legacy_species_entry(id)
    legacy_pokemon_essentials_pokedex
    record = @fanplay_legacy_dex['records'][id.to_s] ||
             @fanplay_legacy_dex['records'].values.find { |row| row['id'].to_s == id.to_s }
    raise ArgumentError, "Unknown game species: #{id}" unless record
    species = record['id']
    bytes = record['bytes'] || legacy_dex_bytes(species)
    summary = @fanplay_legacy_dex['index']['species'].find { |row| row['id'] == record['key'] } || {}
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
      'locations' => @fanplay_legacy_dex['locations'][species],
      'moves' => moves,
      'evolutions' => legacy_evolutions(species),
      'previous' => legacy_previous_evolutions(species),
      'raw' => legacy_species_raw(species, bytes)
    )
  end

  def self.legacy_pokemon_essentials_movedex(refresh = false)
    @fanplay_legacy_movedex = nil if refresh
    return @fanplay_legacy_movedex if @fanplay_legacy_movedex
    machines = legacy_move_machines
    rows = []
    (1..(defined?(PBMoves) ? PBMoves.maxValue : 0)).each do |id|
      details = legacy_move_details(id)
      next unless details
      next if details['name'].nil? || details['name'].to_s.strip.empty?
      rows << details.merge(
        'number' => nil,
        'category_name' => { 0 => 'Physical', 1 => 'Special', 2 => 'Status' }[details['category'].to_i] || details['category'].to_s,
        'machines' => machines[id] || [], 'raw' => legacy_move_raw(id, details)
      )
    end
    @fanplay_legacy_movedex = { 'entries' => rows }
  end

  def self.legacy_pokemon_essentials_itemdex(refresh = false)
    @fanplay_legacy_itemdex = nil if refresh
    return @fanplay_legacy_itemdex if @fanplay_legacy_itemdex
    rows = legacy_item_records.map do |id, record|
      machine = record[9].to_i
      {
        'id' => legacy_item_key(id), 'number' => id,
        'name' => legacy_item_name(id) || legacy_item_key(id),
        'description' => (legacy_message(:ItemDescriptions, id) || record[5]),
        'pocket' => record[3], 'pocket_name' => legacy_pocket_name(record[3]),
        'price' => record[4], 'sell_price' => nil, 'bp_price' => nil, 'consumable' => nil,
        'field_use' => record[6], 'field_use_label' => legacy_item_use_label(record[6]),
        'battle_use' => record[7], 'battle_use_label' => legacy_item_battle_use_label(record[7]),
        'tags' => legacy_item_tags(record, id), 'flags' => [],
        'move' => machine.positive? ? legacy_dex_move(machine, 'Taught by item') : nil,
        'icon' => legacy_item_icon(id), 'raw' => legacy_item_raw(record, id)
      }
    end
    @fanplay_legacy_itemdex = { 'entries' => rows }
  end
end
