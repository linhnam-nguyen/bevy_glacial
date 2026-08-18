//! Local LOD ground grid — a stack of flat square grids of lines
//! centred on the chase-camera's focus.
//!
//! Each level uses a fixed decade spacing (1 m, 10 m, 100 m, 1 km);
//! the level whose cell size best matches the current view scale
//! fades in, neighbours fade down, and far levels disappear. Unlike
//! the old sphere-surface system, the meshes here are built once at
//! startup and just translate with the camera — there's no per-frame
//! vertex rebuild, so panning stays smooth.
//!
//! Fades:
//!   - Per-level Gaussian on `log(cam_dist / step)` — peaks at the
//!     level whose cell size is ~10× the camera distance.
//!   - Radial in the mesh: alpha drops with distance from centre, so
//!     the outer edge dissolves into the ground instead of ending in
//!     a hard square border.
//!   - Major-line boost every 10th line reads as a "chapter" tick
//!     without becoming noisy.

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use super::camera::ChaseCamera;

// ── User-visible settings ───────────────────────────────────────────

#[derive(Resource, Clone, Copy)]
pub struct GroundGrid {
    pub visible: bool,
    /// Base RGB + alpha. Alpha scales everything.
    pub color: Color,
    /// World-space height of the loaded scene's ground reference. `None`
    /// keeps the grid hidden until the host application has a scene bound.
    pub ground_y: Option<f32>,
    /// Desired outer coverage radius for the coarsest level. The host can
    /// raise this for large imported stages; the coarsest level is scaled
    /// without changing the finer local LOD levels.
    pub coverage_radius: f32,
}

impl Default for GroundGrid {
    fn default() -> Self {
        Self {
            visible: true,
            // Cool blue-tinted off-white at ~60 % alpha. Reads
            // against both dark grounds and bright sky clear
            // colours; the major-line × 3.5 boost still fits
            // inside the alpha cap.
            color: Color::srgba(0.62, 0.76, 0.95, 0.6),
            ground_y: None,
            coverage_radius: LEVEL_HALF[LEVEL_HALF.len() - 1],
        }
    }
}

// ── LOD levels ──────────────────────────────────────────────────────

/// Multiplicative spacing between adjacent LOD levels. `4.0` means
/// each level's cells are 4× the previous level's, and every 4th line
/// in a level lines up with a major line of the level above it.
pub const LEVEL_SCALE: f32 = 4.0;
/// Cell size of the finest level (metres). Each subsequent level is
/// ×[`LEVEL_SCALE`] this — set the base smaller for a denser grid,
/// larger for a coarser one.
const BASE_STEP: f32 = 0.5;
/// Cell size per level (metres). Geometric ×[`LEVEL_SCALE`] so
/// neighbouring levels stay tile-aligned.
pub const LEVEL_STEPS: [f32; 4] = [
    BASE_STEP,
    BASE_STEP * LEVEL_SCALE,
    BASE_STEP * LEVEL_SCALE * LEVEL_SCALE,
    BASE_STEP * LEVEL_SCALE * LEVEL_SCALE * LEVEL_SCALE,
];
/// Lines per side for each level. Coarser levels shrink so the
/// "very-far" horizon doesn't extend kilometres past the camera —
/// at the coarsest level we'd otherwise draw a 6 km square. Finer
/// levels keep their full count for close-up detail.
const LINES_PER_SIDE: [f32; 4] = [100.0, 60.0, 35.0, 20.0];
/// Half-extent of each level's square (metres).
pub const LEVEL_HALF: [f32; 4] = [
    LINES_PER_SIDE[0] * LEVEL_STEPS[0],
    LINES_PER_SIDE[1] * LEVEL_STEPS[1],
    LINES_PER_SIDE[2] * LEVEL_STEPS[2],
    LINES_PER_SIDE[3] * LEVEL_STEPS[3],
];
/// Every Nth line is a major line (brighter alpha). Equal to
/// [`LEVEL_SCALE`] so a level's major lines coincide with the next
/// level's minor lines.
const MAJOR_EVERY: i32 = LEVEL_SCALE as i32;
/// Major-line alpha boost (multiplied against the base colour alpha)
/// so every `MAJOR_EVERY`-th line reads as a heavier "chapter" tick.
const MAJOR_BOOST: f32 = 3.5;
/// Fraction of the level's half-extent over which the radial fade
/// kicks in. `1.0` = fade smoothly from centre to edge; `0.92` =
/// full strength only in the inner 8 % of the radius, then a long
/// smoothstep down to 0 over the outer 92 %. High values keep the
/// far horizon transparent so grid lines don't pile up at the
/// vanishing point.
const EDGE_FADE_FRAC: f32 = 0.92;
/// Peak fade at `log10(cam_dist / step) ≈ GAUSS_PEAK`. Set so a
/// level peaks when the camera is [`LEVEL_SCALE`]× its cell size
/// away — i.e. when the cells look like a comfortable fraction of
/// the view.
const GAUSS_PEAK: f32 = 0.602_06; // log10(4)
/// Bell width. Wider = adjacent levels linger longer before fading
/// out as the camera moves between their natural distances.
const GAUSS_WIDTH: f32 = 0.55;
/// How much sharper the per-kind fade is on the "camera too close"
/// side of the Gaussian peak. `1.0` = symmetric (default);
/// `> 1.0` = coarser-than-current levels vanish faster as you zoom
/// in. Dots get the more aggressive value because their absolute
/// size scales with the level's step (a level-N+1 dot is 4 × the
/// area of a level-N dot, so even faint bleed-through reads).
const LINE_CLOSE_FALLOFF: f32 = 2.5;
const DOT_CLOSE_FALLOFF: f32 = 6.0;

