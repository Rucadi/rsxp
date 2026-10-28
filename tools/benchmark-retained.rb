# SPDX-License-Identifier: GPL-2.0-or-later
# Existing save-safe benchmark plus counters for the retained desktop adapters.
begin
  load File.expand_path('benchmark-anil.rb',__dir__)
ensure
  if defined?(FANPLAY::Retained)
    resources=FANPLAY::Retained.instance_variable_get(:@resources)
    compatibility=FANPLAY::Retained.instance_variable_get(:@compatibility)
    if defined?(TilemapRenderer)
      chunks=ObjectSpace.each_object(TilemapRenderer).filter_map do |renderer|
        cache=renderer.instance_variable_get(:@__fanplay_cell_cache)
        cache.chunk_stats if cache && cache.respond_to?(:chunk_stats) && !renderer.disposed?
      end
      totals=chunks.transpose.map(&:sum) unless chunks.empty?
      puts "WORLD_CHUNKS:#{JSON.generate(%i[builds hits checks evictions resident].zip(totals || [0,0,0,0,0]).to_h)}"
    end
    puts "RETAINED_COUNTS:#{JSON.generate(FANPLAY::Retained.stats.merge(resources: resources.length, compatibility: compatibility.length))}"
  end
end
