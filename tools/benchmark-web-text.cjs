// SPDX-License-Identifier: GPL-2.0-or-later
// Measure the real WASM host's text-measurement and text-drawing commands.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),{createHash}=require('node:crypto');
const engine=require(path.resolve(process.argv[2] || 'target/ruby-wasm/check-engine/web_check.js'));
globalThis.__fanplay_check_read=()=>undefined;
engine.init();
const call=(header,text='Opciones: Volumen Música')=>engine.native_call(Buffer.from(header+'\n\0'+text));
const id=Buffer.from(engine.native_call(Buffer.from('bitmap_new\t512\t64\n'))).toString();
const out={};
for(const [name,header] of [['measure','text_size\t22\t0\t0'],['draw',`bitmap_text\t${id}\t0\t0\t512\t64\t0\t22\t0\t0\t240\t30\t20\t255\t0\t0\t0\t0\t0\t255`]]){
 const start=performance.now();for(let i=0;i<50;i++)call(header);out[name+'_ms_per_call']=(performance.now()-start)/50;
}
const sizes=call('text_size\t22\t0\t0');out.measurement=Buffer.from(sizes).toString();
const snapshot=engine.native_call(Buffer.from(`bitmap_get\t${id}\t3\t30\n`));
out.pixel=Buffer.from(snapshot).toString();
const hash=createHash("sha256");for(let y=0;y<64;y++)for(let x=0;x<512;x++)hash.update(Buffer.from(engine.native_call(Buffer.from(`bitmap_get\t${id}\t${x}\t${y}\n`))));out.pixel_sha256=hash.digest("hex");
assert.equal(out.measurement, '268\t25');
assert.equal(out.pixel_sha256, '4a4ddc2737a2e2af8561d18f7a211791e08e4641e29cea97e2532db47785d429');
console.log(JSON.stringify(out,null,2));
