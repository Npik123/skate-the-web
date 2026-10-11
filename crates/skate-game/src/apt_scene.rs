//! Retained movie traversal. All geometry and glyphs come from the owned APT.
use crate::{apt_movie::Movie, apt_vm::Vm};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Vertex {
    pub position: [f32; 2],
    /// Absent on solid fills, which sample a plain white texel.
    #[serde(default)]
    pub uv: [f32; 2],
}
#[derive(Clone, Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: String,
}
#[derive(Clone, Deserialize)]
pub struct Shape {
    /// `None` for solid fills.
    pub texture: Option<Texture>,
    pub triangles: Vec<[Vertex; 3]>,
    pub color: [f32; 4],
}
pub type Shapes = BTreeMap<i32, Vec<Shape>>;
pub struct Draw {
    /// `None` draws the shape's colour with a plain white texel.
    pub texture: Option<String>,
    pub vertices: Vec<Vertex>,
    pub multiply: [f32; 4],
    pub add: [f32; 4],
}
pub(crate) fn compose(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn transform(m: [f32; 6], v: &Vertex) -> Vertex {
    Vertex {
        position: [
            m[0] * v.position[0] + m[2] * v.position[1] + m[4],
            m[1] * v.position[0] + m[3] * v.position[1] + m[5],
        ],
        uv: v.uv,
    }
}
pub fn draw(movie: &Movie, vm: &Vm, shapes: &Shapes) -> Result<Vec<Draw>, String> {
    let mut draws = Vec::new();
    visit(
        movie,
        vm,
        shapes,
        movie.root,
        [1., 0., 0., 1., 0., 0.],
        [1.; 4],
        [0.; 4],
        &mut draws,
    )?;
    Ok(draws)
}
fn visit(
    movie: &Movie,
    vm: &Vm,
    shapes: &Shapes,
    id: usize,
    parent: [f32; 6],
    pm: [f32; 4],
    pa: [f32; 4],
    out: &mut Vec<Draw>,
) -> Result<(), String> {
    if !vm.get(id, "_visible").truth() {
        return Ok(());
    }
    let instance = &movie.instances[&id];
    let matrix = movie.local_matrix(vm, id);
    let matrix = compose(parent, matrix);
    if matrix.iter().any(|x| !x.is_finite()) {
        // Name the clip path and the script-set fields that went bad.
        let mut path = Vec::new();
        let mut at = Some(id);
        while let Some(clip) = at {
            path.push(vm.get(clip, "_name").text());
            at = movie.instances.get(&clip).and_then(|i| i.parent);
        }
        path.reverse();
        let fields: Vec<String> = ["_x", "_y", "_xscale", "_yscale", "_rotation", "_width", "_height"]
            .iter()
            .map(|k| format!("{k}={}", vm.get(id, k).text()))
            .collect();
        return Err(format!(
            "Nonfinite HUD transform on character {} ({}: {})",
            instance.character,
            path.join("."),
            fields.join(" ")
        ));
    }
    let color = instance
        .placement
        .as_ref()
        .map_or([255, 255, 255, 255, 0, 0, 0, 0], |p| p.color);
    let rgba = [1, 2, 3, 0];
    let mut multiply = std::array::from_fn(|i| pm[i] * color[rgba[i]] as f32 / 255.);
    let mut add = std::array::from_fn(|i| pa[i] + pm[i] * color[4 + rgba[i]] as f32 / 255.);
    let alpha = (vm.get(id, "_alpha").number() as f32 / 100.).clamp(0., 1.);
    multiply[3] *= alpha;
    add[3] *= alpha;
    if multiply[3] <= 0. && add[3] <= 0. {
        return Ok(());
    }
    let character = &movie.characters[&instance.character];
    if character.type_name == "shape" {
        for shape in shapes
            .get(&character.id)
            .ok_or("Missing original shape geometry")?
        {
            out.push(Draw {
                texture: shape.texture.as_ref().map(|t| t.rgba.clone()),
                vertices: shape
                    .triangles
                    .iter()
                    .flatten()
                    .map(|v| transform(matrix, v))
                    .collect(),
                multiply: std::array::from_fn(|i| multiply[i] * shape.color[i]),
                add,
            });
        }
    } else if let Some(text) = &character.text {
        let font = &movie.text_assets.fonts
            [&(text["font_id"].as_i64().ok_or("Invalid text font")? as i32)];
        // Native shadow text uses a black atlas pass followed by the sharp
        // futuraheavy glyphs translated +1 in text X (825D6B68/82CA1FD8).
        let passes = if let Some(foreground) = font.foreground {
            vec![
                (font, true, 0.),
                (&movie.text_assets.fonts[&foreground], false, 1.),
            ]
        } else {
            vec![(font, false, 0.)]
        };
        for (font, shadow, advance_x) in passes {
            let height = text["font_height"].as_f64().ok_or("Invalid text height")? as f32;
            let sx = font.scale[0] * height;
            let sy = font.scale[1] * height;
            let value = vm.get(id, "_displayText").text();
            let bounds = character.bounds.ok_or("Missing text bounds")?;
            let alignment = text["alignment"].as_u64().unwrap_or(0);
            let autosize = vm.get(id, "autoSize").text();
            let alignment = if autosize == "left" { 0 } else { alignment };
            let mut vertices = Vec::new();
            let mut buttons = Vec::new();
            // Inline controller buttons (<B:A_hud>) are as tall as a line of text.
            let button = |name: &str| {
                movie.glyphs.get(&glyph_key(name)).map(|(texture, [w, h])| {
                    let gh = height * 1.3;
                    (texture, gh * *w as f32 / (*h).max(1) as f32, gh)
                })
            };
            // Multiline fields break into lines, laid out a line height apart.
            for (row, line) in wrapped(text, &movie.text_assets.fonts, &value, bounds).iter().enumerate() {
                let parts = pieces(line);
                let width: f32 = parts
                    .iter()
                    .map(|part| match part {
                        Piece::Text(t) => font.width(t, height),
                        Piece::Button(name, raw) => button(name).map_or_else(|| font.width(raw, height), |(_, w, _)| w + 4.),
                    })
                    .sum();
                let mut x = bounds[0]
                    + advance_x
                    + font.offset[0] * sx
                    + match alignment {
                        1 => bounds[2] - bounds[0] - width,
                        2 => (bounds[2] - bounds[0] - width) * 0.5,
                        _ => 0.,
                    };
                let y = bounds[1] + font.offset[1] * sy + row as f32 * line_height(height);
                let top = bounds[1] + row as f32 * line_height(height);
                let mut chars: Vec<Option<char>> = Vec::new();
                for part in &parts {
                    match part {
                        Piece::Button(name, raw) => match button(name) {
                            Some(_) => chars.push(None),
                            None => chars.extend(raw.chars().map(Some)),
                        },
                        Piece::Text(t) => chars.extend(t.chars().map(Some)),
                    }
                }
                let mut found = parts.iter().filter_map(|part| match part {
                    Piece::Button(name, _) => button(name),
                    Piece::Text(_) => None,
                });
                for c in chars {
                    let Some(c) = c else {
                        let Some((texture, gw, gh)) = found.next() else { continue };
                        if !shadow {
                            let (x0, y0) = (x + 2., top + (height - gh) * 0.5 + 2.);
                            let corners = [[x0, y0, 0., 0.], [x0 + gw, y0, 1., 0.], [x0 + gw, y0 + gh, 1., 1.], [x0, y0 + gh, 0., 1.]];
                            buttons.push(Draw {
                                texture: Some(texture.clone()),
                                vertices: [0, 1, 2, 0, 2, 3]
                                    .iter()
                                    .map(|&i| {
                                        let [x, y, u, v] = corners[i];
                                        transform(matrix, &Vertex { position: [x, y], uv: [u, v] })
                                    })
                                    .collect(),
                                multiply,
                                add,
                            });
                        }
                        x += gw + 4.;
                        continue;
                    };
                    if let Some(g) = font.glyph(c) {
                        let x0 = x + g.x_offset * sx;
                        let y0 = y + (font.ascent - g.y_offset) * sy;
                        let x1 = x0 + g.width * sx;
                        let y1 = y0 + g.height * sy;
                        let [u0, v0, u1, v1] = g.atlas_bounds;
                        let points = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                        let uvs = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                        for i in [0, 1, 2, 0, 2, 3] {
                            vertices.push(transform(
                                matrix,
                                &Vertex {
                                    position: points[i],
                                    uv: [uvs[i][0] / font.size[0] as f32, uvs[i][1] / font.size[1] as f32],
                                },
                            ));
                        }
                        x = g.x_advance.mul_add(sx, x);
                    }
                }
            }
            let argb = u32::from_str_radix(
                text["color_argb"]
                    .as_str()
                    .ok_or("Missing text color")?
                    .trim_start_matches('#'),
                16,
            )
            .map_err(|e| e.to_string())?;
            // A script's textColor (0xRRGGBB) replaces the authored colour.
            let argb = match vm.get(id, "textColor") {
                crate::apt_vm::Value::Number(rgb) if rgb.is_finite() => (argb & 0xff00_0000) | (rgb as u32 & 0x00ff_ffff),
                _ => argb,
            };
            let color = [
                ((argb >> 16) & 255) as f32 / 255.,
                ((argb >> 8) & 255) as f32 / 255.,
                (argb & 255) as f32 / 255.,
                (argb >> 24) as f32 / 255.,
            ];
            if !vertices.is_empty() {
                out.push(Draw {
                    texture: Some(font.texture.clone()),
                    vertices,
                    multiply: std::array::from_fn(|i| {
                        if shadow && i < 3 {
                            0.
                        } else {
                            multiply[i] * color[i]
                        }
                    }),
                    add,
                });
            }
            out.append(&mut buttons);
        }
    }
    // A button the native image renderer drew: the glyph, fitted to the
    // placeholder's bounds, in place of the placeholder.
    if let Some((texture, [w, h])) = glyph(movie, vm, id) {
        if let Some(b) = movie.local_bounds(vm, id) {
            let scale = ((b[2] - b[0]) / *w as f32).min((b[3] - b[1]) / *h as f32);
            let (gw, gh) = (*w as f32 * scale, *h as f32 * scale);
            let x0 = (b[0] + b[2] - gw) * 0.5;
            let y0 = (b[1] + b[3] - gh) * 0.5;
            let corners = [[x0, y0, 0., 0.], [x0 + gw, y0, 1., 0.], [x0 + gw, y0 + gh, 1., 1.], [x0, y0 + gh, 0., 1.]];
            out.push(Draw {
                texture: Some(texture.clone()),
                vertices: [0, 1, 2, 0, 2, 3]
                    .iter()
                    .map(|&i| {
                        let [x, y, u, v] = corners[i];
                        transform(matrix, &Vertex { position: [x, y], uv: [u, v] })
                    })
                    .collect(),
                multiply,
                add,
            });
        }
        return Ok(());
    }
    for child in instance.ordered_children() {
        visit(movie, vm, shapes, child, matrix, multiply, add, out)?;
    }
    Ok(())
}

/// Distance between the lines of a multiline field.
pub(crate) fn line_height(font_height: f32) -> f32 {
    font_height + 2.0
}
/// A text field's lines, wrapped as the field is set up to (always one line
/// for single-line fields).
pub(crate) fn wrapped(
    text: &serde_json::Value,
    fonts: &BTreeMap<i32, crate::apt_text::Font>,
    value: &str,
    bounds: [f32; 4],
) -> Vec<String> {
    let multiline = text["multiline"].as_bool().unwrap_or(false);
    let wrap = text["word_wrap"].as_bool().unwrap_or(false);
    let font = text["font_id"].as_i64().and_then(|id| fonts.get(&(id as i32)));
    match font {
        Some(font) if multiline || wrap => {
            let height = text["font_height"].as_f64().unwrap_or(12.0) as f32;
            font.lines(value, height, wrap.then_some(bounds[2] - bounds[0] - 4.0))
        }
        _ => vec![value.to_string()],
    }
}

/// The glyph a clip names, if any ("B", "A_hud"...).
fn glyph<'a>(movie: &'a Movie, vm: &Vm, id: usize) -> Option<&'a (String, [u32; 2])> {
    if movie.glyphs.is_empty() {
        return None;
    }
    let name = match vm.get(id, "_test") {
        crate::apt_vm::Value::Text(name) => name,
        _ => return None,
    };
    movie.glyphs.get(&glyph_key(&name))
}

