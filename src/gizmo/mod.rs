//! Transform gizmo — translate / rotate / scale handles for Bevy.
//!
//! Vendored from
//! [`transform-gizmo-bevy`](https://github.com/urholaukkarinen/transform-gizmo)
//! v0.9.0 by **Urho Laukkarinen** (`<urho.laukkarinen@gmail.com>`),
//! dual-licensed MIT or Apache-2.0. See `LICENSE-MIT` /
//! `LICENSE-APACHE` next to `bevy_glacial/Cargo.toml`. The
//! framework-agnostic core is in [`core`]; this module is the Bevy
//! adapter (plugin, `GizmoCamera`, `GizmoTarget`, picking + mouse
//! interaction + render plumbing).
//!
//! # Usage
//!
//! Add [`TransformGizmoPlugin`] to your App, attach [`GizmoCamera`] to
//! whichever camera should render the gizmo, and add [`GizmoTarget`]
//! to any entity whose `Transform` you want manipulable. Configure
//! via the [`GizmoOptions`] resource.

use bevy::app::prelude::*;
use bevy::asset::{AssetApp, Assets};
use bevy::camera::primitives::Aabb;
use bevy::camera::{Camera, Projection};
use bevy::ecs::prelude::*;
use bevy::input::prelude::*;
use bevy::math::Vec2;
use bevy::picking::hover::HoverMap;
use bevy::platform::collections::HashMap;
use bevy::transform::prelude::*;
use bevy::window::{PrimaryWindow, Window};
use uuid::Uuid;

use mouse_interact::MouseGizmoInteractionPlugin;
use picking::TransformGizmoPickingPlugin;
use render::{DrawDataHandles, TransformGizmoRenderPlugin};
use target_transform::apply_result_transform;

use core::config::{
    DEFAULT_SNAP_ANGLE, DEFAULT_SNAP_DISTANCE, DEFAULT_SNAP_SCALE, GizmoModeKind,
    TransformPivotPoint,
};
pub use core::{
    GizmoConfig,
    math::{Pos2, Rect},
    *,
};

pub mod core;
pub mod mouse_interact;
pub mod picking;

mod render;
mod target_transform;

const GIZMO_GROUP_UUID: Uuid = Uuid::from_u128(0x_1c90_3d44_0152_45e1_b1c9_889a_0203_e90c);

/// Adds transform gizmos to the App.
///
/// Gizmos are interactive tools that appear in the scene, allowing users to manipulate
/// entities' transforms (position, rotation, scale) visually.
pub struct TransformGizmoPlugin;

impl Plugin for TransformGizmoPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<render::GizmoDrawData>()
            .init_resource::<GizmoOptions>()
            .init_resource::<GizmoStorage>()
            .add_message::<GizmoDragStarted>()
            .add_message::<GizmoDragging>()
            .add_plugins(TransformGizmoRenderPlugin)
            .add_plugins(TransformGizmoPickingPlugin)
            .add_plugins(MouseGizmoInteractionPlugin)
            .add_systems(
                Last,
                (handle_hotkeys, update_gizmos, draw_gizmos, cleanup_old_data).chain(),
            );
    }
}

/// Various options for configuring the transform gizmos.
#[derive(Resource, Copy, Clone, Debug)]
pub struct GizmoOptions {
    /// Modes to use in the gizmos.
    pub gizmo_modes: EnumSet<GizmoMode>,
    /// Orientation of the gizmo. This affects the behaviour of transformations.
    pub gizmo_orientation: GizmoOrientation,
    /// Orientation of the gizmo. This affects the behaviour of transformations.
    pub pivot_point: TransformPivotPoint,
    /// Look and feel of the gizmo.
    pub visuals: GizmoVisuals,
    /// Whether snapping is enabled in the gizmo transformations.
    /// This may be overwritten with hotkeys ([`GizmoHotkeys::enable_snapping`]).
    pub snapping: bool,
    /// When snapping is enabled, snap twice as often.
    /// This may be overwritten with hotkeys ([`GizmoHotkeys::enable_accurate_mode`]).
    pub accurate_mode: bool,
    /// Angle increment for snapping rotations, in radians.
    pub snap_angle: f32,
    /// Distance increment for snapping translations.
    pub snap_distance: f32,
    /// Scale increment for snapping scalings.
    pub snap_scale: f32,
    /// If `true`, all [`GizmoTarget`]s are transformed
    /// using a single gizmo. If `false`, each target
    /// has its own gizmo.
    pub group_targets: bool,
    /// If set, this mode is forced active and other modes are disabled.
    /// This may be overwritten with hotkeys.
    pub mode_override: Option<GizmoMode>,
    /// Hotkeys for easier interaction with the gizmo.
    pub hotkeys: Option<GizmoHotkeys>,
    /// Allows you to provide a custom viewport rect, which will be used to
    /// scale the cursor position. By default, this is set to `None` which means
    /// the full window size is used as the viewport.
    pub viewport_rect: Option<bevy::math::Rect>,
}

