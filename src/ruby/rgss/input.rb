module Input
  @state={}
  def self.__button(scan, flag) = (@state.fetch(scan,0) & flag)!=0
  # rsxp exposes the raw SDL scancode table (one boolean per scancode); the
  # Win32API GetKeyState/GetAsyncKeyState shim reads it.
  def self.raw_key_states
    states=Array.new(512,false)
    @state.each { |scan,flags| states[scan]=(flags&1)!=0 if scan>=0 && scan<512 }
    states
  end
  def self.raw_keys(key)
    return [Integer(key)] unless key.is_a?(Symbol)
    name=key.to_s
    return [name.ord-'A'.ord+4] if /\A[A-Z]\z/.match?(name)
    return [58+name[1..].to_i-1] if /\AF(?:[1-9]|1[0-2])\z/.match?(name)
    code={RETURN:40,ESCAPE:41,BACKSPACE:42,TAB:43,SPACE:44,GRAVE:53,UP:82,DOWN:81,LEFT:80,RIGHT:79,LSHIFT:225,RSHIFT:229,LCTRL:224,RCTRL:228,LALT:226,RALT:230}[key]
    # rsxp resolves unknown scancode names to 0 (SDL_SCANCODE_UNKNOWN), which
    # is never held, rather than raising.
    [code || 0]
  end
  def self.pressex?(key) = raw_keys(key).any? { |scan| __button(scan,1) }
  def self.triggerex?(key) = raw_keys(key).any? { |scan| __button(scan,2) }
  def self.releaseex?(key) = raw_keys(key).any? { |scan| __button(scan,4) }
  def self.repeatex?(key) = raw_keys(key).any? { |scan| __button(scan,8) }
  MOUSELEFT=1001;MOUSEMIDDLE=1002;MOUSERIGHT=1003;MOUSEX1=1004;MOUSEX2=1005
  DOWN=2; LEFT=4; RIGHT=6; UP=8
  A=11; B=12; C=13; X=14; Y=15; Z=16; L=17; R=18
  SHIFT=21; CTRL=22; ALT=23; F5=25; F6=26; F7=27; F8=28; F9=29
  KEYS={DOWN=>[81],LEFT=>[80],RIGHT=>[79],UP=>[82],A=>[225,229],B=>[27,41],C=>[29,40,44],X=>[4],Y=>[22],Z=>[7],L=>[20,75],R=>[26,78],SHIFT=>[225,229],CTRL=>[224,228],ALT=>[226,230],F5=>[62],F6=>[63],F7=>[64],F8=>[65],F9=>[66],MOUSELEFT=>[-1],MOUSEMIDDLE=>[-2],MOUSERIGHT=>[-3],MOUSEX1=>[-4],MOUSEX2=>[-5]}.freeze
  # Virtual controller actions are independent of physical keyboard state.
  [DOWN,LEFT,RIGHT,UP,A,B,C,X,Y,Z,L,R].each_with_index { |key,i| KEYS[key] << 600+i }
  @held=Hash.new(0)
  def self.code(value)
    # Legacy plugins use nil to disable an optional action binding.
    return 0 if value.nil?
    value.is_a?(Symbol) ? const_get(value) : Integer(value)
  end
  def self.update
    RSXP::GameAPI.poll if defined?(RSXP::GameAPI)
    bytes=RSXP.call('input_poll')
    @state=bytes.unpack('l<C'*(bytes.bytesize/5)).each_slice(2).to_h
    RSXP::GameAPI.inject_input(@state) if defined?(RSXP::GameAPI)
    KEYS.each_key { |key| @held[key]=press?(key) ? @held[key]+1 : 0 }
    RuntimePatches.sync
    nil
  end
  def self.press?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,1) }
  def self.trigger?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,2) }
  def self.release?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,4) }
  def self.time?(key) = @held[code(key)]*1_000_000/Graphics.frame_rate
  def self.mouse_x = RSXP.call('input_mouse').split("\t")[0].to_i
  def self.mouse_y = RSXP.call('input_mouse').split("\t")[1].to_i
  def self.mouse_in_window? = RSXP.call('input_mouse').split("\t")[2]=='1'
  def self.scroll_v = RSXP.call('input_mouse').split("\t")[3].to_i
  def self.text_input = RSXP.call('input_text_mode')=='1'
  def self.text_input=(value)
    RSXP.call('input_text_mode_set',value ? 1 : 0)
  end
  def self.gets = RSXP.call('input_gets')
  def self.repeat?(key)
    count=@held[code(key)]
    count==1 || (count>=24 && count%6==0)
  end
  def self.dir4
    [DOWN,LEFT,RIGHT,UP].find { |key| press?(key) } || 0
  end
  def self.dir8
    x=(press?(RIGHT) ? 1 : 0)-(press?(LEFT) ? 1 : 0)
    y=(press?(DOWN) ? 1 : 0)-(press?(UP) ? 1 : 0)
    x==0 && y==0 ? 0 : 5+x-y*3
  end
  # OS clipboard integration is unavailable; keep an in-process buffer so
  # scripts that copy/paste inside the game still work.
  def self.clipboard = (@clipboard || ''.dup)
  def self.clipboard=(value)
    @clipboard = value.nil? ? nil : value.to_s
  end
  # Hosts publish controller presence alongside virtual action scancodes.
  def self.connected?(index = 0) = (0...16).cover?(index) && __button(620+index,1)
  def self.power_level(_index = 0) = 0
  def self.axes_left(_index = 0) = 0
  def self.axes_right(_index = 0) = 0
  def self.axes_trigger(_index = 0) = 0
  def self.raw_axes(_index = 0) = 0
  def self.raw_button_states(_index = 0) = 0
  def self.count = (0...16).count { |index| connected?(index) }
  module Controller
    def self.connected?(index = 0) = Input.connected?(index)
    def self.name(index = 0) = connected?(index) ? "Game controller" : nil
    def self.power_level(_index = 0) = 0
    def self.axes_left(_index = 0) = 0
    def self.axes_right(_index = 0) = 0
    def self.axes_trigger(_index = 0) = 0
    def self.raw_axes(_index = 0) = 0
    def self.raw_button_states(_index = 0) = 0
    def self.count = Input.count
  end
end

