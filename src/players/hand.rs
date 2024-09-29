use std::collections::HashMap;

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockFace, BlockId, BlockPosition, BlockRotation, BlockState, Blocks, Friction},
    items::Items,
    models::{Model, ModelAnimations, ModelBundle, ModelMap, ModelVisibility, Models},
    networking::NetworkMessage,
    physics::shapes::Aabb,
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    utils,
    world::{chunk::Chunk, BlockUpdate, WorldMap},
};

use crate::{
    items::{GroundItemBundle, ItemUses, RegisterItemUse, UsableItems},
    players::{EquippedItem, Inventory},
};

pub struct HandPlugin;
impl Plugin for HandPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BlockBreakingEvent>().add_systems(
            Update,
            (
                handle_left_clicks,
                handle_right_clicks.in_set(RegisterItemUse),
                break_blocks.after(handle_left_clicks),
            ),
        );
    }
}

/// Together with an Aabb this tracks when a player right clicks an entity
#[derive(Component, Default)]
pub struct HandInteractions {
    player_entities: Vec<Entity>,
}

impl HandInteractions {
    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.player_entities.drain(..)
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.player_entities.push(player_entity);
    }
}

#[derive(Event, Hash, Eq, PartialEq)]
struct BlockBreakingEvent {
    player_entity: Entity,
    block_position: IVec3,
    block_id: BlockId,
}

// Keeps the state of how far along a block is to breaking
#[derive(Debug)]
struct BreakingBlock {
    model_entity: Entity,
    progress: f32,
    prev_hit: std::time::Instant,
}

#[derive(Component)]
struct BreakingBlockMarker;