impl Default for GizmoOptions {
    fn default() -> Self {
        Self {
            gizmo_modes: GizmoMode::all(),
            gizmo_orientation: GizmoOrientation::default(),
            pivot_point: TransformPivotPoint::default(),
            visuals: Default::default(),
            snapping: false,
            accurate_mode: false,
            snap_angle: DEFAULT_SNAP_ANGLE,
            snap_distance: DEFAULT_SNAP_DISTANCE,
            snap_scale: DEFAULT_SNAP_SCALE,
            group_targets: true,
            mode_override: None,
            hotkeys: None,
            viewport_rect: None,
        }
    }
}

/// Hotkeys for easier interaction with the gizmo.
#[derive(Debug, Copy, Clone)]
pub struct GizmoHotkeys {
    /// When pressed, transformations snap to according to snap values
    /// specified in [`GizmoOptions`].
    pub enable_snapping: Option<KeyCode>,
    /// When pressed, snapping is twice as accurate.
    pub enable_accurate_mode: Option<KeyCode>,
    /// Toggles gizmo to rotate-only mode.
    pub toggle_rotate: Option<KeyCode>,
    /// Toggles gizmo to translate-only mode.
    pub toggle_translate: Option<KeyCode>,
    /// Toggles gizmo to scale-only mode.
    pub toggle_scale: Option<KeyCode>,
    /// Limits overridden gizmo mode to X axis only.
    pub toggle_x: Option<KeyCode>,
    /// Limits overridden gizmo mode to Y axis only.
    pub toggle_y: Option<KeyCode>,
    /// Limits overridden gizmo mode to Z axis only.
    pub toggle_z: Option<KeyCode>,
    /// When pressed, deactivates the gizmo if it
    /// was active.
    pub deactivate_gizmo: Option<KeyCode>,
    /// If true, a mouse click deactivates the gizmo if it
    /// was active.
    pub mouse_click_deactivates: bool,
}

impl Default for GizmoHotkeys {
    fn default() -> Self {
        Self {
            enable_snapping: Some(KeyCode::ControlLeft),
            enable_accurate_mode: Some(KeyCode::ShiftLeft),
            toggle_rotate: Some(KeyCode::KeyR),
            toggle_translate: Some(KeyCode::KeyG),
            toggle_scale: Some(KeyCode::KeyS),
            toggle_x: Some(KeyCode::KeyX),
            toggle_y: Some(KeyCode::KeyY),
            toggle_z: Some(KeyCode::KeyZ),
            deactivate_gizmo: Some(KeyCode::Escape),
            mouse_click_deactivates: true,
        }
    }
}

/// Marks an entity as a gizmo target.
///
/// When an entity has this component and a [`Transform`],
/// a gizmo is shown, which can be used to manipulate the
/// transform component.
///
/// If target grouping is enabled in [`GizmoOptions`],
/// a single gizmo is used for all targets. Otherwise
/// a separate gizmo is used for each target entity.
#[derive(Component, Copy, Clone, Debug, Default)]
pub struct GizmoTarget {
    /// Whether any part of the gizmo is currently focused.
    pub(crate) is_focused: bool,

    /// Whether the gizmo is currently being interacted with.
    pub(crate) is_active: bool,

    /// This gets replaced with the result of the most recent
    /// gizmo interaction that affected this entity.
    pub(crate) latest_result: Option<GizmoResult>,
}

impl GizmoTarget {
    /// Whether any part of the gizmo is currently focused.
    pub fn is_focused(&self) -> bool {
        self.is_focused
    }

    /// Whether the gizmo is currently being interacted with.
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// This gets replaced with the result of the most recent
    /// gizmo interaction that affected this entity.
    pub fn latest_result(&self) -> Option<GizmoResult> {
        self.latest_result
    }
}

