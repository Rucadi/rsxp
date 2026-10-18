# SPDX-License-Identifier: GPL-2.0-or-later
# Desktop-only sampling profiler. Loaded by Magnus only when requested.
require 'json'
begin
  require 'stackprof'
rescue LoadError
  raise LoadError, 'Ruby profiling requires StackProf 0.2.28. Use nix develop/nix-shell, or install the stackprof gem for the embedded Ruby.'
end

module RSXP
  module RubyProfiler
    def self.configure(path, mode, interval)
      @path, @mode, @interval = path, mode.to_sym, interval
      @running = false
    end

    def self.start(frame)
      @start_frame = frame
      @started_at = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      @allocations = GC.stat(:total_allocated_objects)
      @gc_count = GC.stat(:count)
      raise 'A StackProf recording is already active' unless StackProf.start(mode: @mode, interval: @interval, raw: true)
      @running = true
      STDERR.puts "Ruby profiling started at update #{frame} (#{@mode}, #{@interval} microseconds)"
    end

    def self.finish(frame)
      return unless @running
      allocated = GC.stat(:total_allocated_objects) - @allocations
      gc_count = GC.stat(:count) - @gc_count
      elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - @started_at
      StackProf.stop
      @running = false
      data = StackProf.results
      File.binwrite(@path, Marshal.dump(data))
      methods = data.fetch(:frames).values.map do |entry|
        { name: entry[:name], file: entry[:file], line: entry[:line],
          self_samples: entry[:samples], total_samples: entry[:total_samples] }
      end.sort_by { |entry| -entry[:self_samples] }
      summary = { mode: @mode, interval_us: @interval, samples: data[:samples],
        missed_samples: data[:missed_samples], gc_samples: data[:gc_samples],
        start_update: @start_frame, end_update: frame, elapsed_wall_seconds: elapsed,
        allocated_objects: allocated, gc_cycles: gc_count,
        yjit_enabled: defined?(RubyVM::YJIT) && RubyVM::YJIT.enabled?, methods: methods }
      File.write(@path+'.json', JSON.pretty_generate(summary))
      File.open(@path+'.txt','w') do |out|
        out.puts "Ruby CPU/wall stack samples; includes native calls and GC. Self samples identify leaves; total samples include callees."
        out.puts "Recorded updates #{@start_frame}..#{frame}; #{elapsed.round(3)} seconds."
        out.puts "Allocated objects: #{allocated}; GC cycles: #{gc_count}."
        StackProf::Report.new(data).print_text(false,40,nil,nil,nil,nil,out)
        out.puts "\nSource locations:"
        methods.first(40).each { |entry| out.puts "#{entry[:name]}: #{entry[:file]}:#{entry[:line]}" }
      end
      STDERR.puts "Ruby profile saved: #{@path} (+ .txt and .json), #{data[:samples]} samples"
      STDERR.puts 'Too few samples for a reliable ranking; record a longer gameplay window.' if data[:samples] < 1000
    end
  end
end
RSXP::RubyProfiler
