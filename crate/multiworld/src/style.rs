use {
    dark_light::Mode::*,
    iced::{
        Background,
        Color,
        Vector,
        pure::widget::*,
    },
};

pub struct Style(pub dark_light::Mode);

impl button::StyleSheet for Style {
    fn active(&self) -> button::Style {
        button::Style {
            shadow_offset: Vector::default(),
            background: Some(Background::Color(match self.0 {
                Dark => Color::from_rgb(0.13, 0.13, 0.13),
                Light => Color::from_rgb(0.87, 0.87, 0.87),
            })),
            border_radius: 2.0,
            border_width: 1.0,
            border_color: match self.0 {
                Dark => Color::from_rgb(0.3, 0.3, 0.3),
                Light => Color::from_rgb(0.7, 0.7, 0.7),
            },
            text_color: match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            },
        }
    }
}

impl checkbox::StyleSheet for Style {
    fn active(&self, _: bool) -> checkbox::Style {
        checkbox::Style {
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.05, 0.05, 0.05),
                Light => Color::from_rgb(0.95, 0.95, 0.95),
            }),
            checkmark_color: match self.0 {
                Dark => Color::from_rgb(0.7, 0.7, 0.7),
                Light => Color::from_rgb(0.3, 0.3, 0.3),
            },
            border_radius: 5.0,
            border_width: 1.0,
            border_color: match self.0 {
                Dark => Color::from_rgb(0.4, 0.4, 0.4),
                Light => Color::from_rgb(0.6, 0.6, 0.6),
            },
            text_color: Some(match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            }),
        }
    }

    fn hovered(&self, is_checked: bool) -> checkbox::Style {
        checkbox::Style {
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.1, 0.1, 0.1),
                Light => Color::from_rgb(0.9, 0.9, 0.9),
            }),
            ..self.active(is_checked)
        }
    }
}

impl pick_list::StyleSheet for Style {
    fn menu(&self) -> pick_list::Menu {
        pick_list::Menu {
            text_color: match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            },
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.13, 0.13, 0.13),
                Light => Color::from_rgb(0.87, 0.87, 0.87),
            }),
            border_width: 1.0,
            border_color: match self.0 {
                Dark => Color::from_rgb(0.3, 0.3, 0.3),
                Light => Color::from_rgb(0.7, 0.7, 0.7),
            },
            selected_text_color: match self.0 {
                Dark => Color::BLACK,
                Light => Color::WHITE,
            },
            selected_background: Background::Color(Color::from_rgb(0.4, 0.4, 1.0)),
        }
    }

    fn active(&self) -> pick_list::Style {
        pick_list::Style {
            text_color: match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            },
            placeholder_color: match self.0 {
                Dark => Color::from_rgb(0.6, 0.6, 0.6),
                Light => Color::from_rgb(0.4, 0.4, 0.4),
            },
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.13, 0.13, 0.13),
                Light => Color::from_rgb(0.87, 0.87, 0.87),
            }),
            border_color: match self.0 {
                Dark => Color::from_rgb(0.3, 0.3, 0.3),
                Light => Color::from_rgb(0.7, 0.7, 0.7),
            },
            ..pick_list::Style::default()
        }
    }

    fn hovered(&self) -> pick_list::Style {
        pick_list::Style {
            border_color: match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            },
            ..self.active()
        }
    }
}

impl radio::StyleSheet for Style {
    fn active(&self) -> radio::Style {
        radio::Style {
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.05, 0.05, 0.05),
                Light => Color::from_rgb(0.95, 0.95, 0.95),
            }),
            dot_color: match self.0 {
                Dark => Color::from_rgb(0.7, 0.7, 0.7),
                Light => Color::from_rgb(0.3, 0.3, 0.3),
            },
            border_width: 1.0,
            border_color: match self.0 {
                Dark => Color::from_rgb(0.4, 0.4, 0.4),
                Light => Color::from_rgb(0.6, 0.6, 0.6),
            },
            text_color: Some(match self.0 {
                Dark => Color::WHITE,
                Light => Color::BLACK,
            }),
        }
    }

    fn hovered(&self) -> radio::Style {
        radio::Style {
            background: Background::Color(match self.0 {
                Dark => Color::from_rgb(0.1, 0.1, 0.1),
                Light => Color::from_rgb(0.9, 0.9, 0.9),
            }),
            ..self.active()
        }
    }
}

impl text_input::StyleSheet for Style {
    fn active(&self) -> text_input::Style {
        text_input::Style {
            background: Background::Color(match self.0 {
                Dark => Color::BLACK,
                Light => Color::WHITE,
            }),
            border_radius: 0.0,
            border_width: 1.0,
            border_color: match self.0 {
                Dark => Color::from_rgb(0.3, 0.3, 0.3),
                Light => Color::from_rgb(0.7, 0.7, 0.7),
            },
        }
    }

    fn focused(&self) -> text_input::Style {
        text_input::Style {
            border_color: Color::from_rgb(0.5, 0.5, 0.5),
            ..self.active()
        }
    }

    fn placeholder_color(&self) -> Color {
        match self.0 {
            Dark => Color::from_rgb(0.3, 0.3, 0.3),
            Light => Color::from_rgb(0.7, 0.7, 0.7),
        }
    }

    fn value_color(&self) -> Color {
        match self.0 {
            Dark => Color::WHITE,
            Light => Color::BLACK,
        }
    }

    fn selection_color(&self) -> Color {
        match self.0 {
            Dark => Color::from_rgb(0.0, 0.0, 0.4),
            Light => Color::from_rgb(0.8, 0.8, 1.0),
        }
    }
}
