use fmc::{
    interfaces::{
        InterfaceEventRegistration, InterfaceInteractionEvents, RegisterInterfaceProvider,
    },
    networking::{NetworkMessage, Server},
    players::Player,
    prelude::*,
    protocol::messages,
};

use serde::{Deserialize, Serialize};

use super::RespawnEvent;

pub struct HealthPlugin;
impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<DamageEvent>()
            .add_event::<HealEvent>()
            .add_systems(
                Update,
                (
                    register_death_interface,
                    change_health,
                    fall_damage.before(change_health),
                    death_interface.after(InterfaceEventRegistration),
                ),
            );
    }
}

#[derive(Default, Bundle)]
pub struct HealthBundle {
    health: Health,
    fall_damage: FallDamage,
}

impl HealthBundle {
    pub fn from_health(health: Health) -> Self {
        Self {
            health,
            ..default()
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone)]
pub struct Health {
    hearts: u32,
    max: u32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            hearts: 20,
            max: 20,
        }
    }
}

impl Health {
    pub fn take_damage(&mut self, damage: u32) -> messages::InterfaceNodeVisibilityUpdate {
        let old_hearts = self.hearts;
        self.hearts = self.hearts.saturating_sub(damage);

        let mut image_update = messages::InterfaceNodeVisibilityUpdate::default();
        for i in self.hearts..old_hearts {
            image_update.set_hidden(format!("hotbar/health/{}", i + 1));
        }

        image_update
    }

    pub fn heal(&mut self, healing: u32) -> messages::InterfaceNodeVisibilityUpdate {
        let old_hearts = self.hearts;
        self.hearts = self.hearts.saturating_add(healing).min(self.max);

        let mut image_update = messages::InterfaceNodeVisibilityUpdate::default();
        for i in old_hearts..self.hearts {
            image_update.set_visible(format!("hotbar/health/{}", i + 1));
        }

        image_update
    }
}

#[derive(Component, Default)]
struct FallDamage(u32);

#[derive(Event)]
struct DamageEvent {
    player_entity: Entity,
    damage: u32,
}

#[derive(Event)]
struct HealEvent {
    player_entity: Entity,
    healing: u32,
}

fn fall_damage(
    mut fall_damage_query: Query<(Entity, &mut FallDamage), With<Player>>,
    mut position_events: EventReader<NetworkMessage<messages::PlayerPosition>>,
    mut damage_events: EventWriter<DamageEvent>,
) {
    for position_update in position_events.read() {
        let (_entity, mut fall_damage) = fall_damage_query
            .get_mut(position_update.player_entity)
            .unwrap();

        if fall_damage.0 != 0 && position_update.velocity.y > -0.1 {
            //damage_events.send(DamageEvent {
            //    entity,
            //    damage: fall_damage.0,
            //});
            fall_damage.0 = 0;
        } else if position_update.velocity.y < 0.0 {
            fall_damage.0 = (position_update.velocity.y.abs() as u32).saturating_sub(15);
        }
    }
}

fn change_health(
    net: Res<Server>,
    mut health_query: Query<&mut Health>,
    mut damage_events: EventReader<DamageEvent>,
    mut heal_events: EventReader<HealEvent>,
) {
    for damage_event in damage_events.read() {
        let mut health = health_query.get_mut(damage_event.player_entity).unwrap();
        let mut interface_update = health.take_damage(damage_event.damage);

        if health.hearts == 0 {
            interface_update.set_visible("death_screen".to_owned());
        }

        net.send_one(damage_event.player_entity, interface_update);
    }

    for heal_event in heal_events.read() {
        let mut health = health_query.get_mut(heal_event.player_entity).unwrap();
        let interface_update = health.heal(heal_event.healing);
        net.send_one(heal_event.player_entity, interface_update);
    }
}

#[derive(Component)]
struct DeathInterface;

fn register_death_interface(
    mut commands: Commands,
    new_player_query: Query<Entity, Added<Player>>,
    mut registration_events: EventWriter<RegisterInterfaceProvider>,
) {
    for player_entity in new_player_query.iter() {
        commands.entity(player_entity).with_children(|parent| {
            let death_interface_entity = parent.spawn(DeathInterface).id();

            registration_events.send(RegisterInterfaceProvider {
                player_entity,
                node_path: String::from("death_interface"),
                node_entity: death_interface_entity,
            });
        });
    }
}

// TODO: This should test that your health is zero. The parent of the DeathInterface is the player
// it belongs to, just query for parent.
fn death_interface(
    net: Res<Server>,
    mut interface_query: Query<
        &mut InterfaceInteractionEvents,
        (Changed<InterfaceInteractionEvents>, With<DeathInterface>),
    >,
    mut respawn_events: EventWriter<RespawnEvent>,
    mut heal_events: EventWriter<HealEvent>,
) {
    for mut interface_events in interface_query.iter_mut() {
        for interface_interaction in interface_events.read() {
            if !matches!(
                *interface_interaction,
                messages::InterfaceInteraction::Button { .. }
            ) {
                continue;
            }

            respawn_events.send(RespawnEvent {
                player_entity: interface_interaction.player_entity,
            });
            heal_events.send(HealEvent {
                player_entity: interface_interaction.player_entity,
                healing: u32::MAX,
            });

            net.send_one(
                interface_interaction.player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "death_screen".to_owned(),
                    visible: true,
                },
            );
        }
    }
}
