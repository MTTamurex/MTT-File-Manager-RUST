use super::*;

#[test]
fn badge_is_hidden_for_one_window_and_caps_large_numbers() {
    assert_eq!(label(0), None);
    assert_eq!(label(1), None);
    for count in 2..=9 {
        assert_eq!(label(count), Some(count.to_string().as_str()));
    }
    assert_eq!(label(10), Some("9+"));
    assert_eq!(label(usize::MAX), Some("9+"));
}

#[test]
fn rendering_preserves_the_base_and_uses_valid_premultiplied_alpha() {
    for size in [16, 24, 32, 48, 64, 128] {
        let base = render(1, size).unwrap();
        assert_eq!(base.data(), render(0, size).unwrap().data());
        let two = render(2, size).unwrap();
        let three = render(3, size).unwrap();
        assert_ne!(base.data(), two.data());
        assert_ne!(two.data(), three.data());
        assert_eq!(
            render(10, size).unwrap().data(),
            render(100, size).unwrap().data()
        );
        // The badge occupies the top of the icon, not the lower app artwork.
        let start = (size * size * 3 / 4 * 4) as usize;
        assert_eq!(&base.data()[start..], &two.data()[start..]);
        for pixel in two.data().chunks_exact(4) {
            assert!(pixel[..3].iter().all(|&channel| channel <= pixel[3]));
        }
    }
    assert!(render(2, 0).is_none());
    assert!(render(2, 1024).is_none());
}

#[test]
fn compact_digits_remain_visible_at_notification_area_size() {
    for count in 2..=10 {
        let pixels = render(count, 16).unwrap();
        // Inspect the interior, excluding the white border of the badge.
        let white_pixels = (3..9)
            .flat_map(|y| (7..14).map(move |x| (y * 16 + x) * 4))
            .filter(|&index| {
                let pixel = &pixels.data()[index..index + 4];
                pixel[..3].iter().all(|&channel| channel > 200) && pixel[3] > 240
            })
            .count();
        assert!(
            white_pixels >= 3,
            "count {count} has too little visible numeral ink"
        );
    }
}
