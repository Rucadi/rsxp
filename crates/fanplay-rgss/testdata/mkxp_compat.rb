# SPDX-License-Identifier: GPL-2.0-or-later
# Smoke test for the mkxp-z compatibility methods games call.
require 'tmpdir'
def check(value, message)
  raise "mkxp-compat: #{message}" unless value
end

Graphics.resize_screen(64, 48)

# Direction queries tolerate the movement-state argument used by Royal.
saved_state=Input.instance_variable_get(:@state)
begin
  [{}, {79=>1}, {79=>1,82=>1}, {603=>1}].each do |state|
    Input.instance_variable_set(:@state,state)
    [:dir4,:dir8].each do |method|
      expected=Input.public_send(method)
      [true,false,nil].each do |moving|
        check(Input.public_send(method,moving)==expected,"#{method} movement argument")
      end
      check(Input.public_send(method,true,false)==expected,"#{method} ignored arguments")
    end
  end
ensure
  Input.instance_variable_set(:@state,saved_state)
end

# Bitmap effects and snapshot/hi-res compatibility.
bitmap = Bitmap.new(4, 4)
bitmap.gradient_fill_rect(0, 0, 4, 4, Color.new(255, 0, 0), Color.new(0, 0, 255))
check(bitmap.get_pixel(0, 0).red > 200, 'gradient start')
check(bitmap.get_pixel(3, 0).blue > 200, 'gradient end')
bitmap.gradient_fill_rect(Rect.new(0, 0, 4, 4), Color.new(0, 0, 0, 0), Color.new(255, 255, 255, 255), true)
bitmap.blur
bitmap.radial_blur(20, 8)
snapshot = bitmap.snap_to_bitmap
check(!snapshot.animated?, 'snapshot should be static')
check(bitmap.hires.equal?(bitmap), 'hires should return self')
snapshot.dispose

Dir.mktmpdir('fanplay-compat') do |dir|
  path = File.join(dir, 'out.png')
  bitmap.to_file(path)
  check(File.exist?(path) && File.size(path) > 0, 'to_file should write a PNG')
end

# Graphics timing/display helpers.
Graphics.update
Graphics.update
check(Graphics.delta.is_a?(Float) && Graphics.delta >= 0.0, 'delta')
check(Graphics.average_frame_rate > 0.0, 'average_frame_rate')
check(Graphics.display_width == 64 && Graphics.display_height == 48, 'display size')
check(Graphics.frameskip == false, 'frameskip default')
Graphics.frameskip = true
check(Graphics.frameskip == true, 'frameskip setter')
Graphics.frameskip = false
check(Graphics.threadsafe == false, 'threadsafe default')
Dir.mktmpdir('fanplay-shot') do |dir|
  path = File.join(dir, 'shot.png')
  Graphics.screenshot(path)
  check(File.exist?(path), 'screenshot should write a PNG')
end

# Input controller/clipboard compatibility surface.
check(Input.connected? == false, 'connected?')
check(Input.power_level == 0, 'power_level')
check(Input.axes_left == 0 && Input.raw_axes == 0 && Input.axes_trigger == 0, 'controller axes')
check(Input::Controller.connected? == false, 'Input::Controller')
Input.clipboard = 'hello'
check(Input.clipboard == 'hello', 'clipboard')

# System helpers.
check(System.file_exist?('.'), 'file_exist?')
check(System.nproc >= 1, 'nproc')
check(System.memory >= 0, 'memory')
check(System.parse_csv("a,b\n1,2\n").is_a?(Array), 'parse_csv')
System.window_title = 'fanplay test'
check(System.window_title == 'fanplay test', 'window_title')
check(System.is_really_linux? == System.is_linux?, 'is_really_linux?')
check(System.is_really_windows? == System.is_windows?, 'is_really_windows?')
check(System.is_wine? == false, 'is_wine?')

# Font aliases and CFG.
font = Font.new
font.b = true
check(font.bold == true && font.b == true, 'font b alias')
font.i = true
check(font.italic == true && font.i == true, 'font i alias')
check(CFG['missing'].nil?, 'CFG read')
CFG['x'] = 1
check(CFG['x'] == 1, 'CFG write')

# Sprite wave compatibility accessors.
sprite = Sprite.new
sprite.wave_amp = 8
check(sprite.wave_amp == 8, 'sprite wave_amp')
sprite.dispose
bitmap.dispose
puts 'MKXP_COMPAT_OK'
