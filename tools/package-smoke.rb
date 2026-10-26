# SPDX-License-Identifier: GPL-2.0-or-later
require 'json'
require 'zlib'
require 'openssl'
require 'tmpdir'
require 'socket'
require 'fiddle'
raise 'Zlib runtime broken' unless Zlib::Inflate.inflate(Zlib::Deflate.deflate('FANPLAY')) == 'FANPLAY'
raise 'OpenSSL runtime broken' unless OpenSSL::Digest::SHA256.hexdigest('FANPLAY').size == 64
raise 'JSON runtime broken' unless JSON.parse(JSON.generate({'fanplay' => 1}))['fanplay'] == 1
load File.expand_path('../crates/fanplay-rgss/testdata/rgss_smoke.rb', __dir__)
puts 'BUNDLED_RUNTIME_OK'
