# SPDX-License-Identifier: GPL-2.0-or-later
# Run with: nix run path:. -- run --script rust/testdata/rgss_demo.rb
tile=Bitmap.new(48,48)
tile.fill_rect(tile.rect,Color.new(64,208,160))
tile.fill_rect(8,8,32,32,Color.new(210,255,230))
tile.fill_rect(14,14,8,8,Color.new(22,28,44))
tile.fill_rect(28,14,8,8,Color.new(22,28,44))
sprite=Sprite.new
sprite.bitmap=tile
sprite.x=(Graphics.width-tile.width)/2
sprite.y=(Graphics.height-tile.height)/2
rgss_main do
  loop do
    Graphics.update
    Input.update
    break if Input.trigger?(:B)
    sprite.x+=3 if Input.press?(:RIGHT)
    sprite.x-=3 if Input.press?(:LEFT)
    sprite.y+=3 if Input.press?(:DOWN)
    sprite.y-=3 if Input.press?(:UP)
  end
end
