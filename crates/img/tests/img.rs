use sol_img::{decode, encode_png, Image};

#[test]
fn png_rgb() {
    let img = decode(include_bytes!("t2x2.png")).unwrap();
    assert_eq!((img.w, img.h), (2, 2));
    assert_eq!(img.get(0, 0), [255, 0, 0, 255]);
    assert_eq!(img.get(1, 0), [0, 255, 0, 255]);
    assert_eq!(img.get(0, 1), [0, 0, 255, 255]);
    assert_eq!(img.get(1, 1), [255, 255, 255, 255]);
}

#[test]
fn bmp_24() {
    let img = decode(include_bytes!("t2x2.bmp")).unwrap();
    assert_eq!((img.w, img.h), (2, 2));
    assert_eq!(img.get(0, 0), [255, 0, 0, 255]); // top-left red
    assert_eq!(img.get(1, 0), [0, 255, 0, 255]);
    assert_eq!(img.get(0, 1), [0, 0, 255, 255]); // bottom-left blue
    assert_eq!(img.get(1, 1), [255, 255, 255, 255]);
}

#[test]
fn gif_2x1() {
    let img = decode(include_bytes!("t2x1.gif")).unwrap();
    assert_eq!((img.w, img.h), (2, 1));
    assert_eq!(img.get(0, 0), [0, 0, 0, 255]);
    assert_eq!(img.get(1, 0), [255, 255, 255, 255]);
}

#[test]
fn png_roundtrip() {
    let mut img = Image::new(3, 2);
    img.set(0, 0, [10, 20, 30, 255]);
    img.set(2, 1, [200, 100, 50, 128]);
    img.set(1, 1, [0, 128, 255, 255]);
    let enc = encode_png(&img);
    let dec = decode(&enc).unwrap();
    assert_eq!((dec.w, dec.h), (3, 2));
    assert_eq!(dec.get(0, 0), [10, 20, 30, 255]);
    assert_eq!(dec.get(2, 1), [200, 100, 50, 128]);
    assert_eq!(dec.get(1, 1), [0, 128, 255, 255]);
}
