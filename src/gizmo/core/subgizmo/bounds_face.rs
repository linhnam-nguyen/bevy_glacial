use glam::DVec3;

use super::super::bounds::BoundsFace;
use super::super::math::{ray_to_ray, round_to_interval};
use super::super::{GizmoDrawData, GizmoResult, gizmo::Ray};
use super::common::{
    PickResult, bounds_face_center, bounds_face_normal, draw_bounds_face, pick_bounds_face,
};
use super::{SubGizmoConfig, SubGizmoKind};

pub(crate) type BoundsFaceSubGizmo = SubGizmoConfig<BoundsFaceGizmo>;

#[derive(Debug, Copy, Clone, Hash)]
pub(crate) struct BoundsFaceParams {
    pub face: BoundsFace,
}

#[derive(Default, Debug, Copy, Clone)]
pub(crate) struct BoundsFaceState {
    start_center: DVec3,
    start_normal: DVec3,
    start_parameter: f64,
}

#[derive(Default, Debug, Copy, Clone)]
pub(crate) struct BoundsFaceGizmo;

impl SubGizmoKind for BoundsFaceGizmo {
    type Params = BoundsFaceParams;
    type State = BoundsFaceState;
    type PickPreview = PickResult;

    fn pick_preview(subgizmo: &BoundsFaceSubGizmo, ray: Ray) -> PickResult {
        pick_bounds_face(&subgizmo.config, ray, subgizmo.face)
    }

    fn pick(subgizmo: &mut BoundsFaceSubGizmo, ray: Ray) -> Option<f64> {
        let pick_result = Self::pick_preview(subgizmo, ray);
        let center = bounds_face_center(&subgizmo.config, subgizmo.face);
        let normal = bounds_face_normal(&subgizmo.config, subgizmo.face);
        let (_, start_parameter) = ray_to_ray(ray.origin, ray.direction, center, normal);

        subgizmo.state.start_center = center;
        subgizmo.state.start_normal = normal;
        subgizmo.state.start_parameter = start_parameter;

        pick_result.picked.then_some(pick_result.t)
    }

    fn update(subgizmo: &mut BoundsFaceSubGizmo, ray: Ray) -> Option<GizmoResult> {
        let (_, current_parameter) = ray_to_ray(
            ray.origin,
            ray.direction,
            subgizmo.state.start_center,
            subgizmo.state.start_normal,
        );
        let mut displacement = current_parameter - subgizmo.state.start_parameter;
        if subgizmo.config.snapping {
            displacement = round_to_interval(displacement, subgizmo.config.snap_distance as f64);
        }

        Some(GizmoResult::ResizeFace {
            face: subgizmo.face,
            delta: displacement,
        })
    }

    fn draw(subgizmo: &BoundsFaceSubGizmo) -> GizmoDrawData {
        draw_bounds_face(&subgizmo.config, subgizmo.face, subgizmo.focused)
    }
}