// ── Components ──────────────────────────────────────────────────────

/// Two layers per LOD level — the cross-hatched line grid, and a
/// triangle-fan disc at every major intersection. Dots ride a
/// hair above lines so they read on top.
#[derive(Component, Copy, Clone, PartialEq, Eq, Debug)]
pub enum GridKind {
    Lines,
    Dots,
}

#[derive(Component)]
pub struct LocalGrid {
    pub level: u8,
    pub kind: GridKind,
    pub material: Handle<StandardMaterial>,
}

/// Dot-disc radius as a fraction of the cell step. ~2.4 % reads as
/// a fine tick at every intersection without becoming a bullet point.
const DOT_RADIUS_FRAC: f32 = 0.024;
/// Number of triangle-fan segments per dot. 8 is enough to read as
/// a circle at the size we draw them and keeps the vertex count
/// down — there's a dot at every minor intersection.
const DOT_SEGMENTS: u32 = 8;
/// Relative Y offset so dots paint on top of lines without introducing a
/// fixed absolute gap for very small imported scenes.
const DOT_OFFSET_FRAC: f32 = 0.0001;

// ── Plugin ──────────────────────────────────────────────────────────

/// Plugin: registers the [`GroundGrid`] resource, spawns the LOD grid
/// meshes at Startup, and runs the per-frame follow / fade systems.
///
/// The grid does not need a ground plane to render against — it sits
/// on the host-provided `GroundGrid::ground_y` plane and is fully
/// self-contained. The grid remains hidden until the host binds a loaded
/// scene ground reference.
pub struct GroundGridPlugin;

impl Plugin for GroundGridPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroundGrid>()
            .add_systems(Startup, setup_ground_grid)
            .add_systems(Update, (build_grid_meshes, update_grid_alpha));
    }
}

fn setup_ground_grid(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cfg: Res<GroundGrid>,
) {
    spawn_circle_meshes(&mut commands, &mut meshes, &mut materials, &cfg);
}

// ── Spawn ───────────────────────────────────────────────────────────

/// Spawn two entities per LOD level — the line cross-hatch and the
/// dots at major intersections. Name kept for back-compat with
/// `main::setup_scene`.
pub fn spawn_circle_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    cfg: &GroundGrid,
) {
    let make_mat = |materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        })
    };

    for level in 0..LEVEL_STEPS.len() {
        let step = LEVEL_STEPS[level];
        let half = LEVEL_HALF[level];
        let is_top = level + 1 == LEVEL_STEPS.len();

        // Lines layer.
        let lines_mesh = meshes.add(build_level_mesh(cfg, step, half, is_top));
        let lines_mat = make_mat(materials);
        commands.spawn((
            Name::new(format!("LocalGrid[L{level}]:Lines")),
            LocalGrid {
                level: level as u8,
                kind: GridKind::Lines,
                material: lines_mat.clone(),
            },
            Transform::default(),
            Mesh3d(lines_mesh),
            MeshMaterial3d(lines_mat),
            NotShadowCaster,
            Visibility::Visible,
        ));

        // Dots layer — small disc at every minor intersection
        // (skipping anything the parent level draws).
        let dots_mesh = meshes.add(build_dots_mesh(cfg, step, half, is_top));
        let dots_mat = make_mat(materials);
        commands.spawn((
            Name::new(format!("LocalGrid[L{level}]:Dots")),
            LocalGrid {
                level: level as u8,
                kind: GridKind::Dots,
                material: dots_mat.clone(),
            },
            Transform::default(),
            Mesh3d(dots_mesh),
            MeshMaterial3d(dots_mat),
            NotShadowCaster,
            Visibility::Visible,
        ));
    }
}

// ── Per-frame systems ──────────────────────────────────────────────