/// Marks a [`GizmoTarget`] as a generic oriented bounds handle.
///
/// Bounds targets expose six explicit face handles. Face drags report a
/// [`GizmoResult::ResizeFace`] and are not applied to the target's ordinary
/// transform by the Bevy adapter; the host owns the bounds semantics. Other
/// gizmo results continue to update the target transform normally.
#[derive(Component, Copy, Clone, Debug, Default)]
pub struct BoundsGizmoTarget;

/// Marker used to specify which camera to use for gizmos.
#[derive(Component)]
pub struct GizmoCamera;

#[derive(Resource, Default)]
pub(crate) struct GizmoStorage {
    target_entities: Vec<Entity>,
    pub(crate) entity_gizmo_map: HashMap<Entity, Uuid>,
    pub(crate) gizmos: HashMap<Uuid, Gizmo>,
}

fn handle_hotkeys(
    mut gizmo_options: ResMut<GizmoOptions>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    mut axes: Local<EnumSet<GizmoDirection>>,
) {
    let Some(hotkeys) = gizmo_options.hotkeys else {
        // Hotkeys are disabled.
        return;
    };

    if let Some(snapping_key) = hotkeys.enable_snapping {
        gizmo_options.snapping = keyboard_input.pressed(snapping_key);
    }

    if let Some(accurate_mode_key) = hotkeys.enable_accurate_mode {
        gizmo_options.accurate_mode = keyboard_input.pressed(accurate_mode_key);
    }

    let invert_modifier = keyboard_input.pressed(KeyCode::ShiftLeft);

    let x_hotkey_pressed = hotkeys
        .toggle_x
        .is_some_and(|key| keyboard_input.just_pressed(key));
    let y_hotkey_pressed = hotkeys
        .toggle_y
        .is_some_and(|key| keyboard_input.just_pressed(key));
    let z_hotkey_pressed = hotkeys
        .toggle_z
        .is_some_and(|key| keyboard_input.just_pressed(key));

    let mut new_axes = EnumSet::empty();

    if x_hotkey_pressed {
        new_axes = if invert_modifier {
            enum_set!(GizmoDirection::Y | GizmoDirection::Z)
        } else {
            enum_set!(GizmoDirection::X)
        };
    };
    if y_hotkey_pressed {
        new_axes = if !invert_modifier {
            enum_set!(GizmoDirection::Y)
        } else {
            enum_set!(GizmoDirection::X | GizmoDirection::Z)
        };
    };
    if z_hotkey_pressed {
        new_axes = if !invert_modifier {
            enum_set!(GizmoDirection::Z)
        } else {
            enum_set!(GizmoDirection::X | GizmoDirection::Y)
        };
    };

    if !new_axes.is_empty() {
        if *axes == new_axes {
            axes.clear();
        } else {
            *axes = new_axes;
        }
    }

    if gizmo_options.mode_override.is_none() {
        axes.clear();
    }

    let rotate_hotkey_pressed = hotkeys
        .toggle_rotate
        .is_some_and(|key| keyboard_input.just_pressed(key));
    let translate_hotkey_pressed = hotkeys
        .toggle_translate
        .is_some_and(|key| keyboard_input.just_pressed(key));
    let scale_hotkey_pressed = hotkeys
        .toggle_scale
        .is_some_and(|key| keyboard_input.just_pressed(key));

    let mode_kind = if rotate_hotkey_pressed {
        if gizmo_options
            .mode_override
            .filter(GizmoMode::is_rotate)
            .is_some()
        {
            Some(GizmoModeKind::Arcball)
        } else {
            Some(GizmoModeKind::Rotate)
        }
    } else if translate_hotkey_pressed {
        Some(GizmoModeKind::Translate)
    } else if scale_hotkey_pressed {
        Some(GizmoModeKind::Scale)
    } else {
        gizmo_options.mode_override.map(|mode| mode.kind())
    };

    if let Some(kind) = mode_kind {
        gizmo_options.mode_override = GizmoMode::from_kind_and_axes(kind, *axes)
            .filter(|mode| gizmo_options.gizmo_modes.contains(*mode))
            .or_else(|| {
                GizmoMode::all_from_kind(kind)
                    .iter()
                    .find(|mode| gizmo_options.gizmo_modes.contains(*mode))
            });
    } else {
        gizmo_options.mode_override = None;
    }

    if (hotkeys.mouse_click_deactivates
        && mouse_input.any_just_pressed([MouseButton::Left, MouseButton::Right]))
        || hotkeys
            .deactivate_gizmo
            .is_some_and(|key| keyboard_input.just_pressed(key))
    {
        gizmo_options.mode_override = None;
    }
}

