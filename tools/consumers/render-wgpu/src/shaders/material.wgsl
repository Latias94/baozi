struct Frame { view_projection: mat4x4<f32>, camera: vec4<f32>, counts: vec4<u32> }
struct Light { position_range: vec4<f32>, direction_kind: vec4<f32>, tint_intensity: vec4<f32>, cone_units: vec4<f32> }
struct BindingParams { row0_strength: vec4<f32>, row1_pad: vec4<f32>, select: vec4<u32> }
struct Material { base: vec4<f32>, emission_metallic: vec4<f32>, roughness_coat: vec4<f32>, sheen: vec4<f32>, flags: vec4<u32>, slots: array<BindingParams,10> }
struct Chunk { counts: vec4<u32>, uv_ordinals: vec4<u32> }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage,read> lights: array<Light>;
@group(0) @binding(2) var sheen_lut: texture_2d<f32>;
@group(0) @binding(3) var sheen_sampler: sampler;
@group(0) @binding(4) var<storage,read_write> fragment_failure: atomic<u32>;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var tex0: texture_2d<f32>;
@group(1) @binding(2) var samp0: sampler;
@group(1) @binding(3) var tex1: texture_2d<f32>;
@group(1) @binding(4) var samp1: sampler;
@group(1) @binding(5) var tex2: texture_2d<f32>;
@group(1) @binding(6) var samp2: sampler;
@group(1) @binding(7) var tex3: texture_2d<f32>;
@group(1) @binding(8) var samp3: sampler;
@group(1) @binding(9) var tex4: texture_2d<f32>;
@group(1) @binding(10) var samp4: sampler;
@group(1) @binding(11) var tex5: texture_2d<f32>;
@group(1) @binding(12) var samp5: sampler;
@group(1) @binding(13) var tex6: texture_2d<f32>;
@group(1) @binding(14) var samp6: sampler;
@group(1) @binding(15) var tex7: texture_2d<f32>;
@group(1) @binding(16) var samp7: sampler;
@group(1) @binding(17) var tex8: texture_2d<f32>;
@group(1) @binding(18) var samp8: sampler;
@group(1) @binding(19) var tex9: texture_2d<f32>;
@group(1) @binding(20) var samp9: sampler;
@group(2) @binding(0) var<uniform> chunk: Chunk;
@group(2) @binding(1) var<storage,read> uv_values: array<vec2<f32>>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // Five user locations leave room for FrontFacing within the requested eight.
    // Each UV pair retains full F32 precision and the same perspective interpolation.
    @location(3) uv01: vec4<f32>,
    @location(4) uv23: vec4<f32>,
}
fn vertex_uv(vertex: u32,ordinal: u32) -> vec2<f32> {
    if ordinal==0xffffffffu { return vec2(0.0); }
    return uv_values[vertex*chunk.counts.y+ordinal];
}
@vertex fn vs_main(@builtin(vertex_index) vertex: u32,@location(0) position: vec3<f32>,@location(1) normal: vec3<f32>,@location(2) tangent: vec4<f32>,@location(3) color: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip=frame.view_projection*vec4(position,1.0);
    // Tangent provenance remains in the vertex buffer; shading uses derivative frames.
    out.world=position;out.normal=normal;out.color=color;
    out.uv01=vec4(vertex_uv(vertex,chunk.uv_ordinals.x),vertex_uv(vertex,chunk.uv_ordinals.y));
    out.uv23=vec4(vertex_uv(vertex,chunk.uv_ordinals.z),vertex_uv(vertex,chunk.uv_ordinals.w));
    return out;
}
fn select_uv(v: VertexOutput,slot: u32) -> vec2<f32> {
    switch slot { case 0u: { return v.uv01.xy; } case 1u: { return v.uv01.zw; } case 2u: { return v.uv23.xy; } default: { return v.uv23.zw; } }
}
fn coordinate(v: VertexOutput,b: BindingParams) -> vec2<f32> {
    let uv=vec3(select_uv(v,b.select.y),1.0);
    return vec2(dot(b.row0_strength.xyz,uv),dot(b.row1_pad.xyz,uv));
}
fn covered(b: BindingParams,uv: vec2<f32>) -> bool {
    return (b.select.x&1u)!=0u && ((b.select.x&2u)==0u || (uv.x>=0.0 && uv.x<=1.0)) && ((b.select.x&4u)==0u || (uv.y>=0.0 && uv.y<=1.0));
}
fn channel(raw: vec4<f32>,code: u32) -> f32 {
    switch code { case 0u: { return raw.r; } case 1u: { return raw.g; } case 2u: { return raw.b; } case 3u: { return raw.a; } case 4u: { return 0.0; } default: { return 1.0; } }
}
fn swizzle(raw: vec4<f32>,b: BindingParams) -> vec4<f32> {
    let codes=b.select.z;
    return vec4(channel(raw,codes&15u),channel(raw,(codes>>4u)&15u),channel(raw,(codes>>8u)&15u),channel(raw,(codes>>12u)&15u));
}
fn finite3(v: vec3<f32>) -> bool { return all(abs(v)<=vec3(3.4028234663852886e38)); }
fn component_scale(v: vec3<f32>) -> f32 { return max(abs(v.x),max(abs(v.y),abs(v.z))); }
struct NormalResult { normal: vec3<f32>, failure: u32 }
fn mapped_normal(n: vec3<f32>,px: vec3<f32>,py: vec3<f32>,ux: vec2<f32>,uy: vec2<f32>,sample: vec4<f32>,b: BindingParams) -> NormalResult {
    let d=ux.x*uy.y-ux.y*uy.x;
    var result: NormalResult;
    result.normal=n;result.failure=0u;
    if abs(d)<1e-20 || !finite3(n) { result.failure=1u;return result; }
    let raw_t=(px*uy.y-py*ux.y)/d;
    let raw_b=(-px*uy.x+py*ux.x)/d;
    let projected=raw_t-n*dot(n,raw_t);
    if dot(projected,projected)<1e-20 || !finite3(projected) { result.failure=2u;return result; }
    let t=normalize(projected);
    let orientation=dot(cross(n,t),raw_b);
    if abs(orientation)<1e-20 { result.failure=3u;return result; }
    let bitangent=cross(n,t)*sign(orientation);
    var tangent_normal=sample.xyz*2.0-1.0;
    if b.select.w==2u { tangent_normal.z=sqrt(max(0.0,1.0-dot(tangent_normal.xy,tangent_normal.xy))); }
    tangent_normal.x*=b.row0_strength.w;tangent_normal.y*=b.row0_strength.w;
    let raw=t*tangent_normal.x+bitangent*tangent_normal.y+n*tangent_normal.z;
    if dot(raw,raw)<1e-20 || !finite3(raw) { result.failure=4u;return result; }
    result.normal=normalize(raw);return result;
}
const PI: f32=3.141592653589793;
fn ggx_visibility(nv: f32,nl: f32,roughness: f32) -> f32 {
    let a=pow(max(roughness,0.045),2.0);let a2=a*a;
    return 0.5/max(1e-20,nl*sqrt(nv*nv*(1.0-a2)+a2)+nv*sqrt(nl*nl*(1.0-a2)+a2));
}
fn ggx(nh: f32,roughness: f32) -> f32 {
    let a=pow(max(roughness,0.045),2.0);let a2=a*a;
    return a2/(PI*pow(nh*nh*(a2-1.0)+1.0,2.0));
}
fn mr(base: vec3<f32>,metallic: f32,roughness: f32,nv: f32,nl: f32,nh: f32,vh: f32) -> vec3<f32> {
    let f0=mix(vec3(0.04),base,metallic);
    let fresnel=f0+(1.0-f0)*pow(1.0-clamp(vh,0.0,1.0),5.0);
    return (1.0-metallic)*(1.0-fresnel)*base/PI+ggx(nh,roughness)*ggx_visibility(nv,nl,roughness)*fresnel;
}
fn sheen_l(x: f32,alpha: f32) -> f32 {
    let t=pow(1.0-alpha,2.0);
    let a=mix(21.5473,25.3245,t);let b=mix(3.82987,3.32435,t);let c=mix(0.19823,0.16801,t);
    let d=mix(-1.97760,-1.27393,t);let e=mix(-4.32054,-4.85967,t);
    return a/(1.0+b*pow(x,c))+d*x+e;
}
fn sheen_lambda(x: f32,alpha: f32) -> f32 {
    if x<0.5 { return exp(sheen_l(x,alpha)); }
    return exp(2.0*sheen_l(0.5,alpha)-sheen_l(1.0-x,alpha));
}
fn charlie(nv: f32,nl: f32,nh: f32,roughness: f32) -> f32 {
    // The consumer uses the same finite grazing domain as its energy lookup.
    let sv=clamp(nv,0.001,1.0);let sl=clamp(nl,0.001,1.0);
    let a=pow(max(roughness,0.045),2.0);
    let d=(2.0+1.0/a)*pow(max(0.0,1.0-nh*nh),0.5/a)/(2.0*PI);
    return d/max(1e-20,(1.0+sheen_lambda(sv,a)+sheen_lambda(sl,a))*4.0*sv*sl);
}
fn albedo(nv: f32,roughness: f32) -> f32 {
    let uv=(vec2((clamp(nv,0.001,1.0)-0.001)/0.999,(clamp(roughness,0.045,1.0)-0.045)/0.955)*31.0+0.5)/32.0;
    return textureSampleLevel(sheen_lut,sheen_sampler,uv,0.0).r;
}
fn sheen_layer_weights(ev: f32,el: f32,maximum_color: f32) -> vec2<f32> {
    // Explicit device approximation; the raw fitted energy texture is unchanged.
    let normalization=1.0/max(1.0,max(ev,el));
    let substrate=min(1.0-maximum_color*clamp(ev,0.0,1.0),1.0-maximum_color*clamp(el,0.0,1.0));
    return vec2(normalization,substrate);
}
struct FragmentOutput { @location(0) color: vec4<f32>, @location(1) failure: u32 }
fn finish(color: vec4<f32>,failure: u32) -> FragmentOutput {
    var result: FragmentOutput;result.color=color;result.failure=failure;
    if !all(abs(color)<=vec4(65504.0)) { result.failure|=64u; }
    if any(color.rgb<vec3(0.0)) { result.failure|=256u; }
    if result.failure!=0u { atomicOr(&fragment_failure,result.failure); }
    return result;
}
@fragment fn fs_main(v: VertexOutput,@builtin(front_facing) front: bool) -> FragmentOutput {
    let uv0=coordinate(v,material.slots[0]);let uv1=coordinate(v,material.slots[1]);
    let uv2=coordinate(v,material.slots[2]);let uv3=coordinate(v,material.slots[3]);
    let uv4=coordinate(v,material.slots[4]);let uv5=coordinate(v,material.slots[5]);
    let uv6=coordinate(v,material.slots[6]);let uv7=coordinate(v,material.slots[7]);
    let uv8=coordinate(v,material.slots[8]);let uv9=coordinate(v,material.slots[9]);
    // Sampling and derivatives precede every potentially nonuniform discard/lighting branch.
    let s0=swizzle(textureSample(tex0,samp0,uv0),material.slots[0]);
    let s1=swizzle(textureSample(tex1,samp1,uv1),material.slots[1]);
    let s2=swizzle(textureSample(tex2,samp2,uv2),material.slots[2]);
    let s3=swizzle(textureSample(tex3,samp3,uv3),material.slots[3]);
    let s4=swizzle(textureSample(tex4,samp4,uv4),material.slots[4]);
    let s5=swizzle(textureSample(tex5,samp5,uv5),material.slots[5]);
    let s6=swizzle(textureSample(tex6,samp6,uv6),material.slots[6]);
    let s7=swizzle(textureSample(tex7,samp7,uv7),material.slots[7]);
    let s8=swizzle(textureSample(tex8,samp8,uv8),material.slots[8]);
    let s9=swizzle(textureSample(tex9,samp9,uv9),material.slots[9]);
    let px=dpdx(v.world);let py=dpdy(v.world);
    let ux=dpdx(uv2);let uy=dpdy(uv2);let cx=dpdx(uv7);let cy=dpdy(uv7);
    var result: FragmentOutput;result.failure=0u;
    var base=material.base*v.color;
    if covered(material.slots[0],uv0) { base=vec4(base.rgb*s0.rgb*material.slots[0].row0_strength.w,base.a*s0.a); }
    if material.flags.y==1u && base.a<material.roughness_coat.w { discard; }
    if material.flags.x==1u {
        return finish(vec4(base.rgb,select(1.0,base.a,material.flags.y==2u)),0u);
    }
    if dot(v.normal,v.normal)<1e-20 || !finite3(v.normal) { return finish(vec4(0.0),5u); }
    let mesh_normal=normalize(v.normal);
    var n=mesh_normal;var nc=mesh_normal;
    if covered(material.slots[2],uv2) {
        let mapped=mapped_normal(mesh_normal,px,py,ux,uy,s2,material.slots[2]);n=mapped.normal;result.failure|=mapped.failure;
    }
    if covered(material.slots[7],uv7) {
        let mapped=mapped_normal(mesh_normal,px,py,cx,cy,s7,material.slots[7]);nc=mapped.normal;result.failure|=mapped.failure;
    }
    if !front { n=-n;nc=-nc; }
    var metallic=material.emission_metallic.w;var roughness=material.roughness_coat.x;
    if covered(material.slots[1],uv1) { metallic*=s1.x;roughness*=s1.y; }
    var emission=material.emission_metallic.rgb;
    if covered(material.slots[4],uv4) { emission*=s4.rgb*material.slots[4].row0_strength.w; }
    var coat=material.roughness_coat.y;var coat_roughness=material.roughness_coat.z;
    if covered(material.slots[5],uv5) { coat*=s5.x; }
    if covered(material.slots[6],uv6) { coat_roughness*=s6.x; }
    var sheen=material.sheen.rgb;var sheen_roughness=material.sheen.w;
    if covered(material.slots[8],uv8) { sheen*=s8.rgb*material.slots[8].row0_strength.w; }
    if covered(material.slots[9],uv9) { sheen_roughness*=s9.x; }
    if any(sheen>vec3(1.0)) || base.a<0.0 || base.a>1.0 { result.failure|=8u; }
    // AO is retained and sampled. This profile's indirect-light term is exactly zero.
    let ao=1.0+material.slots[3].row0_strength.w*(s3.x-1.0);
    var camera_delta=frame.camera.xyz;
    if frame.camera.w<0.5 { camera_delta-=v.world; }
    let camera_scale=component_scale(camera_delta);
    if !finite3(camera_delta) || camera_scale<1.17549435e-38 { return finish(vec4(0.0),result.failure|16u); }
    // Only the bounded vector is squared: direct normalize(delta) can overflow.
    let camera_unit=camera_delta/camera_scale;
    let camera_length=length(camera_unit);
    // The exact length is in [1,sqrt(3)]; allow a small F32 rounding margin.
    // A reciprocal-based division may flush at extreme scales: fail explicitly.
    if !finite3(camera_unit) || !(camera_length>=0.999999 && camera_length<=1.732052) { return finish(vec4(0.0),result.failure|16u); }
    let view=camera_unit/camera_length;
    let nv=max(dot(n,view),1e-5);let ncv=max(dot(nc,view),1e-5);
    let wc=coat*(0.04+0.96*pow(1.0-abs(dot(nc,view)),5.0));
    var color=(1.0-wc)*emission;
    for (var i=0u;i<frame.counts.x;i++) {
        let light=lights[i];
        var l=-light.direction_kind.xyz;var attenuation=1.0;
        if light.direction_kind.w>0.5 {
            let delta=light.position_range.xyz-v.world;
            let distance_scale=component_scale(delta);
            if !finite3(delta) || (distance_scale==0.0 && any(light.position_range.xyz!=v.world))
                || (distance_scale>0.0 && distance_scale<1.17549435e-38) { result.failure|=512u;continue; }
            // This component bound already establishes range exclusion. Skip
            // normalization and inverse-square arithmetic for its exact zero.
            if light.cone_units.z>0.5 && distance_scale>=light.position_range.w { continue; }
            var unit_length=1.0;
            if distance_scale>0.0 {
                let unit_delta=delta/distance_scale;
                unit_length=length(unit_delta);
                if !finite3(unit_delta) || !(unit_length>=0.999999 && unit_length<=1.732052) { result.failure|=512u;continue; }
                l=unit_delta/unit_length;
            }
            var cutoff=1.0;
            if light.cone_units.z>0.5 {
                // A reciprocal of a huge normal range can be subnormal and
                // flush to zero. Common power-of-two scaling keeps the divisor
                // in [1,2^64] without changing a representable range fraction.
                let range_scale=select(1.0,5.421010862427522e-20,light.position_range.w>18446744073709551616.0);
                let ratio=((distance_scale*range_scale)/(light.position_range.w*range_scale))*unit_length;
                cutoff=pow(clamp(1.0-pow(ratio,4.0),0.0,1.0),2.0);
            }
            attenuation=0.0;
            if cutoff>0.0 {
                // Convert the component scale to physical meters before forming distance.
                let physical_distance=(distance_scale*light.cone_units.w)*unit_length;
                // Above 2^63 meters the inverse-square result would be a native-profile
                // subnormal or zero. Report that derived operation instead of black success.
                if !(physical_distance>=0.0 && physical_distance<=9.223372036854776e18) { result.failure|=512u;continue; }
                let inverse_meters=1.0/max(physical_distance,0.001);
                attenuation=(inverse_meters*inverse_meters)*cutoff;
                if !(attenuation>=1.17549435e-38 && attenuation<=1000000.0) { result.failure|=512u;continue; }
            }
            if light.direction_kind.w>1.5 {
                let cosine=dot(-l,light.direction_kind.xyz);
                if light.cone_units.x==light.cone_units.y { attenuation*=select(0.0,1.0,cosine>=light.cone_units.y); }
                else { attenuation*=pow(clamp((cosine-light.cone_units.y)/(light.cone_units.x-light.cone_units.y),0.0,1.0),2.0); }
            }
        }
        let nl=max(dot(n,l),0.0);let ncl=max(dot(nc,l),0.0);
        let half_raw=view+l;
        if dot(half_raw,half_raw)<1e-20 { continue; }
        let h=normalize(half_raw);
        var substrate=vec3(0.0);
        if nl>0.0 {
            substrate=mr(base.rgb,metallic,roughness,nv,nl,max(dot(n,h),0.0),max(dot(view,h),0.0));
            let maximum=max(sheen.r,max(sheen.g,sheen.b));
            if maximum>0.0 {
                let ev=albedo(nv,sheen_roughness);let el=albedo(nl,sheen_roughness);
                if !(ev>=0.0 && ev<=65504.0 && el>=0.0 && el<=65504.0) { result.failure|=128u; }
                let weights=sheen_layer_weights(ev,el,maximum);
                substrate=substrate*weights.y+sheen*charlie(nv,nl,max(dot(n,h),0.0),sheen_roughness)*weights.x;
            }
        }
        var coat_lobe=0.0;
        if ncl>0.0 { coat_lobe=ggx(max(dot(nc,h),0.0),coat_roughness)*ggx_visibility(ncv,ncl,coat_roughness); }
        let incident=light.tint_intensity.rgb*light.tint_intensity.w*attenuation;
        color+=((1.0-wc)*substrate*nl+vec3(wc*coat_lobe*ncl))*incident;
    }
    if !finite3(color) { result.failure|=32u; }
    return finish(vec4(color,select(1.0,base.a,material.flags.y==2u)),result.failure);
}
