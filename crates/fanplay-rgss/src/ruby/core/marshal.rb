# fanplay aliases Marshal.load to force ASCII-8BIT strings to UTF-8. Game data
# saved by older RPG Maker tools has no encoding tag, so do the same here.
class << Marshal
  alias __fanplay_load_alias load unless method_defined?(:__fanplay_load_alias)
  def load(port, proc = nil)
    if proc
      __fanplay_load_alias(port, ->(object) { proc.call(FANPLAY.force_utf8(object)) })
    else
      __fanplay_load_alias(port, FANPLAY.method(:force_utf8))
    end
  end
end

MKXP = FANPLAY unless defined?(MKXP) # Compatibility constant for older games.