// TODO: Take into account player's equipped item
fn break_blocks(
    mut commands: Commands,
    items: Res<Items>,
    models: Res<Models>,
    player_equipped_item_query: Query<(&Inventory, &EquippedItem), With<Player>>,
    mut model_query: Query<(&mut Model, &mut ModelVisibility), With<BreakingBlockMarker>>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut block_breaking_events: EventReader<BlockBreakingEvent>,
    mut being_broken: Local<HashMap<IVec3, BreakingBlock>>,
) {
    let now = std::time::Instant::now();

    let blocks = Blocks::get();

    for breaking_event in block_breaking_events.read() {
        // Guard against duplicate events, many left clicks often arrive at once.
        if let Some(breaking_block) = being_broken.get(&breaking_event.block_position) {
            if now == breaking_block.prev_hit {
                continue;
            }
        }

        let (inventory, equipped_item_index) = player_equipped_item_query
            .get(breaking_event.player_entity)
            .unwrap();
        let equipped_item_stack = &inventory[equipped_item_index.0];
        let tool = if let Some(item) = equipped_item_stack.item() {
            let equipped_item_config = items.get_config(&item.id);
            equipped_item_config.tool.as_ref()
        } else {
            None
        };

        let block_config = blocks.get_config(&breaking_event.block_id);

        // Unbreakable block
        if block_config.hardness.is_none() {
            continue;
        }

        if let Some(breaking_block) = being_broken.get_mut(&breaking_event.block_position) {
            if (now - breaking_block.prev_hit).as_secs_f32() > 0.05 {
                // The interval between two clicks needs to be short in order to be counted as
                // holding the button down.
                breaking_block.prev_hit = now;
                continue;
            }

            let (mut model, mut visibility) =
                model_query.get_mut(breaking_block.model_entity).unwrap();

            let prev_progress = breaking_block.progress;

            // Hardness is 'time to break'. We know it's Some because only blocks with hardness can
            // be hit.
            breaking_block.progress += (now - breaking_block.prev_hit).as_secs_f32()
                / block_config.hardness.unwrap()
                * tool.map(|t| t.efficiency).unwrap_or(1.0);
            breaking_block.prev_hit = now;

            let progress = breaking_block.progress;

            let Model::Custom {
                ref mut material_parallax_texture,
                ..
            } = *model
            else {
                unreachable!()
            };

            // Ordering from high to low lets it skip stages.
            if progress >= 1.0 {
                block_update_writer.send(BlockUpdate::Change {
                    position: breaking_event.block_position,
                    block_id: blocks.get_id("air"),
                    block_state: None,
                });

                let block_config = blocks.get_config(&breaking_event.block_id);
                let (dropped_item_id, count) =
                    match block_config.drop(tool.map(|t| t.name.as_str())) {
                        Some(drop) => drop,
                        None => continue,
                    };
                let item_config = items.get_config(&dropped_item_id);
                let model_config = models.get_by_id(item_config.model_id);

                commands.spawn(GroundItemBundle::new(
                    dropped_item_id,
                    item_config,
                    model_config,
                    count,
                    breaking_event.block_position.as_dvec3(),
                ));
            } else if prev_progress < 0.9 && progress > 0.9 {
                *material_parallax_texture = Some("blocks/breaking_9.png".to_owned());
            } else if prev_progress < 0.8 && progress > 0.8 {
                *material_parallax_texture = Some("blocks/breaking_8.png".to_owned());
            } else if prev_progress < 0.7 && progress > 0.7 {
                *material_parallax_texture = Some("blocks/breaking_7.png".to_owned());
            } else if prev_progress < 0.6 && progress > 0.6 {
                *material_parallax_texture = Some("blocks/breaking_6.png".to_owned());
            } else if prev_progress < 0.5 && progress > 0.5 {
                *material_parallax_texture = Some("blocks/breaking_5.png".to_owned());
            } else if prev_progress < 0.4 && progress > 0.4 {
                *material_parallax_texture = Some("blocks/breaking_4.png".to_owned());
            } else if prev_progress < 0.3 && progress > 0.3 {
                *material_parallax_texture = Some("blocks/breaking_3.png".to_owned());
            } else if prev_progress < 0.2 && progress > 0.2 {
                *material_parallax_texture = Some("blocks/breaking_2.png".to_owned());
            } else if prev_progress < 0.1 && progress > 0.1 {
                visibility.is_visible = true;
            }
        } else if block_config.hardness.unwrap() == 0.0 {
            // Blocks that break instantly
            block_update_writer.send(BlockUpdate::Change {
                position: breaking_event.block_position,
                block_id: blocks.get_id("air"),
                block_state: None,
            });

            let block_config = blocks.get_config(&breaking_event.block_id);
            let (dropped_item_id, count) = match block_config.drop(tool.map(|t| t.name.as_str())) {
                Some(drop) => drop,
                None => continue,
            };
            let item_config = items.get_config(&dropped_item_id);
            let model_config = models.get_by_id(item_config.model_id);

            commands.spawn(GroundItemBundle::new(
                dropped_item_id,
                item_config,
                model_config,
                count,
                breaking_event.block_position.as_dvec3(),
            ));

            // Guard against the block being broken again on the same tick
            being_broken.insert(
                breaking_event.block_position,
                BreakingBlock {
                    model_entity: commands.spawn_empty().id(),
                    progress: 1.0,
                    prev_hit: now,
                },
            );
        } else {
            let model_entity = commands
                .spawn(ModelBundle {
                    model: build_breaking_model(),
                    animations: ModelAnimations::default(),
                    // The model shouldn't show until some progress has been made
                    visibility: ModelVisibility { is_visible: false },
                    global_transform: GlobalTransform::default(),
                    transform: Transform::from_translation(
                        breaking_event.block_position.as_dvec3(),
                    ),
                })
                .insert(BreakingBlockMarker)
                .id();

            being_broken.insert(
                breaking_event.block_position,
                BreakingBlock {
                    model_entity,
                    progress: 0.0,
                    prev_hit: now,
                },
            );
        }
    }

    // Remove break progress after not being hit for 0.5 seconds.
    being_broken.retain(|_, breaking_block| {
        let remove_timout = (now - breaking_block.prev_hit).as_secs_f32() > 0.5;
        let remove_broken = breaking_block.progress >= 1.0;

        if remove_timout || remove_broken {
            commands.entity(breaking_block.model_entity).despawn();
            return false;
        } else {
            return true;
        }
    });
}

// TODO: This needs to be built from the model of what it is breaking. Means we have to load and
// store the quad info for each block including through gltfs
fn build_breaking_model() -> Model {
    let mesh_vertices = vec![
        // Top
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        // Back
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        // Left
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        // Right
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        // Front
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        // Bottom
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ];

    let mesh_normals = vec![
        // Top
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        // Back
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        // Left
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        // Right
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        // Front
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        // Bottom
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
    ];

    const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
    let mut mesh_uvs = Vec::new();
    for _ in 0..6 {
        mesh_uvs.extend(UVS);
    }

    const INDICES: [u32; 6] = [0, 1, 2, 2, 1, 3];
    let mut mesh_indices = Vec::new();
    for i in 0..6 {
        mesh_indices.extend(INDICES.iter().map(|x| x + 4 * i));
    }

    Model::Custom {
        mesh_indices,
        mesh_vertices,
        mesh_normals,
        mesh_uvs: Some(mesh_uvs),
        material_base_color: "FFFFFF".to_owned(),
        material_color_texture: None,
        material_parallax_texture: Some("blocks/breaking_1.png".to_owned()),
        material_alpha_mode: 2,
        material_alpha_cutoff: 0.0,
        material_double_sided: false,
    }
}

