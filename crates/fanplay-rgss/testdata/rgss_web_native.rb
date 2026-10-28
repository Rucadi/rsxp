# SPDX-License-Identifier: GPL-2.0-or-later
# Exercise the engine updates that must cross the web interpreter boundary.
raise 'retained state disabled' unless FANPLAY::Retained::ENABLED
Graphics.resize_screen(32, 24)
bitmap = Bitmap.new(16, 16)
bitmap.fill_rect(bitmap.rect, Color.new(240, 30, 20))
tile = Sprite.new
tile.bitmap = bitmap
tile.instance_variable_set(:@__fanplay_map_covered, true)
tile.instance_variable_set(:@priority, 0)
tiles = [[[tile]]]
Graphics.update
before = FANPLAY::Retained.stats[:synced]
tile.x = tile.x
Graphics.update
raise 'unchanged sprite polled' unless FANPLAY::Retained.stats[:synced] == before

raise 'world camera rejected' unless __fanplay_world_camera(1, tiles, [], 0, 0, 5, 7, 32, 32, 32, false, true)
raise 'world projection' unless [tile.x, tile.y] == [-5, -7]
Graphics.update
snapshot = Graphics.snap_to_bitmap
raise 'world update missed engine' unless snapshot.get_pixel(0, 0).red == 240
snapshot.dispose
raise 'camera move rejected' unless __fanplay_world_camera(1, tiles, [], 0, 0, 9, 7, 32, 32, 32, false, false)
raise 'camera coordinate stale' unless tile.x == -9
__fanplay_world_release(1, tiles)
raise 'release changed coordinate' unless tile.x == -9

raise 'bulk scroll rejected' unless __fanplay_scroll_tiles(tiles, 2, 3, 32, 32, 32, true, true, false, true)
raise 'bulk scroll coordinates' unless [tile.x, tile.y] == [-2, -3]
raise 'RGSS scroll rejected' unless __fanplay_scroll_rgss_tiles([tile], 1, 2)
raise 'RGSS scroll coordinates' unless [tile.x, tile.y] == [-3, -5]
plan = [[bitmap, 0, 0, 8, 12, 10, 0, 0]]
raise 'tile plan rejected' unless __fanplay_apply_rgss_tiles([tile], plan, [0, 0, 0, 0], [0, 0, 0, 0], nil, [0, 0, 32, 24])
Graphics.update
snapshot = Graphics.snap_to_bitmap
raise 'tile plan missed engine' unless snapshot.get_pixel(12, 10).red == 240
snapshot.dispose
tile.dispose
Graphics.update
snapshot = Graphics.snap_to_bitmap
raise 'disposed tile still drawn' if snapshot.get_pixel(12, 10).red == 240
snapshot.dispose

table = Table.new(2, 2, 3)
table[0, 0, 0] = 384
cache = FANPLAYRetainedMap.new
maps = [[1, table, 0, 0, 2, 2, 2]]
raise 'map cache missing tile' unless cache.update(2, 2, maps, false).any? { |cell| cell[4] == 384 }
raise 'unchanged map rescanned' unless cache.update(2, 2, maps, false).empty?
table[0, 0, 0] = 385
raise 'table revision missed' unless cache.update(2, 2, maps, false).any? { |cell| cell[4] == 385 }
cache.shift(1, 0)
raise 'chunk stats unavailable' unless cache.chunk_stats.length == 5
bitmap.dispose
puts 'WEB_NATIVE_OK'
