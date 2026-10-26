module FANPLAY::GameAPI
  def self.dex_registry(kind)
    unless defined?(GameData) && GameData.const_defined?(kind, false) && GameData.const_get(kind, false).respond_to?(:each)
      raise RuntimeError, "This game does not expose a GameData::#{kind} registry"
    end
    records = []
    GameData.const_get(kind, false).each { |record| records << record }
    records.uniq { |record| dex_id(record) }
  end

  def self.pokemon_essentials_movedex(refresh = false)
    return legacy_pokemon_essentials_movedex(refresh) if legacy_essentials?
    @fanplay_movedex = nil if refresh
    return @fanplay_movedex if @fanplay_movedex
    records = dex_registry(:Move)
    machines = Hash.new { |hash, key| hash[key] = [] }
    if GameData.const_defined?(:Item, false) && GameData::Item.respond_to?(:each)
      dex_registry(:Item).each do |item|
        is_tm = item.respond_to?(:is_TM?) ? item.is_TM? : read(item, :field_use) == 3
        move = read(item, :move)
        next unless is_tm && move
        machines[move.to_s] << { 'id' => dex_id(item), 'name' => read_alias(item, :display_name, :name) || dex_id(item) }
      end
    end
    @fanplay_movedex = { 'entries' => records.map do |record|
      id = dex_id(record)
      row = dex_move(id, 'Game move')
      category = read(record, :category)
      row.merge('number' => read(record, :id_number),
        'type_name' => dex_named(:Type, read(record, :type))['name'],
        'category_name' => { 0 => 'Physical', 1 => 'Special', 2 => 'Status' }[category] || category.to_s,
        'target' => dex_named(:Target, read(record, :target))['name'],
        'effect_chance' => read(record, :effect_chance),
        'machines' => machines[id], 'flags' => Array(read(record, :flags)).map(&:to_s), 'raw' => extension_data(record))
    end }
  end

  def self.dex_item_usage(record, key)
    value = read(record, key)
    schema_key = key == :field_use ? 'FieldUse' : 'BattleUse'
    schema = GameData::Item.respond_to?(:schema) ? GameData::Item.schema : nil
    choices = schema && schema[schema_key] && schema[schema_key][2]
    name = choices.key(value) if choices.is_a?(Hash)
    return name.to_s.gsub(/([a-z])([A-Z])/) { "#{$1} #{$2}" } if name
    value.nil? ? 'Not supplied' : (value == 0 ? 'Not usable' : "Mode #{value}")
  rescue StandardError
    value.to_s
  end

  def self.pokemon_essentials_itemdex(refresh = false)
    return legacy_pokemon_essentials_itemdex(refresh) if legacy_essentials?
    @fanplay_itemdex = nil if refresh
    @fanplay_itemdex ||= { 'entries' => dex_registry(:Item).map do |record|
      id = dex_id(record)
      pocket = read(record, :pocket)
      pocket_names = defined?(PokemonBag) ? call(PokemonBag, :pocket_names) : nil
      if pocket_names.is_a?(Array) && pocket.is_a?(Integer) && pocket.positive?
        offset = pocket_names.first.nil? || pocket_names.first.to_s.empty? ? 0 : 1
        pocket_name = pocket_names[pocket - offset]
      end
      tags = { 'Medicine' => :is_healing_item?, 'Berry' => :is_berry?, 'Poké Ball' => :is_poke_ball?,
               'Key item' => :is_key_item?, 'TM / HM / TR' => :is_machine?, 'Evolution item' => :is_evolution_stone?,
               'Mega Stone' => :is_mega_stone?, 'Can be held' => :can_hold?, 'Mail' => :is_mail?,
               'Vitamin' => :is_vitamin?, 'Fossil' => :is_fossil? }.filter_map { |name, predicate| name if call(record, predicate) }
      move = read(record, :move)
      icon = call(GameData::Item, :icon_filename, read(record, :id))
      { 'id' => id, 'number' => read(record, :id_number),
        'name' => read_alias(record, :display_name, :name) || id,
        'description' => read(record, :description), 'pocket' => pocket,
        'pocket_name' => pocket_name || (pocket.nil? ? 'Not supplied' : "Pocket #{pocket}"),
        'price' => read(record, :price), 'sell_price' => read(record, :sell_price), 'bp_price' => read(record, :bp_price),
        'consumable' => read_alias(record, :consumed_after_use?, :consumable),
        'field_use' => read(record, :field_use), 'field_use_label' => dex_item_usage(record, :field_use),
        'battle_use' => read(record, :battle_use), 'battle_use_label' => dex_item_usage(record, :battle_use),
        'tags' => tags, 'flags' => Array(read(record, :flags)).map(&:to_s),
        'move' => move ? dex_move(move, 'Taught by item') : nil,
        'icon' => icon, 'raw' => extension_data(record) }
    end }
  end
end

