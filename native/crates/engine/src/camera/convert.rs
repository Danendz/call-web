//! Camera pixels to the I420 frames WebRTC encodes, cropped to the picked shape on the way.

use std::time::Instant;

use libwebrtc::prelude::I420Buffer;

use crate::video::CAMERA_US;

/// Runs a conversion and records how long it took, for the stats overlay.
pub(crate) fn timed<T>(convert: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = convert();
    CAMERA_US.record(started.elapsed().as_micros());
    out
}

/// The centered part of a `width` x `height` picture that has the wanted shape: all of it for landscape (cameras
/// already send 16:9), a 9:16 slice for portrait. Even, because chroma covers 2x2 pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Crop {
    pub(crate) fn new(width: u32, height: u32, portrait: bool) -> Self {
        let (w, h) = if portrait && width * 16 > height * 9 {
            ((height * 9 / 16) & !1, height & !1)
        } else {
            (width & !1, height & !1)
        };
        Self { x: ((width - w) / 2) & !1, y: ((height - h) / 2) & !1, width: w, height: h }
    }
}

/// Two-plane 4:2:0 (Y, then interleaved UV), as macOS and most Windows cameras deliver it. Done here rather than by
/// libyuv, whose wrapper wants `stride * height` bytes after the crop's first pixel and panics on a portrait slice of
/// a plane that holds exactly the picture.
#[cfg_attr(target_os = "android", expect(dead_code, reason = "Android's cameras deliver YUV_420_888"))]
pub(crate) fn nv12_to_i420(y: &[u8], y_stride: u32, uv: &[u8], uv_stride: u32, crop: Crop) -> Option<I420Buffer> {
    if crop.width == 0 || crop.height == 0 {
        return None;
    }
    let mut out = I420Buffer::new(crop.width, crop.height);
    let (sy, su, sv) = out.strides();
    let (dy, du, dv) = out.data_mut();
    let (w, x) = (crop.width as usize, crop.x as usize);
    let (y_stride, uv_stride, sy, su, sv) =
        (y_stride as usize, uv_stride as usize, sy as usize, su as usize, sv as usize);
    for row in 0..crop.height as usize {
        let from = (crop.y as usize + row) * y_stride + x;
        dy.get_mut(row * sy..row * sy + w)?.copy_from_slice(y.get(from..from + w)?);
    }
    for row in 0..crop.height as usize / 2 {
        let from = (crop.y as usize / 2 + row) * uv_stride + x;
        let src = uv.get(from..from + w)?;
        let u = du.get_mut(row * su..row * su + w / 2)?;
        let v = dv.get_mut(row * sv..row * sv + w / 2)?;
        for (([cb, cr], u), v) in src.as_chunks::<2>().0.iter().zip(u).zip(v) {
            (*u, *v) = (*cb, *cr);
        }
    }
    Some(out)
}

/// Android's YUV_420_888: a Y plane, then U and V planes whose samples are `uv_pixel_stride` apart (1 when planar,
/// 2 when they interleave like NV12 or NV21).
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) struct Yuv420<'a> {
    pub y: &'a [u8],
    pub y_stride: usize,
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub uv_stride: usize,
    pub uv_pixel_stride: usize,
}

