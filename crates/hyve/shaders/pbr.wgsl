// Minimal PBR placeholder shader asset verified at compile/test time via `asset!("/shaders/pbr.wgsl")`.
@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.05, 0.06, 0.08, 1.0);
}