/// Camera-follow: slide every grid level with the chase-camera
/// focus, snapped to that level's minor step so lines stay
/// world-aligned. Also writes each level's fade to its material's
/// alpha — the level whose cell size matches the current zoom blends
/// in, the rest fade out.
pub fn build_grid_meshes(
    cameras: Query<&ChaseCamera>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cfg: Res<GroundGrid>,
    mut grids: Query<(&LocalGrid, &mut Transform, &mut Visibility)>,
) {
    let Ok(cam) = cameras.single() else { return };
    let cam_dist = cam.distance.max(0.1);

    for (grid, mut tr, mut vis) in grids.iter_mut() {
        let Some(ground_y) = cfg.ground_y else {
            *vis = Visibility::Hidden;
            continue;
        };

        let level = grid.level as usize;
        let is_coarsest = level + 1 == LEVEL_STEPS.len();
        let level_scale = if is_coarsest {
            (cfg.coverage_radius.max(LEVEL_HALF[level]) / LEVEL_HALF[level]).max(1.0)
        } else {
            1.0
        };
        let step = LEVEL_STEPS[level] * level_scale;
        // Snap to the *major* step so the mesh translates as a
        // rigid sheet — both lines and dots sit at the same world
        // positions every frame.
        let snap_step = step * MAJOR_EVERY as f32;
        tr.translation.x = (cam.focus.x / snap_step).round() * snap_step;
        tr.translation.y = match grid.kind {
            GridKind::Lines => ground_y,
            GridKind::Dots => ground_y + step * DOT_OFFSET_FRAC,
        };
        tr.translation.z = (cam.focus.z / snap_step).round() * snap_step;
        tr.scale = Vec3::new(level_scale, 1.0, level_scale);

        let fade = match grid.kind {
            GridKind::Lines => level_fade(cam_dist, step, LINE_CLOSE_FALLOFF),
            GridKind::Dots => level_fade(cam_dist, step, DOT_CLOSE_FALLOFF),
        };
        let a = cfg.color.alpha() * fade;
        *vis = if cfg.visible && a > 0.005 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(mut m) = materials.get_mut(&grid.material) {
            let srgba = cfg.color.to_srgba();
            m.base_color = Color::srgba(srgba.red, srgba.green, srgba.blue, a);
        }
    }
}

/// When the UI changes grid colour, rebuild the tiny per-level line
/// meshes so the vertex-colour alpha pattern updates. Infrequent.
pub fn update_grid_alpha(
    cfg: Res<GroundGrid>,
    mut meshes: ResMut<Assets<Mesh>>,
    grids: Query<(&LocalGrid, &Mesh3d)>,
) {
    if !cfg.is_changed() {
        return;
    }
    for (grid, mesh_h) in grids.iter() {
        let step = LEVEL_STEPS[grid.level as usize];
        let half = LEVEL_HALF[grid.level as usize];
        let is_top = grid.level as usize + 1 == LEVEL_STEPS.len();
        if let Some(mut m) = meshes.get_mut(&mesh_h.0) {
            *m = match grid.kind {
                GridKind::Lines => build_level_mesh(&cfg, step, half, is_top),
                GridKind::Dots => build_dots_mesh(&cfg, step, half, is_top),
            };
        }
    }
}

// ── LOD fade ───────────────────────────────────────────────────────

/// Asymmetric Gaussian bell over `log10(cam_dist / step)`. Peak at
/// `GAUSS_PEAK`, width `GAUSS_WIDTH`. The "camera too close" side
/// of the bell (z < 0, this level coarser than the camera needs)
/// is sharpened by `close_falloff` so coarser-level grid doesn't
/// bleed through the finer level the user is on. The "camera too
/// far" side (z > 0) keeps the standard width so adjacent levels
/// hand off smoothly as the user zooms out.
fn level_fade(cam_dist: f32, step: f32, close_falloff: f32) -> f32 {
    let log_r = (cam_dist / step).max(1e-3).log10();
    let z = (log_r - GAUSS_PEAK) / GAUSS_WIDTH;
    let z_eff = if z < 0.0 { z * close_falloff } else { z };
    (-0.5 * z_eff * z_eff).exp()
}

// ── Mesh generation ────────────────────────────────────────────────

