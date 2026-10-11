//! The controls card (pause menu > PLAYER > Controls): the keyboard layout the
//! browser page maps onto a controller, Trackpad Mode, and the controller.
use bevy::prelude::*;

#[derive(Component)]
struct ControlsRoot;
#[derive(Component)]
struct CloseButton;

const KEY: Color = Color::srgb(0.55, 0.85, 1.0);
const TEXT: Color = Color::srgb(0.86, 0.92, 0.96);
const DIM: Color = Color::srgb(0.58, 0.68, 0.74);

pub(crate) fn install(app: &mut App) {
    app.add_systems(Update, show);
}

/// (button, keyboard key, what it does in game; empty when it has no fixed job)
const KEYBOARD: &[(&str, &str, &str)] = &[
    ("Left stick", "W A S D", "Steer, push off, crouch"),
    ("Right stick", "Arrow keys", "Flick It tricks (or Trackpad Mode)"),
    ("A", "Space", "Push"),
    ("B", "E", ""),
    ("X", "Q", ""),
    ("Y", "R", ""),
    ("LB / RB", "Z / X", ""),
    ("LT / RT", "Shift / C", "Grabs"),
    ("L3 / R3", "V / B", ""),
    ("D-pad", "I J K L", ""),
    ("Start", "Esc", "Pause menu"),
    ("Back", "Tab", ""),
];

fn show(
    mut commands: Commands,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Res<crate::customiser::Navigation>,
    roots: Query<Entity, With<ControlsRoot>>,
    close: Query<&Interaction, (Changed<Interaction>, With<CloseButton>)>,
    mut shown: Local<bool>,
) {
    if !menu.open {
        menu.controls_help = false;
    }
    if menu.controls_help && !*shown {
        *shown = true;
        spawn(&mut commands);
        // The press that opened the card must not also close it.
        return;
    }
    if menu.controls_help {
        let pressed = keys.any_just_pressed([KeyCode::Escape, KeyCode::Backspace, KeyCode::Enter, KeyCode::Space])
            || nav.pressed & (0x1000 | 0x2000 | 0x10) != 0
            || close.iter().any(|i| *i == Interaction::Pressed);
        if pressed {
            menu.controls_help = false;
        }
    }
    if !menu.controls_help && *shown {
        *shown = false;
        for root in &roots {
            commands.entity(root).despawn();
        }
    }
}

fn spawn(commands: &mut Commands) {
    let web = cfg!(target_arch = "wasm32");
    commands
        .spawn((
            ControlsRoot,
            GlobalZIndex(12),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.025, 0.04, 0.92)),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: px(820),
                    max_width: percent(94),
                    max_height: percent(94),
                    padding: UiRect::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(1),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.04, 0.07, 0.1, 0.96)),
            ))
            .with_children(|panel| {
                let text = |panel: &mut ChildSpawnerCommands, words: &str, size: f32, color: Color| {
                    panel.spawn((Text::new(words), TextFont { font_size: size, ..default() }, TextColor(color)));
                };
                text(panel, "CONTROLS", 24., Color::WHITE);
                if web {
                    text(panel, "Keyboard (used when no controller is connected)", 15., KEY);
                    for (button, key, job) in KEYBOARD {
                        panel
                            .spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(12), ..default() })
                            .with_children(|line| {
                                line.spawn((Node { width: px(150), ..default() }, Text::new(*key), TextFont { font_size: 14., ..default() }, TextColor(KEY)));
                                line.spawn((Node { width: px(130), ..default() }, Text::new(*button), TextFont { font_size: 14., ..default() }, TextColor(TEXT)));
                                line.spawn((Text::new(*job), TextFont { font_size: 14., ..default() }, TextColor(DIM)));
                            });
                    }
                    text(panel, "F8 = debug camera, T = drop your skater at the camera", 13., DIM);
                    panel.spawn(Node { height: px(5), ..default() });
                    text(panel, "Trackpad Mode (PLAYER > Trackpad Mode)", 15., KEY);
                    text(panel, "The mouse or trackpad becomes the right stick. Click the game to lock the pointer; Esc frees it. \
Ollie: move down, then quickly up. Flip tricks: flick sideways or diagonally. Pick a higher level if flicks feel too short.", 14., TEXT);
                    panel.spawn(Node { height: px(5), ..default() });
                }
                text(panel, "Controller", 15., KEY);
                text(panel, "Any standard (Xbox layout) gamepad. Left stick steers and crouches, the right stick flicks tricks \
(pull back, then flick), A pushes, the triggers grab, Start pauses.", 14., TEXT);
                panel.spawn(Node { height: px(6), ..default() });
                panel
                    .spawn((
                        Button,
                        CloseButton,
                        Node { padding: UiRect::axes(px(16), px(8)), align_self: AlignSelf::FlexStart, ..default() },
                        BackgroundColor(Color::srgb(0.08, 0.2, 0.28)),
                    ))
                    .with_child((Text::new("Back  (Esc / B)"), TextFont { font_size: 16., ..default() }, TextColor(Color::WHITE)));
            });
        });
}
