//! drawio (mxGraph) -> excalidraw element conversion.
//!
//! Handles uncompressed mxGraph XML: `<mxfile><diagram><mxGraphModel><root>
//! <mxCell .../></root></mxGraphModel></diagram></mxfile>` (or a bare
//! `<mxGraphModel>`). Vertices become rectangle/ellipse/diamond/text;
//! edges become arrows wired between vertex centers (with waypoints).
//!
//! Compressed `<diagram>` payloads (base64 + raw-deflate) are detected and
//! rejected with a clear error -- inflation support is a later task.

use std::collections::HashMap;

use serde_json::Value;

use crate::diagram::model::Element;

/// Parse mxGraph XML into excalidraw elements.
pub fn parse(xml: &str) -> Result<Vec<Element>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("invalid drawio XML: {}", e))?;

    let cells: Vec<roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("mxCell"))
        .collect();

    if cells.is_empty() {
        if doc.descendants().any(|n| n.has_tag_name("diagram")) {
            return Err(
                "this drawio file looks compressed; export as uncompressed XML and retry".into(),
            );
        }
        return Err("no mxCell elements found in drawio file".into());
    }

    // Geometry of every vertex, for resolving edge endpoints.
    let mut geom: HashMap<String, (f64, f64, f64, f64)> = HashMap::new();
    for cell in &cells {
        if cell.attribute("vertex") == Some("1")
            && let Some(id) = cell.attribute("id")
            && let Some(g) = geometry(cell)
        {
            geom.insert(id.to_string(), g);
        }
    }

    let mut elements = Vec::new();
    let mut counter = 0usize;

    for cell in &cells {
        if cell.attribute("vertex") == Some("1") {
            let Some((x, y, w, h)) = geometry(cell) else {
                continue;
            };
            let style = parse_style(cell.attribute("style").unwrap_or(""));
            let kind = shape_type(&style);
            let value = clean_value(cell.attribute("value").unwrap_or(""));

            if kind == "text" {
                if value.is_empty() {
                    continue;
                }
                let mut text = Element::base(eid(&mut counter), "text", x, y, w, h);
                text.text = Some(value);
                text.font_size = Some(16.0);
                if let Some(sc) = style.get("strokeColor") {
                    text.stroke_color = map_color(sc);
                }
                elements.push(text);
                continue;
            }

            let mut shape = Element::base(eid(&mut counter), kind, x, y, w, h);
            if let Some(fc) = style.get("fillColor") {
                shape.background_color = map_color(fc);
            }
            if let Some(sc) = style.get("strokeColor") {
                shape.stroke_color = map_color(sc);
            }
            elements.push(shape);

            // Label as a separate text element near the shape's vertical center.
            if !value.is_empty() {
                let font = 16.0;
                let mut label = Element::base(
                    eid(&mut counter),
                    "text",
                    x + 6.0,
                    y + h / 2.0 - font * 0.6,
                    (w - 12.0).max(1.0),
                    font * 1.25,
                );
                label.text = Some(value);
                label.font_size = Some(font);
                elements.push(label);
            }
        } else if cell.attribute("edge") == Some("1") {
            let points = edge_points(cell, &geom);
            if points.len() < 2 {
                continue;
            }
            let (ox, oy) = points[0];
            let rel: Vec<[f64; 2]> = points.iter().map(|(x, y)| [x - ox, y - oy]).collect();
            let min_x = points.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let max_x = points.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let min_y = points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let max_y = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);

            let mut arrow =
                Element::base(eid(&mut counter), "arrow", ox, oy, max_x - min_x, max_y - min_y);
            arrow.points = Some(rel);
            arrow.end_arrowhead = Some(Value::String("arrow".to_string()));
            let style = parse_style(cell.attribute("style").unwrap_or(""));
            if let Some(sc) = style.get("strokeColor") {
                arrow.stroke_color = map_color(sc);
            }
            elements.push(arrow);
        }
    }

    if elements.is_empty() {
        return Err("drawio file contained no convertible shapes".into());
    }
    Ok(elements)
}

fn eid(counter: &mut usize) -> String {
    *counter += 1;
    format!("dio-{}", counter)
}

/// Read the `<mxGeometry>` child as `(x, y, width, height)`.
fn geometry(cell: &roxmltree::Node) -> Option<(f64, f64, f64, f64)> {
    let g = cell.children().find(|n| n.has_tag_name("mxGeometry"))?;
    let num = |name: &str| g.attribute(name).and_then(|v| v.parse::<f64>().ok());
    Some((
        num("x").unwrap_or(0.0),
        num("y").unwrap_or(0.0),
        num("width").unwrap_or(0.0),
        num("height").unwrap_or(0.0),
    ))
}

/// Split a drawio style string `"k=v;flag;k2=v2"` into a map. Flags map to
/// `"true"`.
fn parse_style(style: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in style.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((k, v)) => {
                map.insert(k.to_string(), v.to_string());
            }
            None => {
                map.insert(part.to_string(), "true".to_string());
            }
        }
    }
    map
}

