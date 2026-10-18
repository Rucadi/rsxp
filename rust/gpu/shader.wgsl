// Packed ARGB composition deliberately uses integer source-over, matching the
// software renderer rather than relying on premultiplied hardware blending.
struct Params { a: array<vec4<u32>,16> }
@group(0) @binding(0) var<storage,read> atlas:array<u32>;
@group(0) @binding(1) var<storage,read_write> frame:array<u32>;
@group(0) @binding(2) var<storage,read> commands:array<Params>;
@group(0) @binding(3) var<uniform> batch_start:vec4<u32>;
@group(0) @binding(4) var<storage,read> tile_geometry:array<u32>;
fn f(p:Params,i:u32)->vec4<f32> {return bitcast<vec4<f32>>(p.a[i]);}
fn unpack(c:u32)->vec4<u32> {return vec4((c>>16u)&255u,(c>>8u)&255u,c&255u,c>>24u);}
fn pack(c:vec4<u32>)->u32 {return (c.a<<24u)|(c.r<<16u)|(c.g<<8u)|c.b;}
fn quantize(c:vec4<f32>)->vec4<u32> {return vec4<u32>(clamp(round(c*255.0),vec4(0.0),vec4(255.0)));}
fn compose(p:Params,dst:vec4<u32>,src:vec4<u32>)->vec4<u32> {
 return compose_mode(p.a[10].x,dst,src);
}
fn compose_mode(blend:u32,dst:vec4<u32>,src:vec4<u32>)->vec4<u32> {
 if blend==1u {return vec4(min(dst.rgb+src.rgb*src.a/255u,vec3(255u)),min(dst.a+src.a,255u));}
 if blend==2u {return vec4(vec3<u32>(max(vec3<i32>(dst.rgb)-vec3<i32>(src.rgb*src.a/255u),vec3(0))),min(dst.a+src.a,255u));}
 let back=dst.a*(255u-src.a)/255u;
 let alpha=src.a+back;
 if alpha==0u {return vec4(0u);}
 return vec4((src.rgb*src.a+dst.rgb*back)/alpha,alpha);
}
fn effects(p:Params,color:vec4<f32>)->vec4<f32> {
 var rgb=color.rgb;
 let tone=f(p,7u);
 let gray=dot(rgb,vec3(0.299,0.587,0.114));
 rgb=mix(rgb,vec3(gray),tone.a)+tone.rgb;
 rgb=mix(rgb,f(p,6u).rgb,f(p,6u).a);
 if p.a[10].y!=0u {rgb=vec3(1.0)-rgb;}
 return vec4(rgb,color.a*f(p,11u).x);
}
// Each invocation owns one destination pixel. Native tile layers are walked
// in scene order within that invocation, including overlapping alpha tiles.
fn draw(p:Params,xy:vec2<i32>) {
 let bounds=bitcast<vec4<i32>>(p.a[1]);
 if any(xy<bounds.xy) || any(xy>=bounds.zw) {return;}
 let index=u32(xy.y)*p.a[2].x+u32(xy.x)+p.a[0].y;
 let operation=p.a[0].x;
 if operation==0u {frame[index]=p.a[3].x;return;}
 if operation==3u {
  let c=unpack(frame[index]); frame[index]=pack(vec4(c.rgb*p.a[3].x/255u,c.a)); return;
 }
 if operation==2u {
  let source=unpack(frame[u32(xy.y)*p.a[2].x+u32(xy.x)+p.a[0].z]);
  frame[index]=pack(compose(p,unpack(frame[index]),quantize(effects(p,vec4<f32>(source)/255.0))));return;
 }
 let mode=p.a[10].w;
 let area=bitcast<vec4<i32>>(p.a[3]);
 var point=vec2<f32>(xy);
 if mode==1u {point+=vec2(0.5);}
 let delta=point-f(p,5u).xy;
 var local=vec2(dot(f(p,4u).xy,delta),dot(f(p,4u).zw,delta));
 if mode==0u {
  let d=xy-vec2<i32>(f(p,5u).xy);
  local=vec2<f32>(vec2<u32>(d)*vec2<u32>(area.zw)/p.a[13].xy);
 }
 if mode==2u {
  let tile=f(p,5u).zw;
  let origin=f(p,5u).xy+floor(delta/tile)*tile;
  local=floor((point-origin)*vec2<f32>(p.a[2].zw)/tile);
 }
 if any(local<vec2(0.0)) || any(local>=f(p,5u).zw) && mode!=2u {return;}
 var sample=vec2<i32>(floor(local));
 if p.a[10].z!=0u {sample.x=area.z-1-sample.x;}
 sample+=area.xy;
 if any(sample<vec2(0)) || any(sample>=vec2<i32>(p.a[2].zw)) {return;}
 let source=unpack(atlas[p.a[0].z+u32(sample.y)*p.a[2].z+u32(sample.x)]);
 var shaded=vec4<f32>(source)/255.0;
 if p.a[9].x!=0u && f(p,11u).y>0.0 {
  let pat=vec2<f32>(p.a[9].xy);
  let sz=vec2<f32>(p.a[2].zw);
  var uv=vec2<f32>(sample)/(pat*f(p,8u).xy)-f(p,8u).zw*sz/(pat*pat);
  if p.a[9].z==0u {uv=vec2<f32>(sample)/(sz*f(p,8u).xy)-f(p,8u).zw/pat;}
  let coord=vec2<u32>(floor(fract(uv)*pat));
  let pattern=vec4<f32>(unpack(atlas[p.a[0].w+coord.y*p.a[9].x+coord.x]))/255.0;
  let opacity=pattern.a*f(p,11u).y;
  var rgb=pattern.rgb;
  if p.a[9].w==1u {rgb=min(shaded.rgb+rgb,vec3(1.0));}
  if p.a[9].w==2u {rgb=max(shaded.rgb-rgb,vec3(0.0));}
  shaded=vec4(mix(shaded.rgb,rgb,opacity),shaded.a);
 }
 if p.a[12].y!=0u && sample.y>=bitcast<i32>(p.a[12].z) {shaded.a*=f32(p.a[12].w)/255.0;}
 var output=quantize(effects(p,shaded));
 if f(p,11u).w==1.0 {output.a=source.a*p.a[12].x/255u;}
 frame[index]=pack(compose(p,unpack(frame[index]),output));
}

