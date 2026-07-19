//! GLSL source for the desk scene's two programs: the main prop/CRT/glow
//! program and the offscreen fullscreen-triangle blit program.

pub(super) const VERTEX_SHADER: &str = r#"#version 150
uniform mat4 u_view_proj;
uniform mat4 u_model;
in vec3 a_pos;
in vec3 a_normal;
in vec2 a_uv;
out vec3 v_normal;
out vec2 v_uv;
void main() {
    v_normal = mat3(u_model) * a_normal;
    v_uv = a_uv;
    gl_Position = u_view_proj * u_model * vec4(a_pos, 1.0);
}
"#;

/// `u_mode` values shared between [`FRAGMENT_SHADER`] and the draw loop.
pub(super) const MODE_SOLID: i32 = 0;
pub(super) const MODE_CRT: i32 = 1;
pub(super) const MODE_GLOW: i32 = 2;

pub(super) const FRAGMENT_SHADER: &str = r#"#version 150
uniform vec4 u_color;
uniform sampler2D u_tex;   // solid: unused; CRT: phosphor; glow quad: glow
uniform sampler2D u_glow;  // CRT face only: blurred glow for the bloom add
uniform int u_mode;        // 0 solid lit, 1 CRT face, 2 additive glow quad
uniform vec4 u_crt_a;      // barrel, scanlines, mask, bloom strengths
uniform vec4 u_crt_b;      // reflection strength, source scanline count, -, -
uniform float u_highlight; // hover feedback: mix toward white (solid mode)
in vec3 v_normal;
in vec2 v_uv;
out vec4 frag_color;
const vec3 LIGHT_DIR = vec3(0.35, 0.86, 0.37); // pre-normalized
const float AMBIENT = 0.35;
const float PI = 3.14159265;
// uv displacement toward the corners at barrel strength 1.0
const float BARREL_MAX = 0.5;
// aperture-grille RGB triads across the tube width
const float MASK_TRIADS = 320.0;
// must match Rust's GLOW_QUAD_SCALE
const float GLOW_QUAD_SCALE = 1.5;

// The tube face: barrel-distort the sample position (image bulges, raster
// pulls in from the quad corners), then scanlines and grille on the tube
// pixel, then bloom and the room-light streak on the glass over everything.
vec3 crt_face(vec2 uv) {
    float barrel = u_crt_a.x, scan = u_crt_a.y, mask = u_crt_a.z, bloom = u_crt_a.w;
    float refl = u_crt_b.x, lines = u_crt_b.y;
    vec2 centered = uv - 0.5;
    vec2 tube_uv = 0.5 + centered * (1.0 + barrel * BARREL_MAX * dot(centered, centered) * 2.0);
    vec3 col = vec3(0.0);
    if (all(greaterThanEqual(tube_uv, vec2(0.0))) && all(lessThanEqual(tube_uv, vec2(1.0)))) {
        col = texture(u_tex, tube_uv).rgb;
        col *= 1.0 - scan * 0.5 * (1.0 - cos(tube_uv.y * lines * 2.0 * PI));
        int triad = int(mod(floor(tube_uv.x * MASK_TRIADS * 3.0), 3.0));
        vec3 tint = triad == 0 ? vec3(1.0, 0.6, 0.6)
                  : triad == 1 ? vec3(0.6, 1.0, 0.6)
                               : vec3(0.6, 0.6, 1.0);
        col *= mix(vec3(1.0), tint, mask);
    }
    col += texture(u_glow, uv).rgb * bloom * 0.7;
    float d = dot(uv - vec2(0.30, 0.25), normalize(vec2(0.8, 1.0)));
    col += refl * 0.25 * exp(-d * d * 40.0);
    return col;
}

// The oversized additive quad in front of the bezel: the blurred tube image
// with a radial falloff, so bright screens light the plastic around them.
vec3 glow_quad() {
    vec2 tube = (v_uv - 0.5) * GLOW_QUAD_SCALE + 0.5;
    vec3 g = texture(u_tex, clamp(tube, 0.0, 1.0)).rgb;
    float falloff = smoothstep(1.0, 0.45, length(v_uv - 0.5) * 2.0);
    return g * falloff;
}

void main() {
    if (u_mode == 1) {
        frag_color = vec4(crt_face(v_uv), 1.0);
    } else if (u_mode == 2) {
        frag_color = vec4(glow_quad() * u_crt_a.w * 0.8, 1.0);
    } else {
        float diffuse = max(dot(normalize(v_normal), LIGHT_DIR), 0.0);
        float light = mix(AMBIENT, 1.0, diffuse);
        vec3 col = mix(u_color.rgb * light, vec3(1.0), u_highlight);
        frag_color = vec4(col, u_color.a);
    }
}
"#;

/// Fullscreen-triangle vertex shader for the offscreen passes (no vertex
/// buffer; positions derived from `gl_VertexID`, drawn with an empty VAO).
pub(super) const BLIT_VERTEX_SHADER: &str = r#"#version 150
out vec2 v_uv;
void main() {
    vec2 pos = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    v_uv = pos * 0.5 + 0.5;
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

/// Pass 0: phosphor persistence — new frame combined with the decayed
/// previous phosphor surface (`max`, like phosphor that re-excites).
/// Pass 1: 9-tap blur of the phosphor surface into the small glow target.
pub(super) const BLIT_FRAGMENT_SHADER: &str = r#"#version 150
uniform sampler2D u_src;
uniform sampler2D u_prev;
uniform int u_pass;
uniform float u_decay;
uniform vec2 u_texel;
in vec2 v_uv;
out vec4 frag_color;
void main() {
    if (u_pass == 0) {
        vec3 cur = texture(u_src, v_uv).rgb;
        vec3 prev = texture(u_prev, v_uv).rgb * u_decay;
        frag_color = vec4(max(cur, prev), 1.0);
    } else {
        vec2 o = u_texel * 1.6;
        vec3 sum = texture(u_src, v_uv).rgb * 0.2;
        sum += (texture(u_src, v_uv + vec2(o.x, 0.0)).rgb
              + texture(u_src, v_uv - vec2(o.x, 0.0)).rgb
              + texture(u_src, v_uv + vec2(0.0, o.y)).rgb
              + texture(u_src, v_uv - vec2(0.0, o.y)).rgb) * 0.125;
        sum += (texture(u_src, v_uv + o).rgb
              + texture(u_src, v_uv - o).rgb
              + texture(u_src, v_uv + vec2(o.x, -o.y)).rgb
              + texture(u_src, v_uv + vec2(-o.x, o.y)).rgb) * 0.075;
        frag_color = vec4(sum, 1.0);
    }
}
"#;