fn shape_type(style: &HashMap<String, String>) -> &'static str {
    if style.contains_key("ellipse") {
        "ellipse"
    } else if style.contains_key("rhombus") {
        "diamond"
    } else if style.contains_key("text") {
        "text"
    } else {
        "rectangle"
    }
}

/// drawio colors are `#rrggbb` or `none`.
fn map_color(c: &str) -> String {
    if c.eq_ignore_ascii_case("none") {
        "transparent".to_string()
    } else {
        c.to_string()
    }
}

/// Strip HTML markup and decode the common entities drawio emits in labels.
fn clean_value(value: &str) -> String {
    let with_breaks = value
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    // Drop tags.
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// Compute an edge's polyline: source center (or sourcePoint), waypoints,
/// target center (or targetPoint).
fn edge_points(
    cell: &roxmltree::Node,
    geom: &HashMap<String, (f64, f64, f64, f64)>,
) -> Vec<(f64, f64)> {
    let center = |id: &str| geom.get(id).map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0));

    let g = cell.children().find(|n| n.has_tag_name("mxGeometry"));

    let endpoint = |attr: &str, as_name: &str| -> Option<(f64, f64)> {
        if let Some(id) = cell.attribute(attr)
            && let Some(c) = center(id)
        {
            return Some(c);
        }
        let g = g?;
        let p = g
            .children()
            .find(|n| n.has_tag_name("mxPoint") && n.attribute("as") == Some(as_name))?;
        Some((
            p.attribute("x").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            p.attribute("y").and_then(|v| v.parse().ok()).unwrap_or(0.0),
        ))
    };

    let start = endpoint("source", "sourcePoint");
    let end = endpoint("target", "targetPoint");

    let mut points = Vec::new();
    if let Some(s) = start {
        points.push(s);
    }
    // Waypoints: <Array as="points"><mxPoint x= y=/></Array>
    if let Some(g) = g
        && let Some(arr) = g
            .children()
            .find(|n| n.has_tag_name("Array") && n.attribute("as") == Some("points"))
    {
        for p in arr.children().filter(|n| n.has_tag_name("mxPoint")) {
            points.push((
                p.attribute("x").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                p.attribute("y").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            ));
        }
    }
    if let Some(e) = end {
        points.push(e);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<mxfile>
      <diagram name="Page-1">
        <mxGraphModel>
          <root>
            <mxCell id="0"/>
            <mxCell id="1" parent="0"/>
            <mxCell id="2" value="Start" style="rounded=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
              <mxGeometry x="40" y="40" width="120" height="60" as="geometry"/>
            </mxCell>
            <mxCell id="3" value="End" style="ellipse;fillColor=#d5e8d4;" vertex="1" parent="1">
              <mxGeometry x="300" y="200" width="100" height="80" as="geometry"/>
            </mxCell>
            <mxCell id="4" style="edgeStyle=orthogonalEdgeStyle;strokeColor=#000000;" edge="1" parent="1" source="2" target="3">
              <mxGeometry relative="1" as="geometry"/>
            </mxCell>
          </root>
        </mxGraphModel>
      </diagram>
    </mxfile>"#;

    #[test]
    fn parses_vertices_edges_and_labels() {
        let els = parse(SAMPLE).unwrap();
        // rectangle + its label, ellipse + its label, one arrow = 5.
        assert_eq!(els.len(), 5);

        let rect = els.iter().find(|e| e.element_type == "rectangle").unwrap();
        assert_eq!(rect.x, 40.0);
        assert_eq!(rect.background_color, "#dae8fc");
        assert_eq!(rect.stroke_color, "#6c8ebf");

        let ellipse = els.iter().find(|e| e.element_type == "ellipse").unwrap();
        assert_eq!(ellipse.background_color, "#d5e8d4");

        let labels: Vec<_> = els
            .iter()
            .filter(|e| e.element_type == "text")
            .filter_map(|e| e.text.clone())
            .collect();
        assert!(labels.contains(&"Start".to_string()));
        assert!(labels.contains(&"End".to_string()));

        let arrow = els.iter().find(|e| e.element_type == "arrow").unwrap();
        let pts = arrow.points.as_ref().unwrap();
        assert_eq!(pts.len(), 2);
        // Source center (100,70) -> target center (350,240): first point is origin.
        assert_eq!(arrow.x, 100.0);
        assert_eq!(arrow.y, 70.0);
        assert_eq!(pts[0], [0.0, 0.0]);
        assert_eq!(pts[1], [250.0, 170.0]);
    }

    #[test]
    fn rejects_compressed_drawio() {
        let xml = r#"<mxfile><diagram>7VtZc+I4EP41VO0+...</diagram></mxfile>"#;
        let err = parse(xml).unwrap_err();
        assert!(err.contains("compressed"));
    }

    #[test]
    fn strips_html_labels() {
        assert_eq!(clean_value("<b>Hi</b>&nbsp;there"), "Hi there");
        assert_eq!(clean_value("line1<br>line2"), "line1\nline2");
    }
}
