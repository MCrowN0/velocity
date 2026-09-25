use super::super::ffi as libaom_sys;
use super::{Error, Result};
use libaom_sys::*;
use std::ffi::CStr;
use std::ffi::c_int;
use std::fmt;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::num::NonZeroU32;
use std::ptr;
use std::ptr::NonNull;
use yuv::color::{
    ChromaSamplePosition, ChromaSampling, ColorPrimaries, Depth, MatrixCoefficients, Range,
    TransferCharacteristics,
};

/// AOM decoder context
pub struct Decoder {
    ctx: aom_codec_ctx,
}

/// Result of [`Decoder::frame_meta()`]
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct FrameMeta {
    /// The width the current frame is decoded at (note this may be different
    /// to the display width of the current frame)
    pub frame_width: u32,
    /// The height the current frame is decoded at (note this may be different
    /// to the display height of the current frame)
    pub frame_height: u32,

    /// The intended display width of the current frame (note this may be
    /// different to the width the frame is decoded at)
    pub display_width: u32,
    /// The intended display height of the current frame (note this may be
    /// different to the height the frame is decoded at).
    pub display_height: u32,
}

/// Configuration for the decoer. For now it's just number of threads.
///
/// You can use `std::thread::available_parallelism().map(|v| v.get()).unwrap_or(4)`.
#[derive(Debug, Clone)]
pub struct Config {
    pub threads: usize,
}

impl Decoder {
    /// Create a new decoder
    pub fn new(cfg: &Config) -> Result<Self> {
        let cfg = aom_codec_dec_cfg {
            w: 0,
            h: 0,
            threads: cfg.threads as _,
            allow_lowbitdepth: 1,
        };
        unsafe {
            let mut ctx = MaybeUninit::uninit();
            let res = aom_codec_dec_init_ver(
                ctx.as_mut_ptr(),
                aom_codec_av1_dx(),
                &cfg,
                0,
                AOM_DECODER_ABI_VERSION as i32,
            );
            if let Some(code) = NonZeroU32::new(res) {
                Err(Error::AOM(code, None))
            } else {
                Ok(Self {
                    ctx: ctx.assume_init(),
                })
            }
        }
    }

    /// This parses the AV1 data independently of `decode_frame`
    #[inline]
    pub fn frame_meta(&mut self, av1_data: &[u8]) -> Result<FrameMeta> {
        let res = unsafe {
            aom_codec_decode(
                &mut self.ctx,
                av1_data.as_ptr(),
                av1_data.len(),
                ptr::null_mut(),
            )
        };
        self.is_err(res)?;

        #[cold]
        fn overflow_err<E>(_: E) -> Error {
            Error::Unsupported("overflow")
        }

        let mut frame_w_h = [0 as c_int; 2];
        let res = unsafe {
            aom_codec_control(
                &mut self.ctx,
                AV1D_GET_FRAME_SIZE.try_into().map_err(overflow_err)?,
                frame_w_h.as_mut_ptr(),
            )
        };
        self.is_err(res)?;

        let mut display_w_h = [0 as c_int; 2];
        let res = unsafe {
            aom_codec_control(
                &mut self.ctx,
                AV1D_GET_DISPLAY_SIZE.try_into().map_err(overflow_err)?,
                display_w_h.as_mut_ptr(),
            )
        };
        self.is_err(res)?;

        Ok(FrameMeta {
            frame_width: frame_w_h[0].try_into().map_err(overflow_err)?,
            frame_height: frame_w_h[1].try_into().map_err(overflow_err)?,

            display_width: display_w_h[0].try_into().map_err(overflow_err)?,
            display_height: display_w_h[1].try_into().map_err(overflow_err)?,
        })
    }

    /// Take AV1-compressed data and decode a *single* frame into raw frame data (YUV pixels). This is for AVIF, and can't handle video.
    ///
    /// The returned frame is temporary. You must copy data out of it and drop it before decoding other files.
    ///
    /// See [yuv](https://lib.rs/yuv) crate for conversion to RGB.
    #[inline]
    pub fn decode_frame<'a>(&'a mut self, av1_data: &[u8]) -> Result<FrameTempRef<'a>> {
        Ok(FrameTempRef(
            unsafe {
                let res = aom_codec_decode(
                    &mut self.ctx,
                    av1_data.as_ptr(),
                    av1_data.len(),
                    ptr::null_mut(),
                );
                self.is_err(res)?;

                let mut iter = ptr::null();
                let res = aom_codec_get_frame(&mut self.ctx, &mut iter);
                self.err_if_null(res)?
            },
            PhantomData,
        ))
    }

    #[inline]
    fn is_err(&self, res: u32) -> Result<()> {
        if let Some(code) = NonZeroU32::new(res) {
            Err(Error::AOM(code, self.last_error_msg()))
        } else {
            Ok(())
        }
    }

    #[inline]
    fn err_if_null<T>(&self, ptr: *const T) -> Result<NonNull<T>, Error> {
        self.err_if_null_mut(ptr.cast_mut())
    }

    fn err_if_null_mut<T>(&self, ptr: *mut T) -> Result<NonNull<T>, Error> {
        if let Some(ptr) = NonNull::new(ptr) {
            Ok(ptr)
        } else {
            Err(Error::AOM(
                NonZeroU32::new(libaom_sys::AOM_CODEC_ERROR).unwrap(),
                self.last_error_msg(),
            ))
        }
    }

    fn last_error_msg(&self) -> Option<String> {
        let s = unsafe {
            let err = aom_codec_error(std::ptr::from_ref(&self.ctx).cast_mut());
            if err.is_null() {
                return None;
            }
            CStr::from_ptr(err).to_string_lossy()
        };
        Some(s.into_owned())
    }
}