/// The crop of a [`Yuv420`] picture turned clockwise by `rotation` (0, 90, 180 or 270), as an upright I420 frame: a
/// phone's camera sensor lies sideways in it.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) fn yuv420_to_i420(src: &Yuv420<'_>, crop: Crop, rotation: u32) -> Option<I420Buffer> {
    if crop.width == 0 || crop.height == 0 {
        return None;
    }
    let sideways = rotation % 180 == 90;
    let (w, h) = if sideways { (crop.height, crop.width) } else { (crop.width, crop.height) };
    let mut out = I420Buffer::new(w, h);
    let (sy, su, sv) = out.strides();
    let (dy, du, dv) = out.data_mut();
    // Where the output's pixel (x, y) is in the crop of a plane `cw` x `ch` pixels big
    let at = |x: usize, y: usize, cw: usize, ch: usize| match rotation {
        90 => (y, ch - 1 - x),
        180 => (cw - 1 - x, ch - 1 - y),
        270 => (cw - 1 - y, x),
        _ => (x, y),
    };
    let (w, h, cw, ch) = (w as usize, h as usize, crop.width as usize, crop.height as usize);
    let (x0, y0) = (crop.x as usize, crop.y as usize);
    for row in 0..h {
        for col in 0..w {
            let (x, y) = at(col, row, cw, ch);
            *dy.get_mut(row * sy as usize + col)? = *src.y.get((y0 + y) * src.y_stride + x0 + x)?;
        }
    }
    for row in 0..h / 2 {
        for col in 0..w / 2 {
            let (x, y) = at(col, row, cw / 2, ch / 2);
            let from = (y0 / 2 + y) * src.uv_stride + (x0 / 2 + x) * src.uv_pixel_stride;
            *du.get_mut(row * su as usize + col)? = *src.u.get(from)?;
            *dv.get_mut(row * sv as usize + col)? = *src.v.get(from)?;
        }
    }
    Some(out)
}