// Compact native quad: bounds, destination, atlas cell, blend, color/tone.
// No temporary full sprite Params array or pattern/rotation branches.
fn tile_vec(base:u32)->vec4<u32> {
 return vec4(tile_geometry[base],tile_geometry[base+1u],tile_geometry[base+2u],tile_geometry[base+3u]);
}
fn draw_tile(base:u32,xy:vec2<i32>) {
 let bounds=bitcast<vec4<i32>>(tile_vec(base));
 if any(xy<bounds.xy) || any(xy>=bounds.zw) {return;}
 let destination=tile_vec(base+4u);
 let cell=tile_vec(base+8u);
 let blend=tile_vec(base+12u);
 let delta=vec2<u32>(xy-bitcast<vec2<i32>>(destination.xy));
 var sample=delta*cell.yz/destination.zw;
 if any(sample>=cell.yz) {return;}
 if (blend.w&1u)!=0u {sample.x=cell.y-1u-sample.x;}
 let source=unpack(atlas[cell.x+sample.y*cell.y+sample.x]);
 var output=source;
 if (blend.w&4u)!=0u {
  output.a=source.a*cell.w/255u;
 } else {
  let color=bitcast<vec4<f32>>(tile_vec(base+16u));
  let tone=bitcast<vec4<f32>>(tile_vec(base+20u));
  var rgb=vec3<f32>(source.rgb)/255.0;
  let gray=dot(rgb,vec3(0.299,0.587,0.114));
  rgb=mix(rgb,vec3(gray),tone.a)+tone.rgb;
  rgb=mix(rgb,color.rgb,color.a);
  if (blend.w&2u)!=0u {rgb=vec3(1.0)-rgb;}
  output=quantize(vec4(rgb,f32(source.a)/255.0*(f32(cell.w)/255.0)));
 }
 let index=u32(xy.y)*blend.y+u32(xy.x)+blend.x;
 frame[index]=pack(compose_mode(blend.z,unpack(frame[index]),output));
}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
 let p=commands[batch_start.x+gid.z];
 let bounds=bitcast<vec4<i32>>(p.a[1]);
 let xy=vec2<i32>(gid.xy)+bounds.xy;
 if any(xy>=bounds.zw) {return;}
 if p.a[0].x==4u {
  let bin=p.a[15].x+(u32(xy.y)/p.a[15].z-p.a[14].y)*p.a[15].y+u32(xy.x)/p.a[15].z-p.a[14].x;
  let list=tile_geometry[bin];
  let count=tile_geometry[list];
  for(var i=0u;i<count;i++) {
   let base=tile_geometry[list+1u+i];
   draw_tile(base,xy);
  }
  return;
 }
 draw(p,xy);
}
