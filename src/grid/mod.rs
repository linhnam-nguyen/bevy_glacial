//! Local LOD ground grid module.

mod mesh;
mod systems;
mod types;

use bevy::prelude::*;

pub use systems::{build_grid_meshes, spawn_circle_meshes, update_grid_alpha};
pub use types::{
    BASE_STEP, DOT_CLOSE_FALLOFF, DOT_OFFSET_FRAC, DOT_RADIUS_FRAC, DOT_SEGMENTS, EDGE_FADE_FRAC,
    GAUSS_PEAK, GAUSS_WIDTH, GlacialGridCounters, GridKind, GroundGrid, LEVEL_HALF, LEVEL_SCALE,
    LEVEL_STEPS, LINES_PER_SIDE, LINE_CLOSE_FALLOFF, LocalGrid, MAJOR_BOOST, MAJOR_EVERY,
};

pub struct GroundGridPlugin;

impl Plugin for GroundGridPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroundGrid>()
            .init_resource::<GlacialGridCounters>()
            .add_systems(Startup, systems::setup_ground_grid)
            .add_systems(Update, (build_grid_meshes, update_grid_alpha));
    }
}
