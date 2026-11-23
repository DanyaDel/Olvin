//! Line icons, 1.5 px stroke, drawn in a 20×20 grid.

use eframe::egui::{Color32, Painter, Rect, Shape, Stroke, pos2, vec2};

/// Line icons, 1.5 px stroke, drawn in a 20×20 grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Folder,
    FolderPlus,
    Pencil,
    Gear,
    Plus,
    Play,
    More,
    Search,
    Grid,
    List,
    Close,
    Check,
    Refresh,
    Trash,
    Package,
    Stop,
}

pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 20.0;
    let o = rect.center() - vec2(10.0, 10.0) * s;
    let p = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let stroke = Stroke::new(1.5 * s.max(0.8), color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::closed_line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Folder => closed(&[
            (2.5, 5.0),
            (7.5, 5.0),
            (9.5, 7.0),
            (17.5, 7.0),
            (17.5, 16.0),
            (2.5, 16.0),
        ]),
        Icon::FolderPlus => {
            closed(&[
                (2.5, 5.0),
                (7.5, 5.0),
                (9.5, 7.0),
                (17.5, 7.0),
                (17.5, 16.0),
                (2.5, 16.0),
            ]);
            line(&[(10.0, 9.0), (10.0, 14.0)]);
            line(&[(7.5, 11.5), (12.5, 11.5)]);
        }
        Icon::Pencil => {
            closed(&[
                (13.0, 3.5),
                (16.5, 7.0),
                (7.0, 16.5),
                (3.5, 16.5),
                (3.5, 13.0),
            ]);
            line(&[(11.0, 5.5), (14.5, 9.0)]);
        }
        Icon::Gear => {
            painter.circle_stroke(p(10.0, 10.0), 2.6 * s, stroke);
            painter.circle_stroke(p(10.0, 10.0), 5.6 * s, stroke);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = a.sin_cos();
                line(&[
                    (10.0 + 5.6 * cos, 10.0 + 5.6 * sin),
                    (10.0 + 8.0 * cos, 10.0 + 8.0 * sin),
                ]);
            }
        }
        Icon::Plus => {
            line(&[(10.0, 4.0), (10.0, 16.0)]);
            line(&[(4.0, 10.0), (16.0, 10.0)]);
        }
        Icon::Play => closed(&[(6.0, 4.0), (16.0, 10.0), (6.0, 16.0)]),
        Icon::Stop => closed(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)]),
        Icon::More => {
            for x in [4.5, 10.0, 15.5] {
                painter.circle_filled(p(x, 10.0), 1.4 * s, color);
            }
        }
        Icon::Search => {
            painter.circle_stroke(p(8.5, 8.5), 5.0 * s, stroke);
            line(&[(12.2, 12.2), (16.5, 16.5)]);
        }
        Icon::Grid => {
            for (x, y) in [(3.5, 3.5), (11.0, 3.5), (3.5, 11.0), (11.0, 11.0)] {
                closed(&[(x, y), (x + 5.5, y), (x + 5.5, y + 5.5), (x, y + 5.5)]);
            }
        }
        Icon::List => {
            for y in [5.0, 10.0, 15.0] {
                painter.circle_filled(p(3.5, y), 1.1 * s, color);
                line(&[(7.0, y), (17.0, y)]);
            }
        }
        Icon::Close => {
            line(&[(5.0, 5.0), (15.0, 15.0)]);
            line(&[(15.0, 5.0), (5.0, 15.0)]);
        }
        Icon::Check => line(&[(4.0, 10.5), (8.0, 14.5), (16.0, 5.5)]),
        Icon::Refresh => {
            let arc: Vec<(f32, f32)> = (0..=18)
                .map(|i| {
                    let a = -0.6 + i as f32 / 18.0 * 4.9;
                    (10.0 + 6.0 * a.cos(), 10.0 + 6.0 * a.sin())
                })
                .collect();
            line(&arc);
            line(&[(16.8, 3.0), (15.0, 5.2), (12.4, 4.4)]);
        }
        Icon::Trash => {
            line(&[(3.5, 5.5), (16.5, 5.5)]);
            line(&[(8.0, 5.5), (8.0, 3.5), (12.0, 3.5), (12.0, 5.5)]);
            closed(&[(5.0, 5.5), (15.0, 5.5), (14.0, 17.0), (6.0, 17.0)]);
        }
        Icon::Package => {
            closed(&[
                (10.0, 2.5),
                (17.0, 6.0),
                (17.0, 14.0),
                (10.0, 17.5),
                (3.0, 14.0),
                (3.0, 6.0),
            ]);
            line(&[(3.0, 6.0), (10.0, 9.5), (17.0, 6.0)]);
            line(&[(10.0, 9.5), (10.0, 17.5)]);
        }
    }
}
