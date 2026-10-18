# SPDX-License-Identifier: GPL-2.0-or-later
require 'json'
require 'zlib'
require 'openssl'
require 'tmpdir'
require 'socket'
require 'fiddle'
raise 'Zlib runtime broken' unless Zlib::Inflate.inflate(Zlib::Deflate.deflate('RSXP')) == 'RSXP'
raise 'OpenSSL runtime broken' unless OpenSSL::Digest::SHA256.hexdigest('RSXP').size == 64
raise 'JSON runtime broken' unless JSON.parse(JSON.generate({'rsxp' => 1}))['rsxp'] == 1
load File.expand_path('../rust/testdata/rgss_smoke.rb', __dir__)
puts 'BUNDLED_RUNTIME_OK'