/// Packed 4:2:2 (Y0 U Y1 V), what USB cameras offer uncompressed. Chroma of each row pair is averaged.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn yuyv_to_i420(src: &[u8], stride: u32, crop: Crop) -> Option<I420Buffer> {
    let needed = ((crop.y + crop.height - 1) * stride + (crop.x + crop.width) * 2) as usize;
    if crop.width == 0 || crop.height == 0 || src.len() < needed {
        return None;
    }
    let mut out = I420Buffer::new(crop.width, crop.height);
    let (sy, su, sv) = out.strides();
    let (dy, du, dv) = out.data_mut();
    let (stride, sy, su, sv) = (stride as usize, sy as usize, su as usize, sv as usize);
    for row in (0..crop.height as usize).step_by(2) {
        let top = (crop.y as usize + row) * stride + crop.x as usize * 2;
        let bottom = top + stride;
        for col in (0..crop.width as usize).step_by(2) {
            let [ty0, tu, ty1, tv]: [u8; 4] = src.get(top + col * 2..top + col * 2 + 4)?.try_into().ok()?;
            let [by0, bu, by1, bv]: [u8; 4] = src.get(bottom + col * 2..bottom + col * 2 + 4)?.try_into().ok()?;
            *dy.get_mut(row * sy + col)? = ty0;
            *dy.get_mut(row * sy + col + 1)? = ty1;
            *dy.get_mut((row + 1) * sy + col)? = by0;
            *dy.get_mut((row + 1) * sy + col + 1)? = by1;
            *du.get_mut(row / 2 * su + col / 2)? = u16::midpoint(u16::from(tu), u16::from(bu)) as u8;
            *dv.get_mut(row / 2 * sv + col / 2)? = u16::midpoint(u16::from(tv), u16::from(bv)) as u8;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use libwebrtc::prelude::VideoBuffer;

    use super::*;

    #[test]
    fn landscape_keeps_the_whole_picture() {
        assert_eq!(Crop::new(1280, 720, false), Crop { x: 0, y: 0, width: 1280, height: 720 });
    }

    #[test]
    fn portrait_takes_an_even_centered_slice() {
        let crop = Crop::new(1280, 720, true);
        assert_eq!((crop.width, crop.height), (404, 720));
        assert_eq!(crop.x, 438);
        assert!(crop.x.is_multiple_of(2) && crop.width.is_multiple_of(2));
    }

    #[test]
    fn portrait_of_a_tall_picture_is_left_alone() {
        assert_eq!(Crop::new(720, 1280, true), Crop { x: 0, y: 0, width: 720, height: 1280 });
    }

    #[test]
    fn yuyv_picks_lumas_and_averages_chroma_with_padded_rows() {
        // 4x2 picture, rows padded to 12 bytes
        let src = [
            10, 100, 11, 200, 12, 50, 13, 60, 0, 0, 0, 0, //
            20, 110, 21, 210, 22, 70, 23, 80, 0, 0, 0, 0,
        ];
        let out = yuyv_to_i420(&src, 12, Crop::new(4, 2, false)).unwrap();
        let (y, u, v) = out.data();
        let (sy, _, _) = out.strides();
        assert_eq!(&y[..4], &[10, 11, 12, 13]);
        assert_eq!(&y[sy as usize..sy as usize + 4], &[20, 21, 22, 23]);
        assert_eq!(&u[..2], &[105, 60]);
        assert_eq!(&v[..2], &[205, 70]);
    }

    #[test]
    fn nv12_portrait_slice_of_an_exactly_sized_plane() {
        // 32x8 picture whose planes end right after the last row, as a camera's do; the 9:16 slice is 4 wide at
        // x 14. libyuv's wrapper panicked here, taking the camera's thread and the app with it
        let y: Vec<u8> = (0..=255).collect();
        let uv: Vec<u8> = (0..128).collect();
        let crop = Crop::new(32, 8, true);
        assert_eq!(crop, Crop { x: 14, y: 0, width: 4, height: 8 });
        let out = nv12_to_i420(&y, 32, &uv, 32, crop).unwrap();
        let (oy, u, v) = out.data();
        let (sy, su, _) = out.strides();
        assert_eq!(&oy[..4], &[14, 15, 16, 17]);
        assert_eq!(&oy[7 * sy as usize..7 * sy as usize + 4], &[238, 239, 240, 241]);
        assert_eq!((&u[..2], &v[..2]), (&[14, 16][..], &[15, 17][..]));
        assert_eq!(u[3 * su as usize], 3 * 32 + 14);
    }

    #[test]
    fn yuv420_turns_a_sideways_sensor_upright() {
        // 4x2 picture with interleaved chroma (NV21: V first, U one byte on), turned 90° clockwise into 2x4. The U
        // plane is one byte short of V, as Android hands it
        let y = [1, 2, 3, 4, 5, 6, 7, 8];
        let vu = [20, 10, 21, 11];
        let src = Yuv420 { y: &y, y_stride: 4, u: &vu[1..], v: &vu, uv_stride: 4, uv_pixel_stride: 2 };
        let out = yuv420_to_i420(&src, Crop::new(4, 2, false), 90).unwrap();
        let (oy, u, v) = out.data();
        let (sy, ..) = out.strides();
        let rows: Vec<&[u8]> = (0..4).map(|r| &oy[r * sy as usize..r * sy as usize + 2]).collect();
        assert_eq!(rows, [&[5, 1][..], &[6, 2], &[7, 3], &[8, 4]]);
        assert_eq!((u[0], v[0]), (10, 20));
        assert_eq!((out.width(), out.height()), (2, 4));
    }

    #[test]
    fn yuv420_turned_the_other_ways() {
        let y = [1, 2, 3, 4, 5, 6, 7, 8];
        let (u, v) = ([0; 2], [0; 2]);
        let src = Yuv420 { y: &y, y_stride: 4, u: &u, v: &v, uv_stride: 2, uv_pixel_stride: 1 };
        let luma = |rotation| {
            let out = yuv420_to_i420(&src, Crop::new(4, 2, false), rotation).unwrap();
            let (sy, ..) = out.strides();
            let (w, h) = (out.width() as usize, out.height() as usize);
            (0..h).flat_map(|r| out.data().0[r * sy as usize..r * sy as usize + w].to_vec()).collect::<Vec<_>>()
        };
        assert_eq!(luma(0), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(luma(180), [8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(luma(270), [4, 8, 3, 7, 2, 6, 1, 5]);
    }

    #[test]
    fn short_buffers_are_refused_not_read_past() {
        assert!(yuyv_to_i420(&[0; 10], 8, Crop::new(4, 2, false)).is_none());
        assert!(nv12_to_i420(&[0; 7], 4, &[0; 4], 4, Crop::new(4, 2, false)).is_none());
    }
}
