# fanplay aliases Marshal.load to force ASCII-8BIT strings to UTF-8. Game data
# saved by older RPG Maker tools has no encoding tag, so do the same here.
class << Marshal
  alias __fanplay_load_alias load unless method_defined?(:__fanplay_load_alias)
  def load(port, proc = nil)
    # The reader tags strings as UTF-8 as it creates them; wrapping every
    # object in a Ruby callback made large RGSS data loads several times slower.
    __fanplay_load_alias(port, proc)
  end
end

MKXP = FANPLAY unless defined?(MKXP) # Compatibility constant for older games.

