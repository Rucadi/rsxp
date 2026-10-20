# SPDX-License-Identifier: GPL-2.0-or-later
# Keep YJIT as the default for game scripts. The launcher explicitly selects
# one engine; RSXP_YJIT remains supported for existing command-line workflows.
mode = ENV.fetch('RSXP_JIT') { ENV['RSXP_YJIT'] == '0' ? 'off' : 'yjit' }
case mode
when 'yjit', 'zjit'
  name = mode.upcase
  if RubyVM.const_defined?(name)
    RubyVM.const_get(name).enable
  else
    warn "#{name} is unavailable in this Ruby build; running without JIT"
  end
when 'off'
  # Neither engine is enabled by the embedded VM.
else
  raise ArgumentError, "Unknown RSXP_JIT mode: #{mode.inspect}"
end
