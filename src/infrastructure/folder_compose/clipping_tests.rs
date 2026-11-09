use super::*;

#[test]
fn media_does_not_change_the_lower_folder_shell() {
    let composer = FolderComposer::try_new().unwrap();
    for output_w in BUCKET_OUTPUT_WIDTHS {
        let layers = composer.layers_for(output_w);
        let mut shell = RgbaImage::new(output_w, layers.canvas_h);
        let bx = output_w.saturating_sub(layers.back.width()) / 2;
        imageops::overlay(&mut shell, &layers.back, bx as i64, layers.back_y as i64);
        let fx = output_w.saturating_sub(layers.front.width()) / 2;
        imageops::overlay(&mut shell, &layers.front, fx as i64, layers.front_y as i64);

        for (content_w, content_h) in [(32, 128), (64, 64), (128, 72), (128, 16)] {
            let media = [255, 0, 255, 255].repeat(content_w * content_h);
            let (pixels, w, h) = composer
                .compose_for_size(&media, content_w as u32, content_h as u32, output_w)
                .unwrap();
            let composed = RgbaImage::from_raw(w, h, pixels).unwrap();
            for y in layers.front_y + layers.front.height() / 2..h {
                for x in 0..w {
                    assert_eq!(
                        composed.get_pixel(x, y),
                        shell.get_pixel(x, y),
                        "media leaked into shell at ({x}, {y}), bucket {output_w}, content {content_w}x{content_h}"
                    );
                }
            }

            // The fix must not hide or shift the exposed media above the front flap.
            let x = w / 2;
            let y = layers.back_y + layers.content_margin_top;
            assert_eq!(composed.get_pixel(x, y).0, [255, 0, 255, 255]);

            // Compare the whole upper region with the previous, uncropped composition,
            // including the part of the thumbnail behind the flap's sloped contour.
            let gap_w = w - layers.content_margin_left - layers.content_margin_right;
            let scaled_h = (content_h as f32 * gap_w as f32 / content_w as f32).round() as u32;
            let mut reference = RgbaImage::new(w, h);
            imageops::overlay(
                &mut reference,
                &layers.back,
                bx as i64,
                layers.back_y as i64,
            );
            let media = RgbaImage::from_pixel(gap_w, scaled_h, image::Rgba([255, 0, 255, 255]));
            imageops::overlay(
                &mut reference,
                &media,
                layers.content_margin_left as i64,
                y as i64,
            );
            imageops::overlay(
                &mut reference,
                &layers.front,
                fx as i64,
                layers.front_y as i64,
            );
            for y in 0..layers.front_y + layers.front.height() / 2 {
                for x in 0..w {
                    assert_eq!(composed.get_pixel(x, y), reference.get_pixel(x, y));
                }
            }
        }
    }
}
