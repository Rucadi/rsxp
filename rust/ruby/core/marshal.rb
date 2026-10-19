# rsxp aliases Marshal.load to force ASCII-8BIT strings to UTF-8. Game data
# saved by older RPG Maker tools has no encoding tag, so do the same here.
class << Marshal
  alias __rsxp_load_alias load unless method_defined?(:__rsxp_load_alias)
  def load(port, proc = nil)
    if proc
      __rsxp_load_alias(port, ->(object) { proc.call(RSXP.force_utf8(object)) })
    else
      __rsxp_load_alias(port, RSXP.method(:force_utf8))
    end
  end
end

MKXP = RSXP unless defined?(MKXP) # Compatibility constant for older games.

