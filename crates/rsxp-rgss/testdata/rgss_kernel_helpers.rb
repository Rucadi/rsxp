# SPDX-License-Identifier: GPL-2.0-or-later
# Initial, late-loaded and privatized helpers must retain argument/block/self
# behavior when legacy game scripts call them with an explicit Kernel receiver.
def pbRSXPCommands(value, selected: 0)
  raise 'wrong helper receiver' unless self.equal?(TOPLEVEL_BINDING.eval('self'))
  yield(value + selected)
end
RSXP.publish_top_level_methods
raise 'published helper arguments' unless Kernel.pbRSXPCommands(3, selected: 4) { |n| n * 2 } == 14
Kernel.singleton_class.send(:private, :pbRSXPCommands)
raise 'privatized helper arguments' unless Kernel.pbRSXPCommands(3, selected: 4) { |n| n * 2 } == 14

def pbRSXPLateCommands(value)
  value + 1
end
raise 'late helper unavailable' unless Kernel.pbRSXPLateCommands(5) == 6
raise 'helper discovery failed' unless Kernel.respond_to?(:pbRSXPLateCommands)
class << Kernel
  private
  def __rsxp_private_module_only = 7
end
begin
  Kernel.__rsxp_private_module_only
  raise 'private module-only method exposed'
rescue NoMethodError
end
begin
  Kernel.__rsxp_unknown_helper
  raise 'unknown helper accepted'
rescue NoMethodError
end
require 'tmpdir'
Dir.mktmpdir('rsxp-load-data') do |dir|
  path=File.join(dir,'data.rxdata')
  object={value: [1, 'test']}
  save_data(object,path)
  raise 'default Marshal load' unless load_data(path)==object
  raise 'explicit Marshal load' unless load_data(path,false)==object
  raise 'raw Marshal bytes' unless load_data(path,true)==File.binread(path)
  bytes="\x00\xffnot Marshal\r\n".b
  File.binwrite(path,bytes)
  raw=load_data(path,true)
  raise 'raw binary load' unless raw==bytes && raw.encoding==Encoding::BINARY
  File.binwrite(path,'')
  raise 'empty raw load' unless load_data(path,true)==''
  begin
    load_data(File.join(dir,'missing.rxdata'))
    raise 'missing load accepted'
  rescue Errno::ENOENT
  end
  begin
    load_data(File.join(dir,'missing.rxdata'),true)
    raise 'missing raw load accepted'
  rescue Errno::ENOENT
  end
end
puts 'KERNEL_GAME_HELPERS_OK'
