# SPDX-License-Identifier: GPL-2.0-or-later
# Test-only world-map helpers. Runtime installation lives in the
# essentials-world-map Rhai plugin; this keeps direct Ruby adapter tests
# independent from the Rhai host.

module FANPLAY
  module GameAPI
    def self.map_transfer_edges(map_id, map)
      events = read(map, :events)
      return [] unless events.respond_to?(:values)
      events.values.flat_map do |event|
        pages = read(event, :pages)
        Array(pages).flat_map do |page|
          commands = read(page, :list)
          Array(commands).filter_map do |command|
            next unless read(command, :code).to_i == 201
            parameters = read(command, :parameters)
            next unless parameters.respond_to?(:[]) && parameters[0].to_i == 0
            destination_map = Integer(parameters[1] || 0)
            next unless destination_map.positive?
            [map_id, Integer(read(event, :x) || 0), Integer(read(event, :y) || 0),
             destination_map, Integer(parameters[2] || 0), Integer(parameters[3] || 0)]
          end
        end
      end
    end

    def self.world_map_metadata(map_ids = nil)
      result = {}
      path = File.join(Dir.pwd, 'PBS', 'metadata.txt')
      if File.file?(path)
        current_id = nil
        File.foreach(path, encoding: 'UTF-8') do |raw|
          line = raw.sub(/^\uFEFF/, '').strip
          if line =~ /^\[(\d+)\]$/
            current_id = Integer(Regexp.last_match(1), 10)
            result[current_id] ||= {}
          elsif current_id && line == 'Outdoor=true'
            result[current_id]['outdoor'] = true
          elsif current_id && line.start_with?('MapPosition=')
            values = line.split('=', 2)[1].split(',').map { |value| Integer(value, 10) rescue nil }
            result[current_id]['position'] = values if values.length == 3 && values.none?(&:nil?)
          end
        end
      end
      if map_ids && defined?(GameData::MapMetadata)
        map_ids.each do |map_id|
          record = if GameData::MapMetadata.respond_to?(:try_get)
                     GameData::MapMetadata.try_get(map_id)
                   elsif GameData::MapMetadata.respond_to?(:get)
                     GameData::MapMetadata.get(map_id)
                   end
          next unless record
          entry = result[Integer(map_id)] ||= {}
          outdoor = read_alias(record, :outdoor_map, :outdoor)
          entry['outdoor'] = outdoor == true unless outdoor.nil?
          position = read_alias(record, :town_map_position, :map_position)
          if position.respond_to?(:to_a)
            values = position.to_a.map { |value| Integer(value) rescue nil }
            entry['position'] = values if values.length == 3 && values.none?(&:nil?)
          end
        rescue StandardError
          next
        end
      end
      result
    end
  end
end
