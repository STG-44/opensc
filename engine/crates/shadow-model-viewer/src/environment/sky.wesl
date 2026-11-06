import bevy_pbr::render::forward_io::VertexOutput;

struct Sky {
    camera_flash:vec4<f32>, sun_day:vec4<f32>, cloud:vec4<f32>,
    wind_time:vec4<f32>, settings:vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky:Sky;

fn hash(p:vec3<f32>)->f32 {
    var q=fract(p*0.1031); q+=vec3(dot(q,q.yzx+vec3(33.33))); return fract((q.x+q.y)*q.z);
}
fn noise(p:vec3<f32>)->f32 {
    let i=floor(p);let f=fract(p);let u=f*f*(3.0-2.0*f);
    return mix(mix(mix(hash(i),hash(i+vec3(1.0,0.0,0.0)),u.x),mix(hash(i+vec3(0.0,1.0,0.0)),hash(i+vec3(1.0,1.0,0.0)),u.x),u.y),mix(mix(hash(i+vec3(0.0,0.0,1.0)),hash(i+vec3(1.0,0.0,1.0)),u.x),mix(hash(i+vec3(0.0,1.0,1.0)),hash(i+vec3(1.0,1.0,1.0)),u.x),u.y),u.z);
}
fn fbm(p:vec3<f32>)->f32 {
    return noise(p)*0.57+noise(p*2.03+vec3(17.1))*0.28+noise(p*4.07+vec3(41.7))*0.15;
}
fn density(p:vec3<f32>,height:f32)->f32 {
    let body=fbm(p);let detail=noise(p*7.0);
    let threshold=mix(0.83,0.24,sky.cloud.x);
    let shape=smoothstep(threshold,threshold+0.21,body-detail*0.045);
    return shape*smoothstep(0.0,0.18,height)*(1.0-smoothstep(0.64,1.0,height));
}
@fragment
fn fragment(mesh:VertexOutput)->@location(0) vec4<f32> {
    let ray=normalize(mesh.world_position.xyz-sky.camera_flash.xyz);
    let sun=normalize(sky.sun_day.xyz);let day=sky.sun_day.w;let night=1.0-day;
    let horizon=pow(1.0-max(ray.y,0.0),3.0);
    let sunset=(1.0-smoothstep(0.08,0.42,abs(sun.y)))*day;
    var background=mix(vec3(0.045,0.23,0.53),vec3(0.60,0.74,0.85),horizon);
    background=mix(background,vec3(0.9,0.38,0.12),sunset*horizon*0.7);
    background*=0.015+day*0.985;
    let sun_dot=dot(ray,sun);
    background+=vec3(1.0,0.77,0.45)*pow(max(sun_dot,0.0),90.0)*day*0.5;
    background+=vec3(12.0,10.0,6.0)*smoothstep(0.99998,0.99999,sun_dot)*day;
    background=mix(background,vec3(0.34,0.22,0.10)*max(day,0.04),sky.cloud.y*0.8);
    let star_cell=floor(ray*1400.0);let star_seed=hash(star_cell);
    let star=pow(max(0.0,(star_seed-0.9987)/0.0013),4.0)*night*smoothstep(0.02,0.2,ray.y);
    let moon_dot=dot(ray,-sun);
    let moon=smoothstep(0.99995,0.99997,moon_dot)*night;
    let halo=pow(max(moon_dot,0.0),1200.0)*night*0.012;
    let extras=vec3(0.64,0.76,1.0)*(star*1.5+moon*2.0+halo);
    let steps=u32(clamp(sky.settings.x,2.0,16.0));
    var transmission=1.0;var radiance=vec3(0.0);
    if ray.y>0.005 && sky.cloud.x>0.01 {
        for(var i=0u;i<steps;i+=1u) {
            let height=(f32(i)+0.5)/f32(steps);
            let altitude=700.0+height*850.0;
            let distance=altitude/max(ray.y,0.04);
            let position=(sky.camera_flash.xyz+ray*distance)*0.0012+vec3(sky.wind_time.x,0.0,sky.wind_time.y)*sky.wind_time.z*0.000025;
            let d=density(position,height);
            let optical=d*7.5/f32(steps);
            let absorbed=1.0-exp(-optical);
            let shade=exp(-density(position+sun*0.2,height+sun.y*0.15)*2.0);
            let silver=pow(max(sun_dot,0.0),8.0)*shade;
            var color=mix(vec3(0.075,0.09,0.12),vec3(0.80,0.85,0.91),shade*0.65+silver*0.35);
            color*=mix(0.45,1.0,sky.cloud.z)*day+0.025*night;
            color+=vec3(0.5,0.24,0.08)*sunset*shade;
            color+=vec3(0.7,0.78,1.0)*sky.camera_flash.w*2.3;
            radiance+=transmission*absorbed*color;
            transmission*=1.0-absorbed;
        }
    }
    let horizon_fade=smoothstep(0.005,0.09,ray.y);
    transmission=mix(1.0,transmission,horizon_fade);radiance*=horizon_fade;
    let low=sky.settings.y;
    let extra_alpha=clamp(star+moon+halo,0.0,1.0);
    let alpha=1.0-transmission+transmission*mix(extra_alpha,1.0,low);
    let color=radiance+transmission*(extras+background*low);
    return vec4(color,alpha);
}