impl Drop for Decoder {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            aom_codec_destroy(&mut self.ctx);
        }
    }
}

/// Iterates over rows of pixels in a Y/U/V plane
pub struct RowsIter<'a, Pixel> {
    plane_start: *const u8,
    stride_bytes: isize,
    w: usize,
    h: usize,
    y: isize,
    _data: PhantomData<&'a [Pixel]>,
}

impl<T> RowsIter<'_, T> {
    #[inline(always)]
    #[must_use]
    pub const fn width(&self) -> usize {
        self.w
    }
    #[inline(always)]
    #[must_use]
    pub const fn height(&self) -> usize {
        self.h
    }
}

impl<'a, Pixel> Iterator for RowsIter<'a, Pixel> {
    type Item = &'a [Pixel];
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let y = self.y;
        if (self.y as usize) < self.h {
            self.y += 1;
            // stride may be negative for flipped images?
            Some(unsafe {
                debug_assert_eq!(
                    0,
                    self.stride_bytes.unsigned_abs() % std::mem::align_of::<Pixel>()
                );
                let ptr = self.plane_start.offset(y * self.stride_bytes);
                std::slice::from_raw_parts(ptr.cast::<Pixel>(), self.w)
            })
        } else {
            None
        }
    }
}

/// Iterators of frame's pixels (YUV planes)
///
/// It's an enum, because frames may have different pixel formats, and you get format-specific iterator in the enum
pub enum RowsIters<'a> {
    /// 8-bit YUV (YCbCr, YCgCo, etc.). This is a collection of 3 iterators. Each may have a different size depending on `chroma_sampling`.
    ///
    /// Consult `matrix_coefficients()` for meaning of these channels
    YuvPlanes8 {
        y: RowsIter<'a, u8>,
        u: RowsIter<'a, u8>,
        v: RowsIter<'a, u8>,
        chroma_sampling: ChromaSampling,
    },
    /// 8-bit grayscale
    Mono8(RowsIter<'a, u8>),
    /// 10/12/16-bit color. Not subsampled. See `depth` field for actual range of pixels used.
    ///
    /// It's not `u16`, because it's not guaranteed to be aligned. Use `u16::from_ne_bytes()` or `ptr::read_unaligned`.
    YuvPlanes16 {
        y: RowsIter<'a, [u8; 2]>,
        u: RowsIter<'a, [u8; 2]>,
        v: RowsIter<'a, [u8; 2]>,
        chroma_sampling: ChromaSampling,
        depth: Depth,
    },
    /// 10/12/16-bit grayscale. It's not `u16`, because it's not guaranteed to be aligned. Use `u16::from_ne_bytes()` or `ptr::read_unaligned`.
    Mono16(RowsIter<'a, [u8; 2]>, Depth),
}

/// Frame held in decoder's internal state. Must be dropped before the next call.
pub struct FrameTempRef<'a>(NonNull<aom_image_t>, PhantomData<&'a mut Decoder>);

