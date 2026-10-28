# frozen_string_literal: true

# The extension is statically linked by rbwasm. Requiring this file keeps the
# gem useful from Ruby code while the RGSS web bridge requires the extension by
# its native name during the browser boot sequence.
require "fanplay_native"
