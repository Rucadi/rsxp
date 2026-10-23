module Graphics
	class << self
		alias_method(:win32wrap_update, :update)
		def update
			win32wrap_update
			$win32KeyStates = nil
		end
	end
end

def get_raw_keystates
	if $win32KeyStates == nil
		$win32KeyStates = Input.raw_key_states
	end

	return $win32KeyStates
end

def common_keystate(vkey)
	vkey_name = Scancodes::WIN32INV[vkey]

	states = get_raw_keystates
	pressed = false

	if vkey_name == :LBUTTON
		pressed = Input.press?(Input::MOUSELEFT)
	elsif vkey_name == :RBUTTON
		pressed = Input.press?(Input::MOUSERIGHT)
	elsif vkey_name == :MBUTTON
		pressed = Input.press?(Input::MOUSEMIDDLE)
	elsif vkey_name == :SHIFT
		pressed = double_state(states, :LSHIFT, :RSHIFT)
	elsif vkey_name == :MENU
		pressed = double_state(states, :LALT, :RALT)
	elsif vkey_name == :CONTROL
		pressed = double_state(states, :LCTRL, :RCTRL)
	else
		scan = nil
		if Scancodes::SDL.key?(vkey_name)
			scan = vkey_name
		else
			scan = Scancodes::WIN2SDL[vkey_name]
		end

		pressed = state_pressed(states, scan)
	end

	return pressed ? 1 : 0
end

def memcpy_string(dst, src)
	i = 0
	src.each_byte do |b|
		dst.setbyte(i, b)
		i += 1
	end
end

def state_pressed(states, sdl_scan)
	return states[Scancodes::SDL[sdl_scan]]
end

def double_state(states, left, right)
	return state_pressed(states, left) || state_pressed(states, right)
end