impl FrameTempRef<'_> {
    #[inline(always)]
    const fn as_ref(&self) -> &aom_image_t {
        unsafe { self.0.as_ref() }
    }

    #[inline]
    unsafe fn single_plane_iter<T: Copy>(&self, plane_n: u8) -> Result<RowsIter<'_, T>> {
        assert!(plane_n < 3);

        let img = self.as_ref();
        if img.bit_depth <= 8 {
            assert_eq!(1, std::mem::size_of::<T>());
        } else {
            assert_eq!(2, std::mem::size_of::<T>());
        }

        let w = unsafe { aom_img_plane_width(img, i32::from(plane_n)) as usize };
        let h = unsafe { aom_img_plane_height(img, i32::from(plane_n)) as usize };

        let stride_bytes = img.stride[plane_n as usize] as isize;
        if stride_bytes.unsigned_abs() * std::mem::size_of::<T>() < w {
            return Err(Error::Unsupported("stride"));
        }

        Ok(RowsIter {
            plane_start: img.planes[plane_n as usize],
            stride_bytes,
            w,
            h,
            y: 0,
            _data: PhantomData,
        })
    }

    /// Access pixel data
    ///
    /// Iterator over rows of image data.
    ///
    /// The data can be grayscale (mono) or YUV (YCbCr), so the result is wrapped in an `enum`
    pub fn rows_iter(&self) -> Result<RowsIters<'_>> {
        let chroma_sampling = self.chroma_sampling()?;
        let depth = self.depth()?;
        let flipped_uv = matches!(
            self.as_ref().fmt,
            AOM_IMG_FMT_YV12 | AOM_IMG_FMT_AOMYV12 | AOM_IMG_FMT_YV1216
        );
        Ok(unsafe {
            match (chroma_sampling, depth) {
                (ChromaSampling::Monochrome, Depth::Depth8) => {
                    RowsIters::Mono8(self.single_plane_iter(0)?)
                }
                (ChromaSampling::Monochrome, depth) => {
                    RowsIters::Mono16(self.single_plane_iter(0)?, depth)
                }
                (chroma_sampling, Depth::Depth8) => {
                    let y = self.single_plane_iter::<u8>(0)?;
                    let u = self.single_plane_iter(1)?;
                    let v = self.single_plane_iter(2)?;
                    let (u, v) = if flipped_uv { (v, u) } else { (u, v) };
                    RowsIters::YuvPlanes8 {
                        y,
                        u,
                        v,
                        chroma_sampling,
                    }
                }
                (chroma_sampling, depth) => {
                    let y = self.single_plane_iter::<[u8; 2]>(0)?;
                    let u = self.single_plane_iter(1)?;
                    let v = self.single_plane_iter(2)?;
                    let (u, v) = if flipped_uv { (v, u) } else { (u, v) };
                    RowsIters::YuvPlanes16 {
                        y,
                        u,
                        v,
                        depth,
                        chroma_sampling,
                    }
                }
            }
        })
    }

    /// Whether image uses chroma subsampling or not
    #[inline]
    pub const fn chroma_sampling(&self) -> Result<ChromaSampling> {
        if self.as_ref().monochrome != 0 {
            return Ok(ChromaSampling::Monochrome);
        }
        Ok(match self.as_ref().fmt {
            AOM_IMG_FMT_YV12 | AOM_IMG_FMT_I420 | AOM_IMG_FMT_AOMYV12 | AOM_IMG_FMT_AOMI420
            | AOM_IMG_FMT_I42016 | AOM_IMG_FMT_YV1216 => ChromaSampling::Cs420,
            AOM_IMG_FMT_I422 | AOM_IMG_FMT_I42216 => ChromaSampling::Cs422,
            AOM_IMG_FMT_I444 | AOM_IMG_FMT_I44416 => ChromaSampling::Cs444,
            _ => return Err(Error::Unsupported("Unknown image format")),
        })
    }

    /// How many bits per pixel that is
    #[inline]
    pub const fn depth(&self) -> Result<Depth> {
        Ok(match self.as_ref().bit_depth {
            8 => Depth::Depth8,
            10 => Depth::Depth10,
            12 => Depth::Depth12,
            16 => Depth::Depth16,
            _ => return Err(Error::Unsupported("Bad depth")),
        })
    }

    /// What flavor of RGB color this should be converted to
    #[inline]
    #[must_use]
    pub const fn color_primaries(&self) -> Option<ColorPrimaries> {
        Some(match self.as_ref().cp {
            AOM_CICP_CP_BT_709 => ColorPrimaries::BT709,
            AOM_CICP_CP_BT_470_M => ColorPrimaries::BT470M,
            AOM_CICP_CP_BT_470_B_G => ColorPrimaries::BT470BG,
            AOM_CICP_CP_BT_601 | AOM_CICP_CP_SMPTE_240 => ColorPrimaries::BT601,
            AOM_CICP_CP_GENERIC_FILM => ColorPrimaries::GenericFilm,
            AOM_CICP_CP_BT_2020 => ColorPrimaries::BT2020,
            AOM_CICP_CP_XYZ => ColorPrimaries::XYZ,
            AOM_CICP_CP_SMPTE_431 => ColorPrimaries::SMPTE431,
            AOM_CICP_CP_SMPTE_432 => ColorPrimaries::SMPTE432,
            AOM_CICP_CP_EBU_3213 => ColorPrimaries::EBU3213,
            _ => return None,
        })
    }

    /// That's basically gamma correction
    #[inline]
    #[must_use]
    pub const fn transfer_characteristics(&self) -> Option<TransferCharacteristics> {
        #[allow(deprecated)]
        Some(match self.as_ref().tc {
            AOM_CICP_TC_BT_709 => TransferCharacteristics::BT709,
            AOM_CICP_TC_BT_470_M => TransferCharacteristics::BT470M,
            AOM_CICP_TC_BT_470_B_G => TransferCharacteristics::BT470BG,
            AOM_CICP_TC_BT_601 => TransferCharacteristics::BT601,
            AOM_CICP_TC_SMPTE_240 => TransferCharacteristics::SMPTE240,
            AOM_CICP_TC_LINEAR => TransferCharacteristics::Linear,
            AOM_CICP_TC_LOG_100 => TransferCharacteristics::Log100,
            AOM_CICP_TC_LOG_100_SQRT10 => TransferCharacteristics::Log100Sqrt10,
            AOM_CICP_TC_IEC_61966 => TransferCharacteristics::IEC61966,
            AOM_CICP_TC_BT_1361 => TransferCharacteristics::BT1361,
            AOM_CICP_TC_SRGB => TransferCharacteristics::SRGB,
            AOM_CICP_TC_BT_2020_10_BIT => TransferCharacteristics::BT2020_10Bit,
            AOM_CICP_TC_BT_2020_12_BIT => TransferCharacteristics::BT2020_12Bit,
            AOM_CICP_TC_SMPTE_2084 => TransferCharacteristics::SMPTE2084,
            AOM_CICP_TC_SMPTE_428 => TransferCharacteristics::SMPTE428,
            AOM_CICP_TC_HLG => TransferCharacteristics::HLG,
            _ => return None,
        })
    }

    /// Flavor of YUV used for the pixels
    ///
    /// See [yuv](https://lib.rs/yuv) crate for conversion to RGB.
    #[inline]
    #[must_use]
    pub const fn matrix_coefficients(&self) -> Option<MatrixCoefficients> {
        Some(match self.as_ref().mc {
            AOM_CICP_MC_IDENTITY => MatrixCoefficients::Identity,
            AOM_CICP_MC_BT_709 => MatrixCoefficients::BT709,
            AOM_CICP_MC_FCC => MatrixCoefficients::FCC,
            AOM_CICP_MC_BT_470_B_G => MatrixCoefficients::BT470BG,
            AOM_CICP_MC_BT_601 => MatrixCoefficients::BT601,
            AOM_CICP_MC_SMPTE_240 => MatrixCoefficients::SMPTE240,
            AOM_CICP_MC_SMPTE_YCGCO => MatrixCoefficients::YCgCo,
            AOM_CICP_MC_BT_2020_NCL | AOM_CICP_MC_BT_2020_CL => MatrixCoefficients::BT2020NCL,
            AOM_CICP_MC_SMPTE_2085 => MatrixCoefficients::SMPTE2085,
            AOM_CICP_MC_CHROMAT_NCL => MatrixCoefficients::ChromatNCL,
            AOM_CICP_MC_CHROMAT_CL => MatrixCoefficients::ChromatCL,
            AOM_CICP_MC_ICTCP => MatrixCoefficients::ICtCp,
            _ => return None,
        })
    }

    /// Whether pixels are in 0-255 or 16-235/240 range.
    #[inline(always)]
    #[must_use]
    pub const fn range(&self) -> Range {
        match self.as_ref().range {
            AOM_CR_STUDIO_RANGE => Range::Limited,
            _ => Range::Full,
        }
    }

    /// Alignment of the chroma channels
    ///
    /// Routines in this library don't support this detail.
    /// Also, chroma subsampling is useless in AV1, so please don't use it.
    #[inline(always)]
    #[must_use]
    pub const fn chroma_sample_position(&self) -> Option<ChromaSamplePosition> {
        match self.as_ref().csp {
            AOM_CSP_VERTICAL => Some(ChromaSamplePosition::Vertical),
            AOM_CSP_COLOCATED => Some(ChromaSamplePosition::Colocated),
            _ => None,
        }
    }
}

impl fmt::Debug for FrameTempRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let img = self.as_ref();
        f.debug_struct("FrameTempRef")
            .field("chroma_sampling", &self.chroma_sampling())
            .field("color_primaries", &self.color_primaries())
            .field("transfer_characteristics", &self.transfer_characteristics())
            .field("matrix_coefficients", &self.matrix_coefficients())
            .field("monochrome", &img.monochrome)
            .field("csp", &self.chroma_sample_position())
            .field("range", &self.range())
            .field("depth", &self.depth())
            .field("width", &img.d_w)
            .field("height", &img.d_h)
            .finish()
    }
}
