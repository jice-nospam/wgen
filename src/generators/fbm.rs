use eframe::egui;
use noise::{Fbm, MultiFractal, NoiseFn, Seedable};
use serde::{Deserialize, Serialize};
use three_d::{
    vec3, Camera, CpuMesh, Cull, DepthTest, Gm, Interpolation, Material, MaterialType, Mesh,
    RenderStates, Texture2D, Viewport, Wrapping, WriteMask,
};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FbmConf {
    pub mulx: f32,
    pub muly: f32,
    pub addx: f32,
    pub addy: f32,
    pub octaves: f32,
    pub delta: f32,
    pub scale: f32,
}

impl Default for FbmConf {
    fn default() -> Self {
        Self {
            mulx: 2.20,
            muly: 2.20,
            addx: 0.0,
            addy: 0.0,
            octaves: 6.0,
            delta: 0.0,
            scale: 2.05,
        }
    }
}

pub fn render_fbm(ui: &mut egui::Ui, conf: &mut FbmConf) -> bool {
    let old = conf.clone();
    ui.horizontal(|ui| {
        ui.label("scale x");
        ui.add(
            egui::DragValue::new(&mut conf.mulx)
                .speed(0.1)
                .clamp_range(0.0..=100.0),
        );
        ui.label("y");
        ui.add(
            egui::DragValue::new(&mut conf.muly)
                .speed(0.1)
                .clamp_range(0.0..=100.0),
        );
        ui.label("octaves");
        ui.add(
            egui::DragValue::new(&mut conf.octaves)
                .speed(0.2)
                .clamp_range(1.0..=Fbm::MAX_OCTAVES as f32),
        );
    });
    ui.horizontal(|ui| {
        ui.label("offset x");
        ui.add(
            egui::DragValue::new(&mut conf.addx)
                .speed(0.01)
                .clamp_range(0.0..=200.0),
        );
        ui.label("y");
        ui.add(
            egui::DragValue::new(&mut conf.addy)
                .speed(0.01)
                .clamp_range(0.0..=200.0),
        );
        ui.label("scale");
        ui.add(
            egui::DragValue::new(&mut conf.scale)
                .speed(0.01)
                .clamp_range(0.01..=10.0),
        );
    });
    *conf != old
}

struct FbmMaterial {
    conf: FbmConf,
    seed: u64,
}
impl Material for FbmMaterial {
    fn fragment_shader_source(
        &self,
        _use_vertex_colors: bool,
        _lights: &[&dyn three_d::Light],
    ) -> String {
        String::from(
            "
            uniform float u_octaves;
            uniform float u_addx;
            uniform float u_addy;
            uniform float u_mulx;
            uniform float u_muly;
            uniform float u_scale;
            uniform float u_seed;

            in vec3 pos;
            layout (location = 0) out vec4 color;

            vec2 random(vec2 uv){
                uv = vec2( dot(uv, vec2(127.1,311.7) ),
                           dot(uv, vec2(269.5,183.3) ) );
                return -1.0 + 2.0 * fract(sin(uv) * 43758.5453123);
            }

            float noise(vec2 uv) {
                vec2 uv_index = floor(uv);
                vec2 uv_fract = fract(uv);

                vec2 blur = smoothstep(0.0, 1.0, uv_fract);

                return mix( mix( dot( random(uv_index + vec2(0.0,0.0) ), uv_fract - vec2(0.0,0.0) ),
                                 dot( random(uv_index + vec2(1.0,0.0) ), uv_fract - vec2(1.0,0.0) ), blur.x),
                            mix( dot( random(uv_index + vec2(0.0,1.0) ), uv_fract - vec2(0.0,1.0) ),
                                 dot( random(uv_index + vec2(1.0,1.0) ), uv_fract - vec2(1.0,1.0) ), blur.x), blur.y) + 0.5;
            }


            float fbm(vec2 uv) {
                int octaves = int(u_octaves);
                float amplitude = 0.5;
                float frequency = 3.0;
                float value = 0.0;
                vec2 pos = uv * vec2(u_mulx,u_muly) + vec2(u_addx,u_addy);
                pos.x += mod(u_seed,31) * 5.0;

                for(int i = 0; i < octaves; i++) {
                    value += amplitude * noise(frequency * pos);
                    amplitude *= 0.5;
                    frequency *= 2.0;
                }
                float remain = fract(u_octaves);
                value += remain * amplitude * noise(frequency * pos);
                return value * u_scale;
            }

            void main()
            {
                float h = fbm(pos.xy);
                color = vec4(h, h, h, 1.0f);
            }",
        )
    }

