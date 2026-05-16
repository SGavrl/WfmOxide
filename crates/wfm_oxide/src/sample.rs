use rayon::prelude::*;

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SampleType {
    U8,
    I8,
    U16Le,
    U16Be,
    I16Le,
    I16Be,
    I32Le,
    I32Be,
    F32Le,
}

impl SampleType {
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            SampleType::U8 | SampleType::I8 => 1,
            SampleType::U16Le | SampleType::U16Be | SampleType::I16Le | SampleType::I16Be => 2,
            SampleType::I32Le | SampleType::I32Be | SampleType::F32Le => 4,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Affine {
    pub scale: f32,
    pub offset: f32,
}

impl Affine {
    #[inline(always)]
    pub fn apply(self, raw: f32) -> f32 {
        self.scale * raw + self.offset
    }
}

#[inline(always)]
fn read_sample(buf: &[u8], byte_off: usize, ty: SampleType) -> f32 {
    match ty {
        SampleType::U8 => buf[byte_off] as f32,
        SampleType::I8 => (buf[byte_off] as i8) as f32,
        SampleType::U16Le => u16::from_le_bytes([buf[byte_off], buf[byte_off + 1]]) as f32,
        SampleType::U16Be => u16::from_be_bytes([buf[byte_off], buf[byte_off + 1]]) as f32,
        SampleType::I16Le => i16::from_le_bytes([buf[byte_off], buf[byte_off + 1]]) as f32,
        SampleType::I16Be => i16::from_be_bytes([buf[byte_off], buf[byte_off + 1]]) as f32,
        SampleType::I32Le => i32::from_le_bytes([
            buf[byte_off], buf[byte_off + 1], buf[byte_off + 2], buf[byte_off + 3],
        ]) as f32,
        SampleType::I32Be => i32::from_be_bytes([
            buf[byte_off], buf[byte_off + 1], buf[byte_off + 2], buf[byte_off + 3],
        ]) as f32,
        SampleType::F32Le => f32::from_le_bytes([
            buf[byte_off], buf[byte_off + 1], buf[byte_off + 2], buf[byte_off + 3],
        ]),
    }
}

pub fn decode_with<F>(
    buf: &[u8],
    n: usize,
    ty: SampleType,
    transform: Affine,
    raw_idx: F,
) -> Vec<f32>
where
    F: Fn(usize) -> usize + Sync,
{
    let bpp = ty.bytes_per_sample();
    match ty {
        SampleType::U8 => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::U8)))
            .collect(),
        SampleType::I8 => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::I8)))
            .collect(),
        SampleType::U16Le => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::U16Le)))
            .collect(),
        SampleType::U16Be => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::U16Be)))
            .collect(),
        SampleType::I16Le => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::I16Le)))
            .collect(),
        SampleType::I16Be => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::I16Be)))
            .collect(),
        SampleType::I32Le => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::I32Le)))
            .collect(),
        SampleType::I32Be => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::I32Be)))
            .collect(),
        SampleType::F32Le => (0..n)
            .into_par_iter()
            .map(|i| transform.apply(read_sample(buf, raw_idx(i) * bpp, SampleType::F32Le)))
            .collect(),
    }
}
