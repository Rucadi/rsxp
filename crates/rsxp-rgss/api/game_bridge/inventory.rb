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
# so the Pokédex, Movedex, Itemdex and current-map encounters work for both data
# models. Everything runs on the game thread and only reads existing game state.
