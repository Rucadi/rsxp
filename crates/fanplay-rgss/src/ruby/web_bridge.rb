# SPDX-License-Identifier: GPL-2.0-or-later
# Small browser-only adapter for CRuby built by ruby.wasm.
#
# Ruby values cross the JS boundary as base64 strings so arbitrary game data
# (Marshal, compressed scripts, images and audio) never goes through a UTF-8
# conversion. The Rust callback owns the actual RGSS engine and archive reads.
require 'js'

module FANPLAY
  # bootstrap.rb invokes this as an unqualified call from a FANPLAY singleton
  # method, so the browser hook must be a FANPLAY singleton method rather than
  # a private Object method.
  def self.__fanplay_native(request)
    encoded = [request].pack('m0')
    # JS::Object#call takes a method name, unlike Ruby Proc#call.
    result = JS.global.call(:__fanplay_native_b64, encoded).to_s
    raise FANPLAYStop if result == '__FANPLAY_STOP__'
    result.unpack1('m0')
  end

  class << self
    alias __mkxp_native __fanplay_native
  end
end

# The browser host must return to eframe after each native Graphics.update.
# Ruby's Fiber gives the existing RGSS script loop cooperative scheduling with
# no per-frame JavaScript game-loop implementation.
module FANPLAY
  def self.startup_yield
    Fiber.yield
  end
end
