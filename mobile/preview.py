#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Develop the shared interface on macOS with native Rust and MRI Ruby."""
import argparse
import ctypes
from http.server import HTTPServer, SimpleHTTPRequestHandler
import json
from pathlib import Path
import sys

MOBILE=Path(__file__).resolve().parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port',type=int,default=8765)
args=parser.parse_args()
extension='dylib' if sys.platform=='darwin' else 'so'
library=ctypes.CDLL(str(MOBILE/f'core/target/release/librsxp_mobile.{extension}'))
library.rsxp_mobile_configure.argtypes=[ctypes.c_char_p,ctypes.c_char_p]
library.rsxp_mobile_request.argtypes=[ctypes.c_char_p]
library.rsxp_mobile_request.restype=ctypes.c_void_p
library.rsxp_mobile_free.argtypes=[ctypes.c_void_p]
games=MOBILE/'build/preview-games'
assert library.rsxp_mobile_configure(str(games).encode(),str(MOBILE/"build/cruby-native/CRuby").encode())==1
BRIDGE=b'''<script>
window.prompt = text => {
  if (!text.startsWith('rsxp:')) return null;
  const request=JSON.parse(text.slice(5));
  if(request.op==='import') return JSON.stringify({error:'Copy a game folder into mobile/build/preview-games, then press Refresh. iOS and Android use the native folder picker.'});
  const xhr=new XMLHttpRequest(); xhr.open('POST','/native',false);
  xhr.setRequestHeader('Content-Type','application/json'); xhr.send(text.slice(5));
  return xhr.responseText;
};
</script>'''
class Handler(SimpleHTTPRequestHandler):
    def __init__(self,*a,**kw): super().__init__(*a,directory=str(MOBILE/'build/ui'),**kw)
    def do_GET(self):
        if self.path.split('?')[0] in ('/','/index.html'):
            html=(MOBILE/'build/ui/index.html').read_bytes().replace(b'<script type="module"',BRIDGE+b'<script type="module"',1)
            self.send_response(200); self.send_header('Content-Type','text/html'); self.send_header('Content-Length',str(len(html))); self.end_headers(); self.wfile.write(html)
        else: super().do_GET()
    def do_POST(self):
        if self.path!='/native': self.send_error(404); return
        if self.headers.get('Origin') not in (None,f'http://127.0.0.1:{args.port}',f'http://localhost:{args.port}'):
            self.send_error(403); return
        length=int(self.headers.get('Content-Length','0'))
        if length>32*1024*1024: self.send_error(413); return
        pointer=library.rsxp_mobile_request(self.rfile.read(length))
        try: data=ctypes.string_at(pointer)
        finally: library.rsxp_mobile_free(pointer)
        self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def log_message(self,*args): pass
# One native engine per thread: keep all HTTP/ABI calls on this thread.
print(f'Native mobile preview: http://127.0.0.1:{args.port}',flush=True)
HTTPServer(('127.0.0.1',args.port),Handler).serve_forever()
