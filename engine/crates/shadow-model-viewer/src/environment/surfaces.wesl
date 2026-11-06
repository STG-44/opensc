import bevy_pbr::render::{pbr_fragment::pbr_input_from_standard_material,pbr_functions::alpha_discard};
@if(PREPASS_PIPELINE)
import bevy_pbr::{prepass::io::{VertexOutput,FragmentOutput},deferred::functions::deferred_output};
@else
import bevy_pbr::render::{forward_io::{VertexOutput,FragmentOutput},pbr_functions::{apply_pbr_lighting,main_pass_post_lighting_processing}};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> weather:vec4<f32>;
fn grain(p:vec3<f32>)->f32 {
    return fract(sin(dot(p,vec3(12.9898,78.233,43.157)))*43758.5453);
}
@fragment
fn fragment(mesh:VertexOutput,@builtin(front_facing) front:bool)->FragmentOutput {
    var pbr=pbr_input_from_standard_material(mesh,front);
    pbr.material.base_color=alpha_discard(pbr.material.flags,pbr.material.alpha_cutoff,pbr.material.base_color);
    let up=clamp(pbr.world_normal.y,0.0,1.0);
    let large=grain(floor(mesh.world_position.xyz*0.38));
    let fine=grain(floor(mesh.world_position.xyz*130.0));
    let wet=weather.x*smoothstep(0.0,0.65,up);
    let snow=weather.y*smoothstep(0.34,0.85,up)*smoothstep(large*0.5,0.5+large*0.5,weather.y);
    let puddle=wet*smoothstep(0.4,0.85,large);
    pbr.material.base_color=vec4(mix(pbr.material.base_color.rgb*(1.0-wet*0.22),vec3(0.84,0.90,0.96)*(0.93+fine*0.07),snow),pbr.material.base_color.a);
    pbr.material.perceptual_roughness=mix(mix(pbr.material.perceptual_roughness,0.15,puddle),0.78+fine*0.16,snow);
    pbr.material.metallic*=1.0-snow;
    pbr.material.clearcoat=max(pbr.material.clearcoat,wet*0.4*(1.0-snow));
    var out:FragmentOutput;
    @if(PREPASS_PIPELINE) { out=deferred_output(mesh,pbr); }
    @else { out.color=apply_pbr_lighting(pbr);out.color=main_pass_post_lighting_processing(pbr,out.color); }
    return out;
}
