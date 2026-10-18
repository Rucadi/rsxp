# SPDX-License-Identifier: GPL-2.0-or-later
require 'json'
require_relative 'game_bridge'
def assert(value, message) = (raise(message) unless value)

# These registries deliberately do not define Species or a player. Catalogs must
# be usable without a party and must preserve modified game data and false/zero.
module GameData
  class Type
    def self.try_get(id) = Struct.new(:name).new({ GRASS: 'Planta', FIRE: 'Fuego' }[id] || id.to_s)
  end
  class Target
    def self.try_get(id) = Struct.new(:name).new('One nearby foe')
  end
  class Move
    DATA = {}
    attr_reader :id, :name, :description, :type, :category, :power, :accuracy, :total_pp, :priority, :effect_chance, :target, :function_code, :flags, :id_number
    def initialize(id, power, category = 1)
      @id=id; @name="Custom #{id}"; @description='Game-specific description.'
      @type=:GRASS; @category=category; @power=power; @accuracy=0; @total_pp=17
      @priority=-1; @effect_chance=30; @target=:NearFoe; @function_code='CustomEffect'; @flags=['Contact']; @id_number=25
    end
    def self.each(&block) = DATA.each_value(&block)
    def self.try_get(id) = DATA[id.to_sym]
  end
  class Item
    DATA = {}
    attr_reader :id, :name, :description, :pocket, :price, :sell_price, :bp_price, :consumable, :field_use, :battle_use, :flags, :move
    def initialize(id, machine = false)
      @id=id; @name=machine ? 'Custom TM' : 'Custom potion'; @description='A game-specific item.'
      @pocket=2; @price=0; @sell_price=0; @bp_price=0; @consumable=!machine
      @field_use=machine ? 3 : 1; @battle_use=machine ? 0 : 1; @flags=machine ? [] : ['Healing']
      @move=machine ? :CUSTOMMOVE : nil
    end
    def self.each(&block) = DATA.each_value(&block)
    def self.try_get(id) = DATA[id.to_sym]
    def self.icon_filename(id) = "Graphics/Items/#{id}"
    def self.schema = {'FieldUse'=>[nil,nil,{'OnPokemon'=>1,'TM'=>3}], 'BattleUse'=>[nil,nil,{'OnPokemon'=>1}]}
    def consumed_after_use? = @consumable
    def is_healing_item? = @flags.include?('Healing')
    def is_machine? = !@move.nil?
    def can_hold? = !is_machine?
    def display_name = is_machine? ? "#{@name} #{GameData::Move.try_get(@move).name}" : @name
  end
end
class PokemonBag
  def self.pocket_names = ['', 'Items', 'Medicine']
end
GameData::Move::DATA[:CUSTOMMOVE]=GameData::Move.new(:CUSTOMMOVE,93)
GameData::Move::DATA[:STATUSMOVE]=GameData::Move.new(:STATUSMOVE,0,2)
# Legacy numeric aliases should not duplicate catalog rows.
GameData::Move::DATA[1]=GameData::Move::DATA[:CUSTOMMOVE]
260.times { |i| id="EXTRA#{i}".to_sym; GameData::Move::DATA[id]=GameData::Move.new(id,i) }
GameData::Item::DATA[:POTION]=GameData::Item.new(:POTION)
GameData::Item::DATA[:TM99]=GameData::Item.new(:TM99,true)
api=RSXP::GameAPI
moves=api.dispatch('pokemon_essentials_movedex',['refresh'])['entries']
assert(moves.length==262,'Catalog must include every move and deduplicate aliases without a 200-row cap')
move=moves.find { |row|row['id']=='CUSTOMMOVE' }
assert(move['power']==93&&move['pp']==17&&move['priority']==-1,'Preserve modified game move values')
assert(move['accuracy']==0&&move['type_name']=='Planta'&&move['target']=='One nearby foe','Preserve no-check accuracy and game names')
assert(move['category_name']=='Special'&&move['flags']==['Contact'],'Expose move categories and flags for filtering')
items=api.dispatch('pokemon_essentials_itemdex',['refresh'])['entries']
potion=items.find { |row|row['id']=='POTION' };machine=items.find { |row|row['id']=='TM99' }
assert(potion['pocket_name']=='Medicine'&&potion['tags'].include?('Medicine'),'Use game bag pockets and item predicates')
assert(potion['price']==0&&potion['sell_price']==0,'Free items must not be confused with missing prices')
assert(machine['consumable']==false&&machine['tags'].include?('TM / HM / TR'),'Keep non-consumable machines filterable')
assert(machine['move']['power']==93&&machine['name'].include?('CUSTOMMOVE'),'Read taught moves and game display names')
assert(machine['field_use_label']=='TM'&&machine['battle_use_label']=='Not usable','Decode item use modes from the game schema')
assert(machine['icon']=='Graphics/Items/TM99','Expose the game-resolved icon')
PokemonBag.define_singleton_method(:pocket_names) { ['Items', 'Medicine'] }
assert(api.pokemon_essentials_itemdex(true)['entries'].first['pocket_name']=='Medicine','Support zero-based pocket name arrays without shifting their labels')
GameData::Move::DATA[:CUSTOMMOVE].instance_variable_set(:@power,108)
assert(api.pokemon_essentials_movedex['entries'].first['power']==93,'Catalog should be cached until refresh')
assert(api.pokemon_essentials_movedex(true)['entries'].first['power']==108,'Refresh must rebuild game data')
if ENV['RSXP_REGISTRY_FIXTURE']
  File.write(ENV['RSXP_REGISTRY_FIXTURE'],JSON.pretty_generate({'moves'=>{'entries'=>moves.first(2)},'items'=>{'entries'=>items}}))