fn build_level_mesh(cfg: &GroundGrid, step: f32, half: f32, _is_top: bool) -> Mesh {
    let s = cfg.color.to_srgba();
    let base_rgba = [s.red, s.green, s.blue, s.alpha];

    let n = (half / step) as i32;
    // Each line is subdivided into one segment per cell so the per-
    // vertex alpha can vary along its length. That's what gives the
    // grid a soft *disc* falloff instead of a hard square edge —
    // points along a single line at different radii get different
    // alpha. We deliberately do NOT skip lines that the next-coarser
    // LOD also draws: the `LineList` primitive is 1 pixel wide, so
    // two levels stamping the same line just composite into a
    // slightly brighter pixel (invisible). Skipping introduces gaps
    // as soon as the parent level fades out.
    let segments = 2 * n;
    let total_segments = (2 * n + 1) * 2 * segments;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((total_segments * 2) as usize);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity((total_segments * 2) as usize);

    // Smoothstep on Euclidean radius — full strength inside the
    // inner `1 - EDGE_FADE_FRAC` of the radius, then a smooth roll-
    // off to 0 at `r = half`. Beyond `half` (e.g. at the square's
    // diagonal corners) the fade naturally clamps to 0, which is
    // why corners disappear cleanly.
    let radial_fade = |x: f32, z: f32| -> f32 {
        let r = (x * x + z * z).sqrt();
        let t = (r / half).clamp(0.0, 1.0);
        let u = ((1.0 - t) / EDGE_FADE_FRAC).clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };

    let vertex_color = |x: f32, z: f32, axis_idx: i32| -> [f32; 4] {
        let major = axis_idx.rem_euclid(MAJOR_EVERY) == 0;
        let boost = if major { MAJOR_BOOST } else { 1.0 };
        [
            base_rgba[0],
            base_rgba[1],
            base_rgba[2],
            (base_rgba[3] * radial_fade(x, z) * boost).clamp(0.0, 1.0),
        ]
    };

    // Lines running along +X (constant Z), subdivided cell by cell.
    for i in -n..=n {
        let z = i as f32 * step;
        for s in 0..segments {
            let x0 = -half + s as f32 * step;
            let x1 = -half + (s + 1) as f32 * step;
            positions.push([x0, 0.0, z]);
            positions.push([x1, 0.0, z]);
            colors.push(vertex_color(x0, z, i));
            colors.push(vertex_color(x1, z, i));
        }
    }
    // Lines running along +Z (constant X), subdivided cell by cell.
    for i in -n..=n {
        let x = i as f32 * step;
        for s in 0..segments {
            let z0 = -half + s as f32 * step;
            let z1 = -half + (s + 1) as f32 * step;
            positions.push([x, 0.0, z0]);
            positions.push([x, 0.0, z1]);
            colors.push(vertex_color(x, z0, i));
            colors.push(vertex_color(x, z1, i));
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::LineList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

/// Triangle-fan disc mesh — one disc at every minor intersection
/// inside the level's `half`-extent. Major intersections get a
/// brighter alpha (same `MAJOR_BOOST` as the line layer) so the
/// grid still has tick-mark structure.
fn build_dots_mesh(cfg: &GroundGrid, step: f32, half: f32, _is_top: bool) -> Mesh {
    let s = cfg.color.to_srgba();
    let base_rgba = [s.red, s.green, s.blue, s.alpha];

    let n = (half / step) as i32;
    let radius = step * DOT_RADIUS_FRAC;
    let segs = DOT_SEGMENTS;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for i in -n..=n {
        for j in -n..=n {
            // No major-boost on dots — dots from every LOD stack at
            // the world origin, and a 3.5× boost on each turns them
            // into a single bright disc. Major hierarchy lives on
            // the line layer.
            let cx = i as f32 * step;
            let cz = j as f32 * step;
            // Euclidean radial fade: dots outside the inscribed disc
            // (corners of the square) clamp to zero, so the grid
            // disappears as a soft circle rather than a hard square.
            let r = (cx * cx + cz * cz).sqrt();
            let t = (r / half).clamp(0.0, 1.0);
            let edge_fade = {
                let u = ((1.0 - t) / EDGE_FADE_FRAC).clamp(0.0, 1.0);
                u * u * (3.0 - 2.0 * u)
            };
            let alpha = (base_rgba[3] * edge_fade).clamp(0.0, 1.0);
            let color = [base_rgba[0], base_rgba[1], base_rgba[2], alpha];

            // Shrink in world-space the further the dot sits from
            // the panel centre. Square the fade so the size drops
            // off faster than the alpha — outer dots read as fine
            // pin-pricks before they vanish.
            let dot_radius = radius * edge_fade * edge_fade;

            let centre_idx = positions.len() as u32;
            positions.push([cx, 0.0, cz]);
            colors.push(color);
            for k in 0..segs {
                let theta = (k as f32 / segs as f32) * std::f32::consts::TAU;
                let (sn, cs) = theta.sin_cos();
                positions.push([cx + cs * dot_radius, 0.0, cz + sn * dot_radius]);
                colors.push(color);
            }
            for k in 0..segs {
                let next = (k + 1) % segs;
                indices.push(centre_idx);
                indices.push(centre_idx + 1 + k);
                indices.push(centre_idx + 1 + next);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}