/// A line of text and the controller buttons named in it (<B:A_hud>), as
/// the game's strings write them.
enum Piece<'a> {
    Text(&'a str),
    /// The button's name, and the tag as written (drawn if no glyph).
    Button(&'a str, &'a str),
}
fn pieces(line: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("<B:") {
        let Some(end) = rest[start..].find('>').map(|e| start + e) else { break };
        if start > 0 {
            out.push(Piece::Text(&rest[..start]));
        }
        out.push(Piece::Button(&rest[start + 3..end], &rest[start..=end]));
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    out
}

/// The exported glyph for a button name ("A_hud", "L_Trigger_hud"...).
fn glyph_key(name: &str) -> String {
    let key = name.trim_end_matches("_hud").trim_end_matches("_HUD").to_ascii_uppercase();
    // Screens name the shoulder buttons in full (createskater's "Rotate
    // Skater" uses L_Trigger_hud / R_Trigger_hud).
    match key.as_str() {
        "L_TRIGGER" | "LEFT_TRIGGER" => "LT",
        "R_TRIGGER" | "RIGHT_TRIGGER" => "RT",
        "L_BUMPER" | "LEFT_BUMPER" | "L_SHOULDER" => "LB",
        "R_BUMPER" | "RIGHT_BUMPER" | "R_SHOULDER" => "RB",
        "L_STICK" | "LEFT_STICK" => "LS",
        "R_STICK" | "RIGHT_STICK" => "RS",
        // The Trick Guide's button table (82FCB5D8) names stick directions,
        // clicks and d-pad directions; the exported glyphs are per control.
        k if k.starts_with("STICK_LEFT") || k == "L3_CLICK" => "LS",
        k if k.starts_with("STICK_RIGHT") || k == "R3_CLICK" => "RS",
        k if k.starts_with("DPAD") => "DPAD",
        other => other,
    }
    .to_owned()
}
