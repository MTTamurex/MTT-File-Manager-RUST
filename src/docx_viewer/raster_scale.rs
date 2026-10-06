use docx_layout::display_list::DisplayPage;
use serde_json::{Number, Value};

const TARGET_RASTER_SCALE: f64 = 2.0;
const GEOMETRY_FIELDS: &[&str] = &[
    "advance",
    "ascent",
    "baseline",
    "baselineY",
    "bottom",
    "cp1x",
    "cp1y",
    "cp2x",
    "cp2y",
    "cpx",
    "cpy",
    "customDash",
    "dash",
    "descent",
    "fontSize",
    "h",
    "height",
    "left",
    "letterSpacing",
    "lineWidth",
    "right",
    "size",
    "strokeWidth",
    "top",
    "w",
    "width",
    "wordSpacing",
    "x",
    "x1",
    "x2",
    "y",
    "y1",
    "y2",
];

pub(super) fn high_resolution_scale(width: f64, height: f64) -> f64 {
    let max_side = width.max(height);
    let pixel_count = width * height;
    let side_limit = f64::from(docx_raster::MAX_PAGE_DIM) / max_side;
    let pixel_limit = (docx_raster::MAX_PAGE_PIXELS as f64 / pixel_count).sqrt();
    TARGET_RASTER_SCALE.min(side_limit).min(pixel_limit)
}

pub(super) fn scale_display_page(page: &DisplayPage, scale: f64) -> Result<DisplayPage, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("page raster scale must be finite and positive".to_owned());
    }
    if scale == 1.0 {
        return Ok(page.clone());
    }

    let mut value = serde_json::to_value(page).map_err(|error| error.to_string())?;
    scale_geometry(&mut value, None, scale)?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn scale_geometry(value: &mut Value, field: Option<&str>, scale: f64) -> Result<(), String> {
    match value {
        Value::Object(values) => {
            for (name, value) in values {
                if name == "crop" {
                    continue;
                }
                if name == "font" {
                    if let Value::String(font) = value {
                        scale_css_font(font, scale);
                    }
                    continue;
                }
                scale_geometry(value, Some(name), scale)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                scale_geometry(value, field, scale)?;
            }
        }
        Value::Number(number) if field.is_some_and(|name| GEOMETRY_FIELDS.contains(&name)) => {
            *number = scale_number(number, scale)?;
        }
        _ => {}
    }
    Ok(())
}

fn scale_number(number: &Number, scale: f64) -> Result<Number, String> {
    let value = number
        .as_f64()
        .ok_or_else(|| "page geometry is not numeric".to_owned())?
        * scale;
    Number::from_f64(value).ok_or_else(|| "scaled page geometry is not finite".to_owned())
}

fn scale_css_font(font: &mut String, scale: f64) {
    let Some(unit_start) = font.find("px") else {
        return;
    };
    let size_start = font[..unit_start]
        .rfind(|character: char| !character.is_ascii_digit() && character != '.')
        .map_or(0, |index| index + 1);
    let Ok(size) = font[size_start..unit_start].parse::<f64>() else {
        return;
    };
    if !size.is_finite() || size <= 0.0 {
        return;
    }
    font.replace_range(size_start..unit_start, &format!("{:.3}", size * scale));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{high_resolution_scale, scale_display_page};
    use docx_layout::display_list::DisplayPage;

    #[test]
    fn doubles_a4_raster_resolution_within_page_limits() {
        assert_eq!(high_resolution_scale(794.0, 1123.0), 2.0);
        assert_eq!(high_resolution_scale(8192.0, 2048.0), 1.0);
    }

    #[test]
    fn scales_image_and_text_geometry_but_preserves_crop_and_opacity() {
        let page: DisplayPage = serde_json::from_value(json!({
            "pageIndex": 0,
            "width": 794,
            "height": 1123,
            "primitives": [
                {
                    "kind": "image",
                    "relId": "image1",
                    "x": 10,
                    "y": 20,
                    "w": 30,
                    "h": 40,
                    "opacity": 0.5,
                    "rotationDeg": 15,
                    "crop": {"left": 0.1, "top": 0.2, "right": 0.3, "bottom": 0.4}
                },
                {
                    "kind": "text",
                    "text": "sample",
                    "x": 2,
                    "baselineY": 12,
                    "width": 18,
                    "font": "11px Arial",
                    "color": "#000000"
                }
            ]
        }))
        .unwrap();

        let scaled = scale_display_page(&page, 2.0).unwrap();
        let value = serde_json::to_value(scaled).unwrap();
        assert_eq!(value["width"].as_f64(), Some(1588.0));
        assert_eq!(value["height"].as_f64(), Some(2246.0));
        assert_eq!(value["primitives"][0]["x"].as_f64(), Some(20.0));
        assert_eq!(value["primitives"][0]["w"].as_f64(), Some(60.0));
        assert_eq!(value["primitives"][0]["crop"]["left"].as_f64(), Some(0.1));
        assert_eq!(value["primitives"][0]["opacity"].as_f64(), Some(0.5));
        assert_eq!(value["primitives"][0]["rotationDeg"].as_f64(), Some(15.0));
        assert_eq!(value["primitives"][1]["font"], "22.000px Arial");
        assert_eq!(value["primitives"][1]["baselineY"].as_f64(), Some(24.0));
    }
}
