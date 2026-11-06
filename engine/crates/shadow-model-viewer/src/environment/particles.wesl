import bevy_pbr::render::forward_io::VertexOutput;
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> settings:vec4<f32>;
@fragment
fn fragment(mesh:VertexOutput)->@location(0) vec4<f32> {
    let p=mesh.uv*2.0-vec2(1.0);let radius=length(p);
    var alpha=0.0;var color=mesh.color.rgb;
    if settings.x<0.5 {
        alpha=pow(max(0.0,1.0-abs(p.x)),1.5)*(1.0-p.y*p.y);
    } else if settings.x<1.5 {
        let angle=atan2(p.y,p.x);let arms=0.72+0.28*pow(abs(cos(angle*3.0)),5.0);
        alpha=(1.0-smoothstep(0.2,arms,radius));
    } else if settings.x<2.5 {
        alpha=1.0-smoothstep(0.65,1.0,radius);
        color*=0.5+0.5*sqrt(max(0.0,1.0-radius*radius));
    } else {
        alpha=exp(-radius*radius*3.5)*(1.0-smoothstep(0.6,1.0,radius));
    }
    return vec4(color,mesh.color.a*alpha);
}