    fn use_uniforms(
        &self,
        program: &three_d::Program,
        _camera: &Camera,
        _lights: &[&dyn three_d::Light],
    ) {
        program.use_uniform("u_addx", self.conf.addx);
        program.use_uniform("u_addy", self.conf.addy);
        program.use_uniform("u_mulx", self.conf.mulx);
        program.use_uniform("u_muly", self.conf.muly);
        program.use_uniform("u_scale", self.conf.scale);
        program.use_uniform("u_octaves", self.conf.octaves);
        program.use_uniform("u_seed", self.seed as f32);
    }

    fn render_states(&self) -> RenderStates {
        RenderStates {
            depth_test: DepthTest::Always,
            write_mask: WriteMask::COLOR,
            cull: Cull::Back,
            ..Default::default()
        }
    }

    fn material_type(&self) -> MaterialType {
        MaterialType::Opaque
    }
}

fn gen_fbm_gpu(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FbmConf,
    gl: &std::sync::Arc<glow::Context>,
) -> Result<(), ()> {
    let context = three_d::Context::from_gl_context(gl.clone()).unwrap();
    let mut texture = Texture2D::new_empty::<f32>(
        &context,
        size.0 as u32,
        size.1 as u32,
        Interpolation::Nearest,
        Interpolation::Nearest,
        None,
        Wrapping::ClampToEdge,
        Wrapping::ClampToEdge,
    );
    let pixels = texture.as_color_target(None);

    let camera = Camera::new_orthographic(
        Viewport {
            x: 0,
            y: 0,
            width: size.0 as u32,
            height: size.1 as u32,
        },
        vec3(0.0, 0.0, 1.0),
        vec3(0.0, 0.0, 0.0),
        vec3(0.0, 1.0, 0.0),
        2.0,
        0.0,
        10.0,
    );

    let mesh = Gm::new(
        Mesh::new(&context, &CpuMesh::square()),
        FbmMaterial {
            seed,
            conf: conf.clone(),
        },
    );
    pixels.render(&camera, &[&mesh], &[]);
    let data: Vec<f32> = pixels.read();
    hmap.copy_from_slice(&data[..]);
    Ok(())
}

pub fn gen_fbm(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FbmConf,
    gl: &Option<std::sync::Arc<glow::Context>>,
) {
    if let Some(gl) = gl {
        if gen_fbm_gpu(seed, size, hmap, conf, &gl).is_ok() {
            return;
        }
    }
    // fall back to CPU generator
    let xcoef = conf.mulx / 400.0;
    let ycoef = conf.muly / 400.0;
    let num_threads = num_cpus::get();
    std::thread::scope(|s| {
        let size_per_job = size.1 / num_threads;
        for (i, chunk) in hmap.chunks_mut(size_per_job * size.0).enumerate() {
            let i = i;
            let fbm = Fbm::new()
                .set_seed(seed as u32)
                .set_octaves(conf.octaves as usize);
            s.spawn(move || {
                let yoffset = i * size_per_job;
                let lasty = size_per_job.min(size.1 - yoffset);
                for y in 0..lasty {
                    let f1 = ((y + yoffset) as f32 * 512.0 / size.1 as f32 + conf.addy) * ycoef;
                    let mut offset = y * size.0;
                    for x in 0..size.0 {
                        let f0 = (x as f32 * 512.0 / size.0 as f32 + conf.addx) * xcoef;
                        let value =
                            conf.delta + fbm.get([f0 as f64, f1 as f64]) as f32 * conf.scale;
                        chunk[offset] += value;
                        offset += 1;
                    }
                }
            });
        }
    });
}