// Left clicks are used for block breaking or attacking.
// TODO: Need spatial partitioning of item/mobs/players to do hit detection.
fn handle_left_clicks(
    mut clicks: EventReader<NetworkMessage<messages::LeftClick>>,
    world_map: Res<WorldMap>,
    player_query: Query<(&GlobalTransform, &Camera)>,
    model_map: Res<ModelMap>,
    model_query: Query<(Option<&Aabb>, &GlobalTransform, Option<&BlockPosition>), With<Model>>,
    mut block_breaking_events: EventWriter<BlockBreakingEvent>,
) {
    let blocks = Blocks::get();

    for click in clicks.read() {
        let (player_position, player_camera) = player_query.get(click.player_entity).unwrap();

        let camera_transform = Transform {
            translation: player_position.translation() + player_camera.translation,
            rotation: player_camera.rotation,
            ..default()
        };

        // Test hits for models in all adjacent chunks.
        let mut model_hit = None;
        let chunk_position = utils::world_position_to_chunk_position(
            player_position.translation().floor().as_ivec3(),
        );
        for x_offset in [IVec3::X, IVec3::NEG_X, IVec3::ZERO] {
            for y_offset in [IVec3::Y, IVec3::NEG_Y, IVec3::ZERO] {
                for z_offset in [IVec3::Z, IVec3::NEG_Z, IVec3::ZERO] {
                    let chunk_position = chunk_position
                        + x_offset * Chunk::SIZE as i32
                        + y_offset * Chunk::SIZE as i32
                        + z_offset * Chunk::SIZE as i32;
                    let Some(model_entities) = model_map.get_entities(&chunk_position) else {
                        continue;
                    };
                    for model_entity in model_entities {
                        let Ok((_, model_transform, maybe_block)) = model_query.get(*model_entity)
                        else {
                            continue;
                        };

                        let Some(block_position) = maybe_block else {
                            continue;
                        };

                        let block_id = world_map.get_block(block_position.0).unwrap();
                        let block_config = blocks.get_config(&block_id);

                        let Some(hitbox) = &block_config.hitbox else {
                            continue;
                        };

                        let Some(distance) = hitbox.ray_intersection(
                            camera_transform.translation,
                            camera_transform.forward(),
                            model_transform.compute_transform(),
                        ) else {
                            continue;
                        };

                        if let Some((_, _, closest_distance)) = model_hit {
                            if distance < closest_distance {
                                model_hit = Some((block_position.0, block_id, distance));
                            }
                        } else {
                            model_hit = Some((block_position.0, block_id, distance));
                        }
                    }
                }
            }
        }

        let block_hit = world_map.raycast_to_block(&camera_transform, 5.0);

        let (block_position, block_id) = if block_hit.is_some() && model_hit.is_some() {
            let (model_position, model_block_id, model_distance) = model_hit.unwrap();
            let (block_position, block_id, _, block_distance) = block_hit.unwrap();

            if model_distance < block_distance {
                (model_position, model_block_id)
            } else {
                (block_position, block_id)
            }
        } else if let Some((model_position, model_block_id, _)) = model_hit {
            dbg!(model_position);
            (model_position, model_block_id)
        } else if let Some((block_position, block_id, _, _)) = block_hit {
            (block_position, block_id)
        } else {
            continue;
        };

        block_breaking_events.send(BlockBreakingEvent {
            player_entity: click.player_entity,
            block_position,
            block_id,
        });
    }
}

