use bevy::{
    app::{App, Plugin},
    asset::load_internal_asset,
    core_pipeline::{
        bloom::BloomPlugin,
        clear_color::{ClearColor, ClearColorConfig},
        core_2d::Core2dPlugin,
        core_3d::{
            extract_core_3d_camera_phases, graph, AlphaMask3d, Camera3d, Camera3dDepthLoadOp,
            MainPass3dNode, Opaque3d, Transparent3d,
        },
        fullscreen_vertex_shader::FULLSCREEN_SHADER_HANDLE,
        fxaa::FxaaPlugin,
        tonemapping::{TonemappingNode, TonemappingPlugin},
        upscaling::{UpscalingNode, UpscalingPlugin},
    },
    ecs::{
        entity::Entity,
        query::With,
        system::{Commands, Query, Res, ResMut},
    },
    render::{
        camera::ExtractedCamera,
        extract_component::ExtractComponentPlugin,
        extract_resource::ExtractResourcePlugin,
        render_graph::{EmptyNode, RenderGraph, SlotInfo, SlotType},
        render_phase::{sort_phase_system, DrawFunctions, RenderPhase},
        render_resource::{
            Extent3d, Shader, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderDevice,
        texture::TextureCache,
        view::{Msaa, ViewDepthTexture},
        RenderApp, RenderStage,
    },
    utils::HashMap,
};

#[derive(Default)]
pub struct CorePipelinePlugin;

impl Plugin for CorePipelinePlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            FULLSCREEN_SHADER_HANDLE,
            "core_pipeline_fullscreen.wgsl",
            Shader::from_wgsl
        );

        app.register_type::<ClearColor>()
            .register_type::<ClearColorConfig>()
            .init_resource::<ClearColor>()
            .add_plugin(ExtractResourcePlugin::<ClearColor>::default())
            .add_plugin(Core2dPlugin)
            .add_plugin(Core3dPlugin)
            .add_plugin(TonemappingPlugin)
            .add_plugin(UpscalingPlugin)
            .add_plugin(BloomPlugin)
            .add_plugin(FxaaPlugin);
    }
}

pub struct Core3dPlugin;

impl Plugin for Core3dPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Camera3d>()
            .register_type::<Camera3dDepthLoadOp>()
            .add_plugin(ExtractComponentPlugin::<Camera3d>::default());

        let render_app = match app.get_sub_app_mut(RenderApp) {
            Ok(render_app) => render_app,
            Err(_) => return,
        };

        render_app
            .init_resource::<DrawFunctions<Opaque3d>>()
            .init_resource::<DrawFunctions<AlphaMask3d>>()
            .init_resource::<DrawFunctions<Transparent3d>>()
            .add_system_to_stage(RenderStage::Extract, extract_core_3d_camera_phases)
            .add_system_to_stage(RenderStage::Prepare, prepare_core_3d_depth_textures)
            .add_system_to_stage(RenderStage::PhaseSort, sort_phase_system::<Opaque3d>)
            .add_system_to_stage(RenderStage::PhaseSort, sort_phase_system::<AlphaMask3d>)
            .add_system_to_stage(RenderStage::PhaseSort, sort_phase_system::<Transparent3d>);

        let pass_node_3d = MainPass3dNode::new(&mut render_app.world);
        let tonemapping = TonemappingNode::new(&mut render_app.world);
        let upscaling = UpscalingNode::new(&mut render_app.world);
        let mut graph = render_app.world.resource_mut::<RenderGraph>();

        let mut draw_3d_graph = RenderGraph::default();
        draw_3d_graph.add_node(graph::node::MAIN_PASS, pass_node_3d);
        draw_3d_graph.add_node(graph::node::TONEMAPPING, tonemapping);
        draw_3d_graph.add_node(graph::node::END_MAIN_PASS_POST_PROCESSING, EmptyNode);
        draw_3d_graph.add_node(graph::node::UPSCALING, upscaling);
        let input_node_id = draw_3d_graph.set_input(vec![SlotInfo::new(
            graph::input::VIEW_ENTITY,
            SlotType::Entity,
        )]);
        draw_3d_graph
            .add_slot_edge(
                input_node_id,
                graph::input::VIEW_ENTITY,
                graph::node::MAIN_PASS,
                MainPass3dNode::IN_VIEW,
            )
            .unwrap();
        draw_3d_graph
            .add_slot_edge(
                input_node_id,
                graph::input::VIEW_ENTITY,
                graph::node::TONEMAPPING,
                TonemappingNode::IN_VIEW,
            )
            .unwrap();
        draw_3d_graph
            .add_slot_edge(
                input_node_id,
                graph::input::VIEW_ENTITY,
                graph::node::UPSCALING,
                UpscalingNode::IN_VIEW,
            )
            .unwrap();
        draw_3d_graph
            .add_node_edge(graph::node::MAIN_PASS, graph::node::TONEMAPPING)
            .unwrap();
        draw_3d_graph
            .add_node_edge(
                graph::node::TONEMAPPING,
                graph::node::END_MAIN_PASS_POST_PROCESSING,
            )
            .unwrap();
        draw_3d_graph
            .add_node_edge(
                graph::node::END_MAIN_PASS_POST_PROCESSING,
                graph::node::UPSCALING,
            )
            .unwrap();
        graph.add_sub_graph(graph::NAME, draw_3d_graph);
    }
}

#[allow(clippy::type_complexity)]
pub fn prepare_core_3d_depth_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    msaa: Res<Msaa>,
    render_device: Res<RenderDevice>,
    views_3d: Query<
        (Entity, &ExtractedCamera),
        (
            With<RenderPhase<Opaque3d>>,
            With<RenderPhase<AlphaMask3d>>,
            With<RenderPhase<Transparent3d>>,
        ),
    >,
) {
    let mut textures = HashMap::default();
    for (entity, camera) in &views_3d {
        if let Some(physical_target_size) = camera.physical_target_size {
            let cached_texture = textures
                .entry(camera.target.clone())
                .or_insert_with(|| {
                    texture_cache.get(
                        &render_device,
                        TextureDescriptor {
                            label: Some("view_depth_texture"),
                            size: Extent3d {
                                depth_or_array_layers: 1,
                                width: physical_target_size.x,
                                height: physical_target_size.y,
                            },
                            mip_level_count: 1,
                            sample_count: msaa.samples,
                            dimension: TextureDimension::D2,
                            format: TextureFormat::Depth32Float, /* PERF: vulkan docs recommend using 24
                                                                  * bit depth for better performance */
                            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                        },
                    )
                })
                .clone();
            commands.entity(entity).insert(ViewDepthTexture {
                texture: cached_texture.texture,
                view: cached_texture.default_view,
            });
        }
    }
}
