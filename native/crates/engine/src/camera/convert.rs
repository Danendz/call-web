//! Camera pixels to the I420 frames WebRTC encodes, cropped to the picked shape on the way.

use libwebrtc::native::yuv_helper;
use libwebrtc::prelude::I420Buffer;

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

/// Two-plane 4:2:0 (Y, then interleaved UV), as macOS and most Windows cameras deliver it.
pub(crate) fn nv12_to_i420(y: &[u8], y_stride: u32, uv: &[u8], uv_stride: u32, crop: Crop) -> Option<I420Buffer> {
    let y_start = (crop.y * y_stride + crop.x) as usize;
    let uv_start = (crop.y / 2 * uv_stride + crop.x) as usize;
    let y_needed = y_start + ((crop.height - 1) * y_stride + crop.width) as usize;
    let uv_needed = uv_start + ((crop.height / 2 - 1) * uv_stride + crop.width) as usize;
    if crop.width == 0 || crop.height == 0 || y.len() < y_needed || uv.len() < uv_needed {
        return None;
    }
    let mut out = I420Buffer::new(crop.width, crop.height);
    let (sy, su, sv) = out.strides();
    let (dy, du, dv) = out.data_mut();
    yuv_helper::nv12_to_i420(
        y.get(y_start..)?,
        y_stride,
        uv.get(uv_start..)?,
        uv_stride,
        dy,
        sy,
        du,
        su,
        dv,
        sv,
        crop.width as i32,
        crop.height as i32,
    );
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
    fn short_buffers_are_refused_not_read_past() {
        assert!(yuyv_to_i420(&[0; 10], 8, Crop::new(4, 2, false)).is_none());
        assert!(nv12_to_i420(&[0; 7], 4, &[0; 4], 4, Crop::new(4, 2, false)).is_none());
    }
}