fn handle_right_clicks(
    world_map: Res<WorldMap>,
    items: Res<Items>,
    usable_items: Res<UsableItems>,
    model_map: Res<ModelMap>,
    model_query: Query<(&Aabb, &GlobalTransform), With<Model>>,
    mut player_query: Query<
        (&mut Inventory, &EquippedItem, &GlobalTransform, &Camera),
        With<Player>,
    >,
    mut item_use_query: Query<&mut ItemUses>,
    mut hand_interaction_query: Query<&mut HandInteractions>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut clicks: EventReader<NetworkMessage<messages::RightClick>>,
) {
    for right_click in clicks.read() {
        let (mut inventory, equipped_item, player_position, player_camera) =
            player_query.get_mut(right_click.player_entity).unwrap();

        let camera_transform = Transform {
            translation: player_position.translation() + player_camera.translation,
            rotation: player_camera.rotation,
            ..default()
        };

        let block_hit = world_map.raycast_to_block(&camera_transform, 5.0);

        let block_hit_distance = if let Some((_, _, _, distance)) = block_hit {
            distance
        } else {
            f64::MAX
        };

        // Test hits for models in all adjacent chunks.
        let mut model_hit = None;
        let chunk_position = utils::world_position_to_chunk_position(
            player_position.translation().floor().as_ivec3(),
        );
        for x_offset in [IVec3::X, IVec3::NEG_X, IVec3::ZERO] {
            for y_offset in [IVec3::Y, IVec3::NEG_Y, IVec3::ZERO] {
                for z_offset in [IVec3::Z, IVec3::NEG_Z, IVec3::ZERO] {
                    let chunk_position = chunk_position
                        + x_offset * Chunk::SIZE as i32
                        + y_offset * Chunk::SIZE as i32
                        + z_offset * Chunk::SIZE as i32;
                    let Some(model_entities) = model_map.get_entities(&chunk_position) else {
                        continue;
                    };
                    for model_entity in model_entities {
                        let Ok((aabb, model_transform)) = model_query.get(*model_entity) else {
                            continue;
                        };

                        let aabb = Aabb {
                            center: aabb.center + model_transform.translation(),
                            half_extents: aabb.half_extents,
                        };

                        let Some(distance) = aabb.ray_intersection(
                            camera_transform.translation,
                            camera_transform.forward(),
                        ) else {
                            continue;
                        };

                        if block_hit_distance < distance {
                            continue;
                        }

                        if let Some((_, closest_distance)) = model_hit {
                            if distance < closest_distance {
                                model_hit = Some((*model_entity, distance));
                            }
                        } else {
                            model_hit = Some((*model_entity, distance));
                        }
                    }
                }
            }
        }

        if let Some((model_entity, _distance)) = model_hit {
            if let Ok(mut hand_interaction) = hand_interaction_query.get_mut(model_entity) {
                hand_interaction.push(right_click.player_entity);
            }
            continue;
        }

        let Some((block_pos, block_id, block_face, _)) = block_hit else {
            continue;
        };

        // TODO: Needs an override, sneak = always place block
        // If the block can be interacted with, the click always counts as an interaction
        let (chunk_position, block_index) =
            utils::world_position_to_chunk_position_and_block_index(block_pos);
        let chunk = world_map.get_chunk(&chunk_position).unwrap();
        if let Some(block_entity) = chunk.block_entities.get(&block_index) {
            if let Ok(mut interactions) = hand_interaction_query.get_mut(*block_entity) {
                interactions.push(right_click.player_entity);
                continue;
            }
        }

        let equipped_item = &mut inventory[equipped_item.0];

        if equipped_item.is_empty() {
            continue;
        }

        let item_id = equipped_item.item().unwrap().id;

        if let Some(item_use_entity) = usable_items.get(&item_id) {
            let mut uses = item_use_query.get_mut(*item_use_entity).unwrap();
            uses.push(
                right_click.player_entity,
                block_hit.map(|(block_position, block_id, _, _)| (block_id, block_position)),
            );
        }

        let blocks = Blocks::get();

        let replaced_block_position = if blocks.get_config(&block_id).replaceable {
            block_pos
        } else if let Some(block_id) = world_map.get_block(block_face.shift_position(block_pos)) {
            if !blocks.get_config(&block_id).replaceable {
                continue;
            }
            block_face.shift_position(block_pos)
        } else {
            continue;
        };

        let item_config = items.get_config(&item_id);

        let Some(block_id) = item_config.block else {
            continue;
        };

        equipped_item.subtract(1);

        let block_config = blocks.get_config(&block_id);
        let block_state = if block_config.placement.rotatable
            || (block_config.placement.side_transform.is_some()
                && block_face != BlockFace::Top
                && block_face != BlockFace::Bottom)
        {
            if (block_face == BlockFace::Bottom && block_config.placement.ceiling)
                || (block_face == BlockFace::Top && block_config.placement.floor)
            {
                // If the bottom or top face is clicked it should rotate the block based on the
                // relative distance to the block clicked so that the block is always facing the
                // player.
                let distance = player_position.translation().as_ivec3() - block_pos;
                let max = IVec2::new(distance.x, distance.z).max_element();

                if max == distance.x {
                    if distance.x.is_positive() {
                        Some(BlockState::new(BlockRotation::Once))
                    } else {
                        Some(BlockState::new(BlockRotation::Thrice))
                    }
                } else if max == distance.z {
                    if distance.z.is_positive() {
                        None
                    } else {
                        Some(BlockState::new(BlockRotation::Twice))
                    }
                } else {
                    unreachable!()
                }
            } else if (block_face != BlockFace::Bottom || block_face != BlockFace::Top)
                && block_config.placement.sides
            {
                Some(BlockState::new(block_face.to_rotation()))
            } else {
                None
            }
        } else {
            None
        };

        block_update_writer.send(BlockUpdate::Change {
            position: replaced_block_position,
            block_id,
            block_state,
        });
    }
}