#[derive(Debug, Message, Default)]
pub struct GizmoDragStarted;
#[derive(Debug, Message, Default)]
pub struct GizmoDragging;

#[allow(clippy::too_many_arguments)]
fn update_gizmos(
    q_window: Query<&Window, With<PrimaryWindow>>,
    q_gizmo_camera: Query<(&Camera, &GlobalTransform), With<GizmoCamera>>,
    mut q_targets: Query<
        (
            Entity,
            &mut Transform,
            &mut GizmoTarget,
            Option<&BoundsGizmoTarget>,
        ),
        Without<GizmoCamera>,
    >,
    mut drag_started: MessageReader<GizmoDragStarted>,
    mut dragging: MessageReader<GizmoDragging>,
    gizmo_options: Res<GizmoOptions>,
    mut gizmo_storage: ResMut<GizmoStorage>,
    mut last_cursor_pos: Local<Vec2>,
    mut last_scaled_cursor_pos: Local<Vec2>,
    hover_map: Res<HoverMap>,
) {
    let Ok(window) = q_window.single() else {
        return;
    };

    let mut cursor_pos = window.cursor_position().unwrap_or_else(|| *last_cursor_pos);
    *last_cursor_pos = cursor_pos;

    let scale_factor = window.scale_factor();

    let (camera, camera_transform) = {
        let mut active_camera = None;

        for camera in q_gizmo_camera.iter() {
            if !camera.0.is_active {
                continue;
            }
            if active_camera.is_some() {
                bevy::log::warn!("Only one camera with a GizmoCamera component is supported.");
                return;
            }
            active_camera = Some(camera);
        }

        match active_camera {
            Some(camera) => camera,
            None => return,
        }
    };

    let Some(viewport) = camera.logical_viewport_rect() else {
        return;
    };

    if let Some(custom_viewport) = gizmo_options.viewport_rect {
        let vp_ratio = viewport.size() / custom_viewport.size();
        let mut scaled_cursor_pos = (cursor_pos - (custom_viewport.min - viewport.min)) * vp_ratio;
        if !viewport.contains(scaled_cursor_pos) {
            scaled_cursor_pos = *last_scaled_cursor_pos;
        }
        *last_scaled_cursor_pos = scaled_cursor_pos;
        cursor_pos = scaled_cursor_pos;
    };

    let viewport = Rect::from_min_max(
        Pos2::new(viewport.min.x, viewport.min.y),
        Pos2::new(viewport.max.x, viewport.max.y),
    );

    let projection_matrix = camera.clip_from_view();

    let view_matrix = camera_transform.to_matrix().inverse();

    let mut snap_angle = gizmo_options.snap_angle;
    let mut snap_distance = gizmo_options.snap_distance;
    let mut snap_scale = gizmo_options.snap_scale;

    if gizmo_options.accurate_mode {
        snap_angle /= 2.0;
        snap_distance /= 2.0;
        snap_scale /= 2.0;
    }

    let gizmo_config = GizmoConfig {
        view_matrix: view_matrix.as_dmat4().into(),
        projection_matrix: projection_matrix.as_dmat4().into(),
        viewport,
        modes: gizmo_options.gizmo_modes,
        mode_override: gizmo_options.mode_override,
        orientation: gizmo_options.gizmo_orientation,
        pivot_point: gizmo_options.pivot_point,
        visuals: gizmo_options.visuals,
        snapping: gizmo_options.snapping,
        snap_angle,
        snap_distance,
        snap_scale,
        pixels_per_point: scale_factor,
        bounds_faces: false,
        bounds_min_thickness: core::DEFAULT_BOUNDS_MIN_THICKNESS,
    };

    let any_gizmo_hovered = q_targets
        .iter()
        .any(|(entity, ..)| hover_map.iter().any(|(_, map)| map.contains_key(&entity)));

    let hovered = any_gizmo_hovered || gizmo_options.mode_override.is_some();

    let gizmo_interaction = GizmoInteraction {
        cursor_pos: (cursor_pos.x, cursor_pos.y),
        hovered,
        drag_started: drag_started.read().len() > 0,
        dragging: dragging.read().len() > 0,
    };

    let mut target_entities: Vec<Entity> = vec![];
    let mut target_transforms: Vec<Transform> = vec![];
    let mut bounds_face_target_count = 0;

    for (entity, mut target_transform, mut gizmo_target, bounds_target) in &mut q_targets {
        target_entities.push(entity);
        target_transforms.push(*target_transform);
        if bounds_target.is_some() {
            bounds_face_target_count += 1;
        }

        if gizmo_options.group_targets {
            gizmo_storage
                .entity_gizmo_map
                .insert(entity, GIZMO_GROUP_UUID);
            continue;
        }

        let mut gizmo_uuid = *gizmo_storage
            .entity_gizmo_map
            .entry(entity)
            .or_insert_with(Uuid::new_v4);

        if gizmo_uuid == GIZMO_GROUP_UUID {
            gizmo_uuid = Uuid::new_v4();
            gizmo_storage.entity_gizmo_map.insert(entity, gizmo_uuid);
        }

        let gizmo = gizmo_storage.gizmos.entry(gizmo_uuid).or_default();
        let mut target_config = gizmo_config;
        target_config.bounds_faces = bounds_target.is_some();
        gizmo.update_config(target_config);

        let gizmo_result = gizmo.update(
            gizmo_interaction,
            &[math::Transform {
                translation: target_transform.translation.as_dvec3().into(),
                rotation: target_transform.rotation.as_dquat().into(),
                scale: target_transform.scale.as_dvec3().into(),
            }],
        );

        let is_focused = gizmo.is_focused();

        gizmo_target.is_active = gizmo_result.is_some();
        gizmo_target.is_focused = is_focused;

        if let Some((result, updated_targets)) = &gizmo_result {
            let Some(result_transform) = updated_targets.first() else {
                bevy::log::warn!("No transform found in GizmoResult!");
                continue;
            };

            apply_result_transform(&mut target_transform, *result, *result_transform);
        }

        gizmo_target.latest_result = gizmo_result.map(|(result, _)| result);
    }

    if gizmo_options.group_targets {
        let gizmo = gizmo_storage.gizmos.entry(GIZMO_GROUP_UUID).or_default();
        let bounds_faces =
            !target_transforms.is_empty() && bounds_face_target_count == target_transforms.len();
        let mut group_config = gizmo_config;
        group_config.bounds_faces = bounds_faces;
        gizmo.update_config(group_config);

        let gizmo_result = gizmo.update(
            gizmo_interaction,
            target_transforms
                .iter()
                .map(|transform| math::Transform {
                    translation: transform.translation.as_dvec3().into(),
                    rotation: transform.rotation.as_dquat().into(),
                    scale: transform.scale.as_dvec3().into(),
                })
                .collect::<Vec<_>>()
                .as_slice(),
        );

        let is_focused = gizmo.is_focused();

        for (i, (_, mut target_transform, mut gizmo_target, _)) in q_targets.iter_mut().enumerate()
        {
            gizmo_target.is_active = gizmo_result.is_some();
            gizmo_target.is_focused = is_focused;

            if let Some((result, updated_targets)) = &gizmo_result {
                let Some(result_transform) = updated_targets.get(i) else {
                    bevy::log::warn!("No transform {i} found in GizmoResult!");
                    continue;
                };

                apply_result_transform(&mut target_transform, *result, *result_transform);
            }

            gizmo_target.latest_result = gizmo_result.as_ref().map(|(result, _)| *result);
        }
    }

    gizmo_storage.target_entities = target_entities;
}