end
puts 'Movedex and Itemdex adapter checks passed'

# Inventory changes use the live bag's methods, never direct instance-variable edits.
class TestBag
  attr_reader :calls
  def initialize(capacity=10) = (@counts=Hash.new(0); @capacity=capacity; @calls=0)
  def quantity(item) = @counts[item]
  def can_add?(item,quantity) = @counts[item]+quantity<=@capacity
  def add(item,quantity) = (@calls+=1; @counts[item]+=quantity; true)
end
$bag=TestBag.new
change=api.dispatch('pokemon_essentials_add_item',['POTION','3'])
assert(change['added']==3&&change['total']==3&&change['complete'],'Confirm the actual added quantity and bag total')
assert(api.pokemon_essentials_add_item('POTION',2)['total']==5,'Repeated deliberate additions should update the live bag')
%w[0 -1 10000 1.5 invalid].each do |quantity|
  begin
    api.pokemon_essentials_add_item('POTION',quantity)
    raise 'Invalid quantity accepted'
  rescue ArgumentError
  end
end
assert($bag.calls==2,'Invalid quantities must not call the bag mutation method')
begin
  api.pokemon_essentials_add_item('UNKNOWN',1)
  raise 'Unknown item accepted'
rescue ArgumentError
end
$bag=TestBag.new(2)
begin
  api.pokemon_essentials_add_item('POTION',3)
  raise 'Full bag accepted'
rescue RuntimeError=>error
  assert(error.message.include?('espacio'),'Report insufficient capacity')
end
assert($bag.calls==0&&$bag.quantity(:POTION)==0,'Capacity failure must not partially add a stack')
$bag=nil
begin
  api.pokemon_essentials_add_item('POTION',1)
  raise 'Missing bag accepted'
rescue RuntimeError=>error
  assert(error.message.include?('disponible'),'Report an uninitialized game bag')
end
class LegacyTestBag
  def initialize = (@counts=Hash.new(0))
  def pbQuantity(item) = @counts[item]
  def pbCanStore?(item,quantity) = @counts[item]+quantity<=10
  def pbStoreItem(item,quantity) = (@counts[item]+=quantity; true)
end
$PokemonBag=LegacyTestBag.new
assert(api.pokemon_essentials_add_item('TM99',2)['total']==2,'Support legacy bag accessors')
$PokemonBag=nil
class PartialTestBag<TestBag
  def add(item,quantity) = super(item,1)
end
$bag=PartialTestBag.new
partial=api.pokemon_essentials_add_item('POTION',3)
assert(partial['added']==1&&!partial['complete'],'Report actual partial additions from a custom bag rather than claiming full success')
$bag=nil
machines=api.pokemon_essentials_movedex(true)['entries'].find{|row|row['id']=='CUSTOMMOVE'}['machines']
assert(machines.map{|row|row['id']}==['TM99'],'Movedex must link to the actual matching game TM')
assert(api.pokemon_essentials_movedex['entries'].find{|row|row['id']=='STATUSMOVE'}['machines'].empty?,'Do not invent a TM for a move without one')
# HM and TR records must not be offered as TMs.
[:HM01,:TR01].zip([4,5]).each do |id,field_use|
  item=GameData::Item.new(id,true);item.instance_variable_set(:@field_use,field_use);GameData::Item::DATA[id]=item
end
assert(api.pokemon_essentials_movedex(true)['entries'].find{|row|row['id']=='CUSTOMMOVE'}['machines'].map{|row|row['id']}==['TM99'],'Only real TMs belong in the add-TM action')
puts 'Inventory and move TM checks passed'
