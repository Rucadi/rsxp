# SPDX-License-Identifier: GPL-2.0-or-later
Graphics.resize_screen(320, 240)
background = Sprite.new
background.bitmap = Bitmap.new(320, 240)
background.bitmap.fill_rect(0, 0, 320, 240, Color.new(20, 31, 49))
background.bitmap.font.size = 17
background.bitmap.draw_text(12, 12, 296, 28, 'RSXP · Native mobile core', 1)
background.bitmap.font.size = 13
background.bitmap.draw_text(12, 46, 296, 24, 'Move with the D-pad. A changes colour.', 1)
player = Sprite.new
player.bitmap = Bitmap.new(24, 24)
player.bitmap.fill_rect(0, 0, 24, 24, Color.new(108, 224, 189))
player.x, player.y = 148, 130
loop do
  Input.update
  player.x -= 3 if Input.press?(Input::LEFT)
  player.x += 3 if Input.press?(Input::RIGHT)
  player.y -= 3 if Input.press?(Input::UP)
  player.y += 3 if Input.press?(Input::DOWN)
  player.x = [[player.x, 0].max, 296].min
  player.y = [[player.y, 78].max, 216].min
  if Input.trigger?(Input::C)
    player.bitmap.fill_rect(0, 0, 24, 24, Color.new(rand(180)+60, rand(180)+60, rand(180)+60))
  end
  Graphics.update
end
