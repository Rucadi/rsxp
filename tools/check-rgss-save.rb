# SPDX-License-Identifier: GPL-2.0-or-later
# Opt-in Essentials/Multi Save integration check. Reads game assets, writes
# only to FANPLAY_SAVE_CHECK_DIR, where original.rxdata must be a save copy.
check_dir=File.expand_path(ENV.fetch('FANPLAY_SAVE_CHECK_DIR'))
Dir.chdir(ENV.fetch('FANPLAY_SAVE_CHECK_GAME'))
load 'preload.rb'
$RGSS_SCRIPTS=load_data('Data/Scripts.rxdata')
$RGSS_SCRIPTS.each do |s|
  next if s[1]=='Main'
  s[3]=Zlib::Inflate.inflate(s[2]).force_encoding(Encoding::UTF_8)
  MKXP.eval_script(s[3],s[1])
  MKXP.publish_top_level_methods
end
load_data('Data/PluginScripts.rxdata').each do |name,meta,scripts|
  next if meta[:disabled] && ['true','verdadero','si','x'].include?(meta[:disabled].to_s.downcase)
  PluginManager.register(meta)
  scripts.each do |file,compressed|
    MKXP.eval_script(Zlib::Inflate.inflate(compressed).force_encoding(Encoding::UTF_8),"[#{name}] #{file}")
    MKXP.publish_top_level_methods
  end
end
path=File.join(check_dir,'original.rxdata')
save=SaveData.get_data_from_file(path)
raise 'save is not Hash' unless save.is_a?(Hash)
player=save[:player]
raise 'missing player' unless player.is_a?(Player)
if player.last_time_saved
  raise 'timestamp' unless player.last_time_saved.is_a?(Time)
  player.last_time_saved.strftime('%x %I:%M%p')
end
File.open(File.join(check_dir,'roundtrip.rxdata'),'wb') { |f| Marshal.dump(save,f) }
again=SaveData.get_data_from_file(File.join(check_dir,'roundtrip.rxdata'))
raise 'party changed' unless player.party.size==again[:player].party.size
raise 'map changed' unless save[:map_factory].map.map_id==again[:map_factory].map.map_id
raise 'time changed' unless player.last_time_saved==again[:player].last_time_saved
raise 'money changed' unless player.money==again[:player].money
puts "SAVE_ROUNDTRIP_OK keys=#{save.size} map=#{save[:map_factory].map.map_id} party=#{player.party.size}"
# Exercise the exact UI method in the reported backtrace without opening a menu.
$save_check_roundtrip=File.join(check_dir,'roundtrip.rxdata')
module SaveData
  def self.get_full_path(slot); $save_check_roundtrip; end
  def self.read_from_file(path); get_data_from_file(path); end
end
$game_temp=Game_Temp.new
text=PokemonSaveScreen.allocate.getSaveInfoBoxContents('test')
raise 'slot content' unless text.is_a?(String) && !text.empty?
puts 'SAVE_SLOT_PREVIEW_OK'
