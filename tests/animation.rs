use std::sync::Arc;
use velocity::{AnimatedSprite, SpriteFrames, Texture, vec2};
fn texture() -> Arc<Texture> {
    Arc::new(Texture::from_rgba(128, 96, vec![255; 128 * 96 * 4]).unwrap())
}
fn frames() -> Arc<SpriteFrames> {
    Arc::new(SpriteFrames::from_sparrow(texture(), include_str!("fixtures/sparrow.xml")).unwrap())
}
#[test]
fn fnf_pinned_flixel_golden_decode() {
    let frames = frames();
    let rows: Vec<_> = include_str!("fixtures/sparrow.tsv").lines().collect();
    assert_eq!(frames.len(), rows.len());
    for (i, row) in rows.iter().enumerate() {
        let fields: Vec<_> = row.split('\t').collect();
        let f = frames.get(i).unwrap();
        assert_eq!(frames.name(i).unwrap(), fields[0]);
        let actual = [
            f.rect[0],
            f.rect[1],
            f.rect[2],
            f.rect[3],
            f.source_size.x,
            f.source_size.y,
            f.offset.x,
            f.offset.y,
        ];
        for (n, (&a, e)) in actual.iter().zip(&fields[1..9]).enumerate() {
            assert_eq!(a, e.parse::<f32>().unwrap(), "frame {i} component {n}");
        }
        assert_eq!(
            if f.rotated { -90 } else { 0 },
            fields[9].parse::<i32>().unwrap(),
            "angle {i}"
        );
        assert_eq!(f.flip_x, fields[10] == "true");
        assert_eq!(f.flip_y, fields[11] == "true");
        assert_eq!(f.empty, fields[12] == "true");
    }
    let names: usize = (0..frames.len())
        .map(|i| frames.name(i).unwrap().len())
        .sum();
    assert_eq!(frames.metadata_bytes(), frames.len() * 40 + names);
    println!(
        "{} golden frames; {} total metadata bytes; 32 geometry + 8 name/flags bytes per frame",
        frames.len(),
        frames.metadata_bytes()
    );
}
#[test]
fn manual_frames_playback_and_mutation() {
    let mut f = SpriteFrames::new(texture());
    for i in [10, 2, 1] {
        f.add_frame(
            &format!("idle{i}"),
            velocity::Frame::new([i as f32, 0., 2., 3.]),
        )
        .unwrap();
    }
    let mut s = AnimatedSprite::new(Arc::new(f));
    s.position = vec2(20., 30.);
    s.set_position(vec2(2., 3.));
    assert_eq!(s.position, vec2(2., 3.));
    s.add_by_prefix("idle", "idle", 4., true).unwrap();
    s.play("idle").unwrap();
    assert_eq!(s.frame(), 0);
    assert_eq!(s.animation(), Some("idle"));
    assert_eq!(s.framerate(), 4.);
    s.update(0.125);
    assert_eq!(s.frame(), 0);
    s.update(0.125);
    assert_eq!(s.frame(), 1);
    s.update(1000.);
    assert_eq!(s.frame(), 2);
    s.pause();
    s.update(4.);
    assert_eq!(s.frame(), 2);
    s.resume();
    s.update(0.25);
    assert_eq!(s.frame(), 0);
    s.set_frame(2).unwrap();
    s.stop();
    assert_eq!(s.frame(), 0);
    assert!(!s.playing());
    s.rename_animation("idle", "walk").unwrap();
    assert_eq!(s.animation(), Some("walk"));
    s.set_looping("walk", false).unwrap();
    s.set_looping("walk", true).unwrap();
    assert!(s.set_looping("missing", true).is_err());
    s.set_framerate("walk", 0.).unwrap();
    s.resume();
    s.update(f32::MAX);
    assert_eq!(s.frame(), 0);
    s.add_animation("once", &[0, 1, 2], 2., false).unwrap();
    s.play("once").unwrap();
    s.update(1.);
    assert_eq!(s.frame(), 2);
    assert!(s.playing());
    s.update(0.5);
    assert!(!s.playing());
    assert_eq!(s.frame(), 2);
    assert!(s.set_frame(3).is_err());
    assert!(s.play("missing").is_err());
    assert!(s.add_animation("bad", &[99], 1., true).is_err());
    for bad in [f32::NAN, f32::INFINITY, -1.] {
        assert!(s.set_framerate("once", bad).is_err());
        s.update(bad);
        assert_eq!(s.frame(), 2);
    }
    s.play("walk").unwrap();
    s.set_framerate("walk", f32::MAX).unwrap();
    s.update(f32::MAX);
    assert!(s.frame() < 3);
}
#[test]
fn malformed_xml_and_empty_atlas() {
    for xml in [
        "",
        "<TextureAtlas>",
        "<TextureAtlas></Wrong>",
        "<TextureAtlas/><TextureAtlas/>",
        "<TextureAtlas><SubTexture name='a' x='0' y='0' width='NaN' height='1'/></TextureAtlas>",
        "<!DOCTYPE TextureAtlas><TextureAtlas/>",
        "garbage<TextureAtlas/>",
    ] {
        assert!(SpriteFrames::from_sparrow(texture(), xml).is_err(), "{xml}");
    }
    let f = SpriteFrames::from_sparrow(texture(), "<TextureAtlas/>").unwrap();
    assert!(f.is_empty());
    let mut s = AnimatedSprite::new(Arc::new(f));
    s.update(1.);
    assert!(!s.playing());
    assert_eq!(s.framerate(), 0.);
}
