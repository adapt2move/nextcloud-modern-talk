//! Retained, bounded texture layers for the native material surfaces.
use crate::presentation::{Desktop, MaterialImages};
use gpui::{
    AnyView, Bounds, Context, Corners, Entity, Hsla, ImageSource, Pixels, Render, Styled, Window,
    canvas, div, point, prelude::*, px, size,
};

const TILE: f32 = 128.;
const LIGHT_GLASS_ALPHA: f32 = 0.98;
const DARK_GLASS_ALPHA: f32 = 0.92;
// A window cannot create an unbounded number of retained decorative views.
const MAX_TILES_PER_AXIS: usize = 64;

/// Paint the filtered backdrop before its palette tint. A renderer without
/// backdrop support paints the original opaque palette color instead.
pub(in crate::presentation) fn glass(tint: Hsla, dark: bool, radius: f32) -> impl IntoElement {
    glass_corners(tint, dark, Corners::from(px(radius)))
}

pub(in crate::presentation) fn glass_corners(
    tint: Hsla,
    dark: bool,
    corners: Corners<Pixels>,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let supported = window.paint_backdrop(bounds, corners, px(18.));
            let fill = if supported {
                tint.opacity(if dark {
                    DARK_GLASS_ALPHA
                } else {
                    LIGHT_GLASS_ALPHA
                })
            } else {
                tint
            };
            window.paint_quad(gpui::fill(bounds, fill).corner_radii(corners));
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{Appearance, Palette, Theme};

    fn blend(front: gpui::Rgba, back: gpui::Rgba) -> gpui::Rgba {
        gpui::Rgba {
            r: front.r * front.a + back.r * (1. - front.a),
            g: front.g * front.a + back.g * (1. - front.a),
            b: front.b * front.a + back.b * (1. - front.a),
            a: 1.,
        }
    }

    fn contrast(a: gpui::Rgba, b: gpui::Rgba) -> f32 {
        let luminance = |c: gpui::Rgba| {
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn glass_and_grain_keep_palette_labels_readable_on_extreme_backdrops() {
        for palette in [Palette::Blue, Palette::Sage] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let theme = Theme::resolve(palette, appearance, gpui::WindowAppearance::Light);
                let alpha = if theme.dark {
                    DARK_GLASS_ALPHA
                } else {
                    LIGHT_GLASS_ALPHA
                };
                for backdrop in [gpui::black(), gpui::white()] {
                    let glass = blend(theme.surface.opacity(alpha).into(), backdrop.into());
                    for grain in [gpui::black(), gpui::white()] {
                        let background = blend(grain.opacity(6. / 255.).into(), glass);
                        for color in [theme.text, theme.muted, theme.selected_text] {
                            assert!(
                                contrast(color.into(), background) >= 4.5,
                                "{palette:?} {appearance:?}"
                            );
                        }
                        assert!(contrast(theme.focus.into(), background) >= 3.);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::presentation) enum Texture {
    Grain,
    Card,
    Tile,
    PatternLight,
    PatternDark,
}

struct Layer {
    image: ImageSource,
    radius: f32,
}

impl Render for Layer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let image = match &self.image {
            ImageSource::Render(image) => Some(image.clone()),
            _ => None,
        };
        let radius = self.radius;
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, (), window, _| {
                let Some(image) = image else { return };
                let width = f32::from(bounds.size.width).max(0.);
                let height = f32::from(bounds.size.height).max(0.);
                let columns = ((width / TILE).ceil() as usize).min(MAX_TILES_PER_AXIS);
                let rows = ((height / TILE).ceil() as usize).min(MAX_TILES_PER_AXIS);
                if columns == 0 || rows == 0 {
                    return;
                }
                // Enormous windows remain bounded; ordinary windows use the
                // native 128-point tile without stretching.
                let tile_width = TILE.max(width / columns as f32);
                let tile_height = TILE.max(height / rows as f32);
                for row in 0..rows {
                    for column in 0..columns {
                        let x = column as f32 * tile_width;
                        let y = row as f32 * tile_height;
                        let tile = Bounds::new(
                            point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                            size(
                                px(tile_width.min(width - x)),
                                px(tile_height.min(height - y)),
                            ),
                        );
                        let corner = |at_corner| px(if at_corner { radius } else { 0. });
                        let radii = Corners {
                            top_left: corner(row == 0 && column == 0),
                            top_right: corner(row == 0 && column + 1 == columns),
                            bottom_left: corner(row + 1 == rows && column == 0),
                            bottom_right: corner(row + 1 == rows && column + 1 == columns),
                        };
                        // Missing GPU capacity must not compromise input or
                        // content; the opaque surface remains the fallback.
                        let _ = window.paint_image(tile, radii, image.clone(), 0, false);
                    }
                }
            },
        )
        .size_full()
    }
}

struct Pool {
    views: Vec<Entity<Layer>>,
    used: std::cell::Cell<usize>,
}

impl Pool {
    fn new(
        image: Option<ImageSource>,
        radius: f32,
        count: usize,
        cx: &mut Context<Desktop>,
    ) -> Self {
        let views = image.map_or_else(Vec::new, |image| {
            (0..count)
                .map(|_| {
                    cx.new(|_| Layer {
                        image: image.clone(),
                        radius,
                    })
                })
                .collect()
        });
        Self {
            views,
            used: std::cell::Cell::new(0),
        }
    }

    fn next(&self) -> Option<AnyView> {
        let index = self.used.get();
        let view = self.views.get(index)?.clone();
        self.used.set(index + 1);
        let mut placement = div().absolute().inset_0();
        Some(AnyView::from(view).cached(placement.style().clone()))
    }

    fn reset(&self) {
        self.used.set(0);
    }
}

pub(in crate::presentation) struct Materials {
    grain: Pool,
    card: Pool,
    tile: Pool,
    pattern_light: Pool,
    pattern_dark: Pool,
}

impl Materials {
    pub fn new(images: MaterialImages, cx: &mut Context<Desktop>) -> Self {
        let MaterialImages { grain, light, dark } = images;
        Self {
            grain: Pool::new(grain.clone(), 0., 8, cx),
            card: Pool::new(grain.clone(), 16., 8, cx),
            tile: Pool::new(grain, 12., 24, cx),
            pattern_light: Pool::new(light, 0., 4, cx),
            pattern_dark: Pool::new(dark, 0., 4, cx),
        }
    }

    pub fn begin_frame(&self) {
        for pool in [
            &self.grain,
            &self.card,
            &self.tile,
            &self.pattern_light,
            &self.pattern_dark,
        ] {
            pool.reset();
        }
    }

    pub fn layer(&self, texture: Texture, disabled: bool) -> Option<AnyView> {
        if disabled {
            return None;
        }
        match texture {
            Texture::Grain => &self.grain,
            Texture::Card => &self.card,
            Texture::Tile => &self.tile,
            Texture::PatternLight => &self.pattern_light,
            Texture::PatternDark => &self.pattern_dark,
        }
        .next()
    }
}
