module Win32API_Impl
	# Route Win32API.new('KGL2.klib', 'KglXxx', ...) to the compatibility module.
	KGL2 = KGL2_Impl
	module User32
		class Keybd_event
			Seq = [
				[0xA4, 0, 0, 0],
				[0xD, 0, 0, 0],
				[0xD, 0, 2, 0],
				[0xA4, 0, 2, 0]]
			Seq2 = [
				[0x12, 0, 0, 0],
				[0xD, 0, 0, 0],
				[0xD, 0, 2, 0],
				[0x12, 0, 2, 0]]
			def initialize
				@index = 0
			end
			def call(args)
				seq = [args[0], args[1], args[2], args[3]]

				if seq == Seq[@index] or seq == Seq2[@index]
					@index += 1
				else
					@index = 0
				end

				if @index == 4
					@index = 0
					Graphics.fullscreen = !Graphics.fullscreen
				end
			end
		end

		class GetKeyState
			def call(vkey)
				return common_keystate(vkey[0])
			end
		end
		class GetAsyncKeyState
			PRESSED_BIT = (1 << 15)
			def call(vkey)
				return common_keystate(vkey[0]) == 1 ? PRESSED_BIT : 0
			end
		end
		# Games such as Realidea poll keys through Win32API and discard them
		# unless their RGSS window is foreground. RSXP already routes input only
		# to its active game window, represented by handle 42 in this shim.
		class GetForegroundWindow
			def call(_args) = 42
		end
		class GetKeyboardState
			PRESSED_BIT = 0x80
			def call(args)
				out_states = args[0]

				Scancodes::WIN32.each do |name, val|
					pressed = common_keystate(val) == 1

					out_states.setbyte(val, pressed ? PRESSED_BIT : 0)
				end
				return 1
			end
		end

		class ShowCursor
			def initialize
				@cursor_count = 0
			end
			def call(args)
				if args[0] == 1
					@cursor_count += 1
				else
					@cursor_count -= 1
				end

				Graphics.show_cursor = @cursor_count >= 0
			end
		end

		class GetCursorPos
			def call(args)
				out = [Input.mouse_x, Input.mouse_y].pack('ll')
				memcpy_string(args[0], out)
				return 1
			end
		end

		class GetClientRect
			def call(args)
				return 0 if args[0] != 42
				rect = [0, 0, 640, 480]
				begin
					rect[2] = Graphics.width
					rect[3] = Graphics.height
				rescue
				end
				memcpy_string(args[1], rect.pack('l4'))
				return 1
			end
		end

		class ScreenToClient
			def call(args)
				return 1
			end
		end

		class FindWindowA
			def call(args)
				if args[0] == "RGSS Player"
					return 42
				else
					return 0
				end
			end
		end

    FindWindow = FindWindowA
    GetWindowRect = GetClientRect
    class FindWindowEx
      def call(args)
        args[1]==0 && args[2]=='RGSS Player' ? 42 : 0
      end
    end
    FindWindowExA = FindWindowEx
    class GetWindowThreadProcessId
      def call(args)
        return 0 unless args[0]==42
        args[1].replace([Process.pid].pack('L')) if args[1].is_a?(String)
        Thread.current.object_id
      end
    end
    class GetSystemMetrics
      def call(args)
        case args[0]
        when 0 then Graphics.width
        when 1 then Graphics.height
        else 0
        end
      end
    end
    class SetWindowTextA
      def call(args)
        return 0 unless args[0]==42
        RSXP.call('window_title',payload:args[1].to_s)
        1
      end
    end
    SetWindowText = SetWindowTextA
    # Presentation is managed by the host; RGSS window decoration/position
    # requests need no extra OS window or framebuffer resize.
    class SetWindowPos
      def call(args) = args[0]==42 ? 1 : 0
    end
    ShowWindow = SetWindowPos
    class SetWindowLong
      def call(_args) = 0
    end
    SetWindowLongA = SetWindowLong
	end
  module Kernel32
    class GetCurrentThreadId
      def call(_args) = Thread.current.object_id
    end
    class GetCurrentProcessId
      def call(_args) = Process.pid
    end
    class GetPrivateProfileString
      def call(args)
        section,key,default,buffer,capacity,path=args
        value=default.to_s
        begin
          current=nil
          RSXP.call('read',payload:path.to_s).each_line do |line|
            line=line.strip
            if line.start_with?('[') && line.end_with?(']')
              current=line[1...-1]
            elsif current && current.casecmp?(section.to_s)
              name,entry=line.split('=',2)
              value=entry.strip if entry && name.strip.casecmp?(key.to_s)
            end
          end
        rescue StandardError
          # A missing INI file returns the caller's supplied default.
        end
        count=[Integer(capacity),buffer.bytesize].min
        return 0 if count<=0
        bytes=value.b.byteslice(0,count-1)
        memcpy_string(buffer,bytes+"\0")
        bytes.bytesize
      end
    end
    GetPrivateProfileStringA = GetPrivateProfileString
  end
end

def kappatalize(s)
	s[0] = s[0].upcase
	return s
end

