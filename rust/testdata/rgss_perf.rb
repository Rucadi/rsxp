# SPDX-License-Identifier: GPL-2.0-or-later
# Repeatable full RGSS update workload, excluding startup and asset decoding.
# Run with --headless and either --renderer software or --renderer wgpu.
Graphics.resize_screen(640,480)
bitmap=Bitmap.new(64,64)
bitmap.fill_rect(bitmap.rect,Color.new(80,170,230,210))
viewport=Viewport.new(0,0,640,480)
viewport.tone=Tone.new(10,-5,0,25)
sprites=Array.new(96) do |i|
  sprite=Sprite.new(viewport)
  sprite.bitmap=bitmap
  sprite.x=(i*53)%620-20
  sprite.y=(i*37)%460-20
  sprite.opacity=190
  sprite
end
20.times { Graphics.update; Input.update }
frames=300
start=Process.clock_gettime(Process::CLOCK_MONOTONIC)
frames.times do |frame|
  sprites.each_with_index { |sprite,i| sprite.x=(i*53+frame)%620-20 if i%4==0 }
  Graphics.update
  Input.update
  8.times { Input.press?(:C); Input.trigger?(:B); Input.pressex?(:F1) }
end
# Snap also fences GPU work so reported throughput includes completed frames.
Graphics.snap_to_bitmap.dispose
elapsed=Process.clock_gettime(Process::CLOCK_MONOTONIC)-start
puts "RGSS_BENCH frames=#{frames} ms_per_frame=#{(elapsed*1000/frames).round(3)} fps=#{(frames/elapsed).round(1)}"
sprites.each(&:dispose)
viewport.dispose
bitmap.dispose
