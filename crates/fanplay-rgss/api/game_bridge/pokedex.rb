module FANPLAY::GameAPI
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
    @fanplay_dex = nil if refresh
    return @fanplay_dex['index'] if @fanplay_dex
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
    @fanplay_dex = { 'records' => by_id, 'locations' => locations,
      'index' => { 'species' => index, 'note' => 'Known sources include wild encounters, party, evolution and breeding. Scripted gifts, trades, quests and conditional forms may not be indexed. No known source does not mean unobtainable.' } }
    @fanplay_dex['index']
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
    record = @fanplay_dex['records'][id.to_s]
    raise ArgumentError, "Unknown game species: #{id}" unless record
    summary = @fanplay_dex['index']['species'].find { |row| row['id'] == id.to_s }
    evolutions = dex_evolutions(record)
    previous = @fanplay_dex['records'].values.flat_map do |other|
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
      'locations' => @fanplay_dex['locations'][id.to_s], 'moves' => moves,
      'evolutions' => evolutions, 'previous' => previous,
      'raw' => extension_data(record))
  end
end

