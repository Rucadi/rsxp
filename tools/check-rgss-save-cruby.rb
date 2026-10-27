# SPDX-License-Identifier: GPL-2.0-or-later
# Compare serialized values using CRuby; empty class shells decode game data
# without executing game scripts. Usage: ruby check-rgss-save-cruby.rb ORIGINAL REWRITTEN
def load_game_data(path)
  loop do
    begin
      return Marshal.load(File.binread(path))
    rescue ArgumentError => e
      name=e.message[/undefined class\/module ([A-Za-z_][A-Za-z_0-9:]*)/,1]
      raise unless name
      scope=Object
      name.sub(/::\z/,'').split('::').each do |part|
        unless scope.const_defined?(part,false)
          type=Class.new do
            def self._load(bytes); obj=allocate; obj.instance_variable_set(:@raw_payload,bytes); obj; end
            def _dump(*); @raw_payload; end
          end
          scope.const_set(part,type)
        end
        scope=scope.const_get(part,false)
      end
    end
  end
end
original=load_game_data(ARGV.fetch(0))
rewritten=load_game_data(ARGV.fetch(1))
$visited={}
def compare(a,b,path)
  raise "class mismatch #{path}: #{a.class} vs #{b.class}" unless a.class==b.class
  return if $visited[[a.object_id,b.object_id]]
  $visited[[a.object_id,b.object_id]]=true
  case a
  when Time
    raise "timestamp mismatch #{path}" unless a.to_i==b.to_i && a.nsec==b.nsec && a.utc? == b.utc? && a.utc_offset==b.utc_offset
  when Array
    raise "array size #{path}" unless a.size==b.size
    a.each_index { |i| compare(a[i],b[i],"#{path}[#{i}]") }
  when Hash
    raise "hash keys #{path}" unless a.keys==b.keys
    a.each { |k,v| compare(v,b[k],"#{path}[#{k}]") }
    compare(a.default,b.default,"#{path}.default")
  when String,Symbol,Numeric,NilClass,TrueClass,FalseClass
    raise "value mismatch #{path}: #{a.inspect} (#{a.respond_to?(:encoding) ? a.encoding : nil}) vs #{b.inspect} (#{b.respond_to?(:encoding) ? b.encoding : nil})" unless a==b || (a.is_a?(Float) && a.nan? && b.nan?)
  end
  av=a.instance_variables.sort; bv=b.instance_variables.sort
  raise "ivars mismatch #{path}: #{av} vs #{bv}" unless av==bv
  av.each { |n| compare(a.instance_variable_get(n),b.instance_variable_get(n),"#{path}.#{n}") }
end
compare(original,rewritten,'save')
puts "CRUBY_FULL_SAVE_GRAPH_OK pairs=#{$visited.size}"