fn draw_gizmos(
    gizmo_storage: Res<GizmoStorage>,
    mut draw_data_assets: ResMut<Assets<render::GizmoDrawData>>,
    mut draw_data_handles: ResMut<DrawDataHandles>,
) {
    for (gizmo_uuid, gizmo) in &gizmo_storage.gizmos {
        let draw_data = gizmo.draw();

        let handle = draw_data_handles
            .handles
            .entry(*gizmo_uuid)
            .or_insert_with(|| {
                draw_data_assets
                    .add(render::GizmoDrawData::default())
                    .into()
            })
            .clone();
        let mut asset = draw_data_assets.get_mut(&handle).unwrap();

        let viewport = &gizmo.config().viewport;

        asset.0.vertices.clear();
        asset
            .0
            .vertices
            .extend(draw_data.vertices.into_iter().map(|vert| {
                [
                    ((vert[0] - viewport.left()) / viewport.width()) * 2.0 - 1.0,
                    ((vert[1] - viewport.top()) / viewport.height()) * 2.0 - 1.0,
                ]
            }));

        asset.0.colors = draw_data.colors;
        asset.0.indices = draw_data.indices;
    }
}

/// Settings for [`auto_scale_gizmo_to_target`]. The system reads these
/// every frame, so they can be tweaked at runtime via the resource.
#[derive(Resource, Debug, Clone, Copy)]
pub struct GizmoAutoScale {
    /// Gizmo's outer ring matches this fraction of the target's
    /// projected screen radius. `0.8` keeps the handles tucked just
    /// inside the bounding box.
    pub object_fraction: f32,
    /// Hard ceiling on the gizmo's screen size. Prevents the gizmo
    /// from spilling off the viewport when the camera is right on top
    /// of the target.
    pub max_pixels: f32,
    /// Hard floor on the gizmo's screen size. Keep this small (or 0)
    /// so the gizmo genuinely shrinks when the target is far away.
    pub min_pixels: f32,
    /// Fallback world-radius used when a [`GizmoTarget`] entity has
    /// no [`Aabb`] component (e.g. an empty proxy entity).
    pub fallback_world_radius: f32,
}

impl Default for GizmoAutoScale {
    fn default() -> Self {
        Self {
            object_fraction: 0.8,
            max_pixels: 200.0,
            min_pixels: 6.0,
            fallback_world_radius: 1.0,
        }
    }
}

/// Per-frame system: size the gizmo so its outer ring tracks the
/// selected target's apparent screen size. Object near the camera ⇒
/// big gizmo (capped); object far away ⇒ small gizmo (floored).
///
/// Add this in your `Update` schedule (it runs before the upstream
/// gizmo's `Last`-schedule pipeline picks up `GizmoOptions`):
///
/// ```ignore
/// app.add_systems(Update, bevy_glacial::auto_scale_gizmo_to_target);
/// ```
///
/// Without this system the upstream `transform-gizmo` keeps the
/// handles at a constant pixel size, which dwarfs distant objects.
pub fn auto_scale_gizmo_to_target(
    settings: Res<GizmoAutoScale>,
    cameras: Query<(&GlobalTransform, &Projection), With<GizmoCamera>>,
    targets: Query<(&GlobalTransform, Option<&Aabb>), With<GizmoTarget>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut options: ResMut<GizmoOptions>,
) {
    let Ok((cam_gt, projection)) = cameras.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };

    let mut max_screen_radius: f32 = 0.0;
    for (target_gt, aabb) in &targets {
        let world_radius = aabb
            .map(|a| a.half_extents.x.max(a.half_extents.y).max(a.half_extents.z) as f32)
            .unwrap_or(settings.fallback_world_radius);

        let distance = (cam_gt.translation() - target_gt.translation())
            .length()
            .max(0.01);

        let screen_radius = match projection {
            Projection::Perspective(p) => {
                let half_h_world = distance * (p.fov * 0.5).tan();
                (world_radius / half_h_world) * (window.height() * 0.5)
            }
            // Orthographic: world-to-pixel ratio is constant. Pick a
            // mid-range fallback rather than try to dig the scale out
            // of the projection matrix.
            _ => 100.0,
        };
        max_screen_radius = max_screen_radius.max(screen_radius);
    }

    if max_screen_radius == 0.0 {
        return;
    }

    options.visuals.gizmo_size = (max_screen_radius * settings.object_fraction)
        .clamp(settings.min_pixels, settings.max_pixels);
}

fn cleanup_old_data(
    gizmo_options: Res<GizmoOptions>,
    mut gizmo_storage: ResMut<GizmoStorage>,
    mut draw_data_handles: ResMut<DrawDataHandles>,
) {
    let target_entities = std::mem::take(&mut gizmo_storage.target_entities);

    let mut gizmos_to_keep = vec![];

    if gizmo_options.group_targets && !target_entities.is_empty() {
        gizmos_to_keep.push(GIZMO_GROUP_UUID);
    }

    gizmo_storage.entity_gizmo_map.retain(|entity, uuid| {
        if !target_entities.contains(entity) {
            false
        } else {
            gizmos_to_keep.push(*uuid);

            true
        }
    });

    gizmo_storage
        .gizmos
        .retain(|uuid, _| gizmos_to_keep.contains(uuid));

    draw_data_handles
        .handles
        .retain(|uuid, _| gizmos_to_keep.contains(uuid));
}
